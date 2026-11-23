//! Frames: a big-endian `u32` length, then a postcard-encoded `Frame`. Blob
//! content follows its `Blob` frame as raw bytes.
//!
//! A sync stream, initiator I and responder R:
//! 1. I `Hello`; R `Welcome` or `Refused`.
//! 2. Reconciliation in lockstep: I `Round`, R `Round` (its replies), until I
//!    has nothing to send and says `Finish`. Replies only answer messages, so
//!    R never holds anything back.
//! 3. Both at once: the files the other lacks as `File`, then `FilesDone`.
//! 4. Both at once: `WantBlobs`, then the blobs the other wants as `Blob` and
//!    raw bytes, then `BlobsDone`.
//! 5. R `Bye` once it has read everything; I closes on it, so nothing I sent
//!    is still in flight.
//!
//! In version 3 (the initiator's `Hello` says which) step 4 is gone: each
//! side fetches the blobs it wants afterwards, on blob streams of the same
//! connection, several at once.
//!
//! A blob stream: `GetBlob`, answered by `Blob` and raw bytes; or (version
//! 3) `GetBlobAt`, answered by `Blob` with the whole size and the bytes
//! from the offset on, so a transfer cut off goes on where it stopped.
//!
//! A summary stream (version 3): `Summary` with a fingerprint for each
//! repository the initiator holds, answered by `Summaries` with the
//! responder's (`None`: not held there). Only those that differ are
//! synced: a sync with nothing new is this one round trip. A node of
//! version 2 does not know the frame and drops the stream unanswered: the
//! initiator then syncs every repository as before, in version 2.
//!
//! Connections are kept (one per peer) and either side opens streams on
//! one, whichever side dialed: a node of version 2 serves only those it
//! accepted, so a connection it dialed carries nothing to it.
//!
//! A question stream: `Ask`, answered by `Answer` (or `Refused`). What is
//! asked and answered is the host's own encoding; the protocol carries it.

use iroh::endpoint::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use stitch_store::sync::{Fingerprint, Message};

use crate::{Error, Result};

pub const ALPN: &[u8] = b"stitch/node/1";
pub const VERSION: u32 = 3;
/// The version before summaries; still served, and spoken to a peer that
/// refuses the current one.
pub const LEGACY: u32 = 2;
/// Records and segments are small; large content goes to blobs.
pub const MAX_FRAME: usize = 32 << 20;

#[derive(Debug, Serialize, Deserialize)]
pub enum Frame {
  Hello {
    version: u32,
    repo: String,
  },
  Welcome,
  Refused {
    reason: String,
  },
  Round(Vec<Message>),
  Finish,
  File {
    path: String,
    bytes: Vec<u8>,
  },
  FilesDone,
  WantBlobs(Vec<[u8; 32]>),
  /// `size: None` when the sender does not hold it; else `size` raw bytes follow.
  Blob {
    hash: [u8; 32],
    size: Option<u64>,
  },
  BlobsDone,
  GetBlob {
    hash: [u8; 32],
  },
  Bye,
  Ask(Vec<u8>),
  Answer(Vec<u8>),
  /// New variants go below: their numbers are their place.
  Summary {
    version: u32,
    /// Seal the turns open in these repositories first, as a sync does.
    seal: bool,
    repos: Vec<(String, Fingerprint)>,
  },
  Summaries {
    version: u32,
    repos: Vec<Option<Fingerprint>>,
  },
  GetBlobAt {
    hash: [u8; 32],
    offset: u64,
  },
}

impl Frame {
  pub fn name(&self) -> &'static str {
    match self {
      Frame::Hello { .. } => "Hello",
      Frame::Welcome => "Welcome",
      Frame::Refused { .. } => "Refused",
      Frame::Round(_) => "Round",
      Frame::Finish => "Finish",
      Frame::File { .. } => "File",
      Frame::FilesDone => "FilesDone",
      Frame::WantBlobs(_) => "WantBlobs",
      Frame::Blob { .. } => "Blob",
      Frame::BlobsDone => "BlobsDone",
      Frame::GetBlob { .. } => "GetBlob",
      Frame::Bye => "Bye",
      Frame::Ask(_) => "Ask",
      Frame::Answer(_) => "Answer",
      Frame::Summary { .. } => "Summary",
      Frame::Summaries { .. } => "Summaries",
      Frame::GetBlobAt { .. } => "GetBlobAt",
    }
  }
}

pub async fn write_frame(send: &mut SendStream, frame: &Frame) -> Result<()> {
  write_message(send, frame, frame.name()).await
}

pub async fn read_frame(recv: &mut RecvStream) -> Result<Frame> {
  read_message(recv).await
}

/// How long a peer may send nothing in a session or a transfer before it
/// is given up on: a session that moves takes as long as it needs.
pub const IDLE: std::time::Duration = std::time::Duration::from_secs(30);

/// `read_frame`, given up after `IDLE` without one.
pub async fn read_frame_idle(recv: &mut RecvStream) -> Result<Frame> {
  match tokio::time::timeout(IDLE, read_message(recv)).await {
    Ok(r) => r,
    Err(_) => Err(Error::Protocol(format!(
      "nothing from the peer for {}s",
      IDLE.as_secs()
    ))),
  }
}

/// Writes any message with the frame encoding.
pub async fn write_message<T: Serialize>(
  send: &mut SendStream,
  message: &T,
  name: &str,
) -> Result<()> {
  let bytes = postcard::to_stdvec(message).map_err(|e| Error::Protocol(e.to_string()))?;
  if bytes.len() > MAX_FRAME {
    return Err(Error::Protocol(format!(
      "{name} frame of {} bytes exceeds {MAX_FRAME}",
      bytes.len()
    )));
  }
  send
    .write_all(&(bytes.len() as u32).to_be_bytes())
    .await
    .map_err(Error::net)?;
  send.write_all(&bytes).await.map_err(Error::net)
}

pub async fn read_message<T: serde::de::DeserializeOwned>(recv: &mut RecvStream) -> Result<T> {
  let mut len = [0u8; 4];
  recv.read_exact(&mut len).await.map_err(Error::net)?;
  let len = u32::from_be_bytes(len) as usize;
  if len > MAX_FRAME {
    return Err(Error::Protocol(format!(
      "frame of {len} bytes exceeds {MAX_FRAME}"
    )));
  }
  let mut buf = vec![0u8; len];
  recv.read_exact(&mut buf).await.map_err(Error::net)?;
  postcard::from_bytes(&buf).map_err(|e| Error::Protocol(e.to_string()))
}

pub fn unexpected(frame: &Frame, wanted: &str) -> Error {
  Error::Protocol(format!("expected {wanted}, got {}", frame.name()))
}
