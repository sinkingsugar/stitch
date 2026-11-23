//! `stitch browser-host`: the native messaging host Chrome starts for the
//! extension, once per profile (design §2.18). It relays the extension's
//! messages (Chrome's framing: a native-endian `u32` length, then JSON) to a
//! browser link on the node's socket, starting the node when none runs.
//! It holds no state: when the node goes away it connects again and says
//! the extension's hello again; when it meets a node of another version, the
//! node refuses the link, or `stitch` was installed again, it exits, so the
//! extension starts it again from the binary installed now.

use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::api::{self, Reply, Request};
use crate::client::Client;
use crate::{Error, Result};

/// Chrome's limit for a message to the extension.
const MAX_TO_CHROME: usize = 1 << 20;
/// Chrome sends at most 4 GB; Stitch takes far less.
const MAX_FROM_CHROME: usize = 64 << 20;

/// Reads one native message; `None` when Chrome closed the pipe.
pub async fn read_native<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> Result<Option<String>> {
  let mut len = [0u8; 4];
  match r.read_exact(&mut len).await {
    Ok(_) => {}
    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
    Err(e) => return Err(Error::io("stdin")(e)),
  }
  let len = u32::from_ne_bytes(len) as usize;
  if len > MAX_FROM_CHROME {
    return Err(Error::Protocol(format!("a native message of {len} bytes")));
  }
  let mut buf = vec![0u8; len];
  r.read_exact(&mut buf).await.map_err(Error::io("stdin"))?;
  String::from_utf8(buf)
    .map(Some)
    .map_err(|e| Error::Protocol(e.to_string()))
}

/// Writes one native message.
pub async fn write_native<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, json: &str) -> Result<()> {
  if json.len() > MAX_TO_CHROME {
    return Err(Error::Protocol(format!(
      "a message of {} bytes for Chrome (at most 1 MB)",
      json.len()
    )));
  }
  w.write_all(&(json.len() as u32).to_ne_bytes())
    .await
    .map_err(Error::io("stdout"))?;
  w.write_all(json.as_bytes())
    .await
    .map_err(Error::io("stdout"))?;
  w.flush().await.map_err(Error::io("stdout"))
}

/// Runs the host until Chrome closes its pipe.
pub async fn run(data: &Path, exe: &Path) -> Result<()> {
  // Chrome's messages, read on their own so a node restart loses none of
  // the hello.
  let (from_chrome, mut chrome_rx) = mpsc::unbounded_channel::<String>();
  tokio::spawn(async move {
    let mut stdin = tokio::io::stdin();
    while let Ok(Some(msg)) = read_native(&mut stdin).await {
      if from_chrome.send(msg).is_err() {
        break;
      }
    }
  });
  let (to_chrome, mut out_rx) = mpsc::unbounded_channel::<String>();
  let writer = tokio::spawn(async move {
    let mut stdout = tokio::io::stdout();
    while let Some(msg) = out_rx.recv().await {
      if let Err(e) = write_native(&mut stdout, &msg).await {
        eprintln!("stitch browser-host: {e}");
        if matches!(e, Error::Protocol(_)) {
          continue;
        }
        break;
      }
    }
  });
  let started_as = installed(exe);
  let mut hello: Option<String> = None;
  loop {
    // The hello first: the extension sends it as soon as it connects.
    while hello.is_none() {
      match chrome_rx.recv().await {
        None => return Ok(()),
        Some(m) if is_hello(&m) => hello = Some(m),
        Some(_) => {}
      }
    }
    // Started by launchd, not as Chrome's child (quarantine, prompts).
    let client = match Client::connect_or_start_detached(data, exe).await {
      Ok(c) => c,
      Err(e @ (Error::VersionMismatch { .. } | Error::Restarting)) => {
        // Another binary is current: Chrome starts the host again.
        eprintln!("stitch browser-host: {e}");
        return Ok(());
      }
      Err(e) => {
        eprintln!("stitch browser-host: {e}");
        tokio::time::sleep(Duration::from_secs(2)).await;
        continue;
      }
    };
    let Client { mut r, mut w, .. } = client;
    let h = hello.clone().unwrap_or_default();
    if api::write(&mut w, &Request::BrowserLink { hello: h })
      .await
      .is_err()
    {
      tokio::time::sleep(Duration::from_secs(1)).await;
      continue;
    }
    // Relay until either side ends.
    // Whether to stop: Chrome closed the pipe, or the node refused the link.
    let done = loop {
      tokio::select! {
        msg = chrome_rx.recv() => match msg {
          None => break true,
          Some(m) => {
            if is_hello(&m) {
              hello = Some(m.clone());
            }
            if api::write(&mut w, &Request::BrowserMsg(m)).await.is_err() {
              break false;
            }
          }
        },
        reply = api::read::<Reply, _>(&mut r) => match reply {
          Ok(Some(Reply::BrowserMsg(m))) => {
            if to_chrome.send(m).is_err() {
              break true;
            }
          }
          // The node refused the link: most likely this host is an older
          // binary than the node's, so trying again would fail the same
          // way, forever. Exit: the extension starts the host again from
          // the binary installed now.
          Ok(Some(Reply::Error(e))) => {
            eprintln!("stitch browser-host: the node says: {e}");
            break true;
          }
          Ok(Some(_)) => {}
          Ok(None) | Err(_) => break false,
        },
      }
    };
    if done {
      break;
    }
    // `stitch` was installed again since this host started: its node may
    // speak another protocol under the same version, so exit and let the
    // extension start the new binary rather than retry this one.
    if installed(exe) != started_as {
      eprintln!("stitch browser-host: a new stitch is installed, exiting");
      break;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
  }
  drop(to_chrome);
  let _ = writer.await;
  Ok(())
}

/// When the binary at `exe` was last written; `None` when it is gone (on
/// Linux a replaced binary's own path reads `… (deleted)`).
fn installed(exe: &Path) -> Option<std::time::SystemTime> {
  std::fs::metadata(exe).and_then(|m| m.modified()).ok()
}

fn is_hello(m: &str) -> bool {
  serde_json::from_str::<serde_json::Value>(m)
    .ok()
    .and_then(|v| v.get("t").and_then(|t| t.as_str()).map(|t| t == "hello"))
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn native_frames_round_trip() {
    let mut buf = Vec::new();
    write_native(&mut buf, r#"{"t":"hello"}"#).await.unwrap();
    assert_eq!(&buf[..4], &13u32.to_ne_bytes());
    let mut r = &buf[..];
    assert_eq!(
      read_native(&mut r).await.unwrap().as_deref(),
      Some(r#"{"t":"hello"}"#)
    );
    assert_eq!(read_native(&mut r).await.unwrap(), None);
    assert!(is_hello(r#"{"t":"hello","proto":1}"#));
    assert!(!is_hello(r#"{"t":"res"}"#));
  }

  #[tokio::test]
  async fn too_big_for_chrome_is_refused() {
    let mut buf = Vec::new();
    let big = "x".repeat(MAX_TO_CHROME + 1);
    assert!(write_native(&mut buf, &big).await.is_err());
    assert!(buf.is_empty());
  }
}
