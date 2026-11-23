//! A local client of the node: connects to its socket, starting the node in
//! the background first when none runs (design §2.1).

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;

use crate::api::{self, Reply, Request};
use crate::node::{running, socket_path};
use crate::{Error, Result, VERSION};

fn label_path(data: &Path) -> String {
  let path: String = data
    .to_string_lossy()
    .chars()
    .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
    .collect();
  path.trim_matches('-').to_owned()
}

/// The launchd label of the one-shot job a client (the browser host)
/// starts the node of `data` as: one per data directory.
pub fn launchd_label(data: &Path) -> String {
  format!("dev.stitch.node.{}", label_path(data))
}

/// The label of the LaunchAgent `stitch node service install` writes for
/// the node of `data`: its own, so installing it never touches a job a
/// client started (and the node running in it).
pub fn service_label(data: &Path) -> String {
  format!("dev.stitch.service.{}", label_path(data))
}

fn xml_escape(s: &str) -> String {
  s.replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
    .replace('"', "&quot;")
}

/// The plist of a one-shot launchd job running `exe --data <data> node`,
/// with this process's environment (as a child would have it).
fn launchd_plist(label: &str, data: &Path, exe: &Path, log: &Path) -> String {
  let s = |v: &str| format!("<string>{}</string>", xml_escape(v));
  // Run by launchd, which starts it again when it exits for a restart
  // (status 75) or fails: `stitch node` exits for it then.
  let env: String = std::env::vars()
    .filter(|(k, _)| k != "STITCH_SUPERVISED")
    .chain([("STITCH_SUPERVISED".to_owned(), "1".to_owned())])
    .map(|(k, v)| format!("<key>{}</key>{}", xml_escape(&k), s(&v)))
    .collect();
  format!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
     <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
     <plist version=\"1.0\"><dict>\
     <key>Label</key>{label}\
     <key>ProgramArguments</key><array>{exe}<string>--data</string>{data}<string>node</string></array>\
     <key>EnvironmentVariables</key><dict>{env}</dict>\
     <key>WorkingDirectory</key>{data}\
     <key>StandardOutPath</key>{log}<key>StandardErrorPath</key>{log}\
     <key>RunAtLoad</key><true/>\
     <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\
     <key>ProcessType</key><string>Interactive</string>\
     </dict></plist>\n",
    label = s(label),
    exe = s(&exe.to_string_lossy()),
    data = s(&data.to_string_lossy()),
    log = s(&log.to_string_lossy()),
  )
}

/// The service that runs the node of `data`, as `stitch node service
/// install` wrote it to `<data>/service`: its kind (`systemd`, `launchd`)
/// and its unit's name or agent's label.
pub fn service_of(data: &Path) -> Option<(String, String)> {
  let text = std::fs::read_to_string(data.join("service")).ok()?;
  let mut lines = text.lines().map(str::trim);
  let kind = lines.next()?.to_owned();
  let name = lines.next()?.to_owned();
  (matches!(kind.as_str(), "systemd" | "launchd") && !name.is_empty()).then_some((kind, name))
}

/// How long a client waits for a node's service to bring the node up
/// before it starts one of its own outside it: 30 s;
/// `STITCH_SERVICE_WAIT_SECS` for tests.
pub fn service_wait() -> Duration {
  let secs = std::env::var("STITCH_SERVICE_WAIT_SECS")
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(30);
  Duration::from_secs(secs)
}

/// Starts the service of a node (it may be starting or restarting already).
pub fn start_service(data: &Path, kind: &str, name: &str) -> Result<()> {
  let out = if kind == "systemd" {
    Command::new("systemctl")
      .args(["--user", "start", &format!("{name}.service")])
      .output()
  } else {
    let uid = std::os::unix::fs::MetadataExt::uid(
      &std::fs::metadata(data).map_err(Error::io(data.display()))?,
    );
    Command::new("launchctl")
      .args(["kickstart", &format!("gui/{uid}/{name}")])
      .output()
  }
  .map_err(Error::io(kind))?;
  if !out.status.success() {
    return Err(Error::Config(format!(
      "the node's service ({kind} {name}) did not start: {}",
      String::from_utf8_lossy(&out.stderr).trim()
    )));
  }
  Ok(())
}

/// Starts the node as a launchd job of the user's GUI session: written to
/// `node.plist` in the data directory, the job of an earlier start (ended)
/// booted out first, since a label is loaded once.
fn launchd_start(data: &Path, exe: &Path, log: &Path) -> Result<()> {
  let label = launchd_label(data);
  let plist = data.join("node.plist");
  std::fs::write(&plist, launchd_plist(&label, data, exe, log))
    .map_err(Error::io(plist.display()))?;
  // The user's: the owner of the data directory (no unsafe `getuid`).
  let uid = std::os::unix::fs::MetadataExt::uid(
    &std::fs::metadata(data).map_err(Error::io(data.display()))?,
  );
  let domain = format!("gui/{uid}");
  let _ = Command::new("launchctl")
    .args(["bootout", &format!("{domain}/{label}")])
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status();
  let out = Command::new("launchctl")
    .arg("bootstrap")
    .arg(&domain)
    .arg(&plist)
    .output()
    .map_err(Error::io("launchctl"))?;
  if !out.status.success() {
    return Err(Error::Config(format!(
      "launchctl bootstrap {domain} {}: {}",
      plist.display(),
      String::from_utf8_lossy(&out.stderr).trim()
    )));
  }
  Ok(())
}

/// A node's hello: its version, its name, and (`None` from a node before
/// them) what runs and whether it restarts.
type Hello = (String, String, Option<(Vec<String>, bool)>);

fn hello_of(frame: &[u8]) -> Result<Hello> {
  match postcard::from_bytes::<Reply>(frame) {
    Ok(Reply::Hello {
      version,
      node,
      busy,
      restarting,
    }) => Ok((version, node, Some((busy, restarting)))),
    // A node from before `busy` and `restarting`.
    _ => match postcard::from_bytes::<OldHello>(frame) {
      Ok(OldHello::Hello { version, node }) => Ok((version, node, None)),
      Err(e) => Err(Error::Protocol(format!("expected Hello: {e}"))),
    },
  }
}

/// `Reply::Hello` as nodes before `busy` and `restarting` sent it.
#[derive(serde::Deserialize)]
enum OldHello {
  Hello { version: String, node: String },
}

/// A connection's halves: the node's socket, or one in process (the app).
pub(crate) type Reader = Box<dyn AsyncRead + Unpin + Send>;
pub(crate) type Writer = Box<dyn AsyncWrite + Unpin + Send>;

pub struct Client {
  pub(crate) r: Reader,
  pub(crate) w: Writer,
  /// The node's name.
  pub node: String,
}

/// This process's environment, as a chat sends it for the shells it
/// starts: without what describes this terminal or this shell only.
pub fn client_env() -> Vec<(String, String)> {
  const NOT: &[&str] = &[
    "PWD",
    "OLDPWD",
    "SHLVL",
    "_",
    "TERM",
    "COLUMNS",
    "LINES",
    "PS1",
    "PS2",
    "PROMPT_COMMAND",
    "STITCH_SHELL_NONCE",
    "STITCH_THREAD",
  ];
  std::env::vars()
    .filter(|(k, _)| !NOT.contains(&k.as_str()))
    .collect()
}

impl Client {
  /// Connects to the running node.
  pub async fn connect(data: &Path) -> Result<Client> {
    let stream = UnixStream::connect(socket_path(data)?)
      .await
      .map_err(|_| Error::NotRunning)?;
    let (r, w) = stream.into_split();
    Client::greet(Box::new(r), Box::new(w)).await
  }

  /// Says hello on a connection to a node, and hears its.
  pub(crate) async fn greet(mut r: Reader, mut w: Writer) -> Result<Client> {
    api::write(
      &mut w,
      &Request::Hello {
        version: VERSION.into(),
      },
    )
    .await?;
    let Some(frame) = api::read_frame(&mut r).await? else {
      return Err(Error::Protocol("the node closed the connection".into()));
    };
    let (version, node, news) = hello_of(&frame)?;
    if version == VERSION {
      return Ok(Client { r, w, node });
    }
    let what = match news {
      Some((_, true)) => return Err(Error::Restarting),
      Some((busy, false)) if api::newer(VERSION, &version) => format!(
        "it restarts once idle (now: {}), or stop it now with `stitch node stop`",
        busy.join("; ")
      ),
      Some(_) => {
        "the node is newer: use the newer `stitch`, or stop the node with `stitch node stop`".into()
      }
      None => "it predates restarting on its own: stop it once with `stitch node stop`".into(),
    };
    Err(Error::VersionMismatch {
      node: version,
      client: VERSION.into(),
      what,
    })
  }

  /// Connects, starting `exe node` in the background when no node runs.
  /// Its output goes to `node.log` in the data directory.
  pub async fn connect_or_start(data: &Path, exe: &Path) -> Result<Client> {
    Client::connect_or_start_with(data, exe, false).await
  }

  /// `connect_or_start` for a process another app started (the browser host,
  /// Chrome's child): on macOS the node is started by launchd instead, so it
  /// is not that app's descendant. One is quarantined as the app is (every
  /// file it writes, compiled build scripts included, gets that app's
  /// `com.apple.quarantine`, and Gatekeeper then refuses to run them) and
  /// its prompts (the keychain) are shown as that app's.
  pub async fn connect_or_start_detached(data: &Path, exe: &Path) -> Result<Client> {
    Client::connect_or_start_with(data, exe, cfg!(target_os = "macos")).await
  }

  async fn connect_or_start_with(data: &Path, exe: &Path, detached: bool) -> Result<Client> {
    match Client::connect(data).await {
      Err(Error::NotRunning) => {}
      // An older node, idle, stops for this one.
      Err(Error::Restarting) => {
        for _ in 0..150 {
          if !running(data) {
            break;
          }
          tokio::time::sleep(Duration::from_millis(100)).await;
        }
      }
      other => return other,
    }
    // A node restarting: its successor (started by its process or its
    // service) is waited for, rather than a node of this client's own.
    if crate::node::restart_due(data) {
      let start = std::time::Instant::now();
      while start.elapsed() < crate::node::RESTARTING_FOR && crate::node::restart_due(data) {
        match Client::connect(data).await {
          Ok(c) => return Ok(c),
          Err(Error::Restarting) | Err(Error::NotRunning) => {}
          Err(e) => return Err(e),
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
      }
      if let Ok(c) = Client::connect(data).await {
        return Ok(c);
      }
    }
    // A node run by a service: the service starts it (it may be
    // restarting already), never a node of this client's own while it
    // may still come: the client waits [`service_wait`] for it, even when
    // starting the service failed (it may be restarting on its own; a
    // node of a client's own, outside the service, then held the lock
    // while the service's waited behind it, and took the notice of a
    // new build meant for it: incident 2026-11-02). Only a service that
    // stays down past it (no user bus over SSH, before the GUI login, a
    // unit that fails) gets a node started here, as before, said in
    // `node.log`.
    if !running(data)
      && let Some((kind, name)) = service_of(data)
    {
      let started = start_service(data, &kind, &name);
      let window = service_wait();
      let since = std::time::Instant::now();
      while since.elapsed() < window && !running(data) {
        match Client::connect(data).await {
          Ok(c) => return Ok(c),
          Err(Error::NotRunning) | Err(Error::Restarting) => {}
          Err(e) => return Err(e),
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
      }
      if !running(data) {
        let why = match started {
          Ok(()) => format!("the node's service ({kind} {name}) started"),
          Err(e) => e.to_string(),
        };
        let line = format!(
          "{} client {}: {why}; no node within {} s: starting a node outside it",
          jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S%.3f"),
          std::process::id(),
          window.as_secs()
        );
        eprintln!("{line}");
        if let Ok(mut f) = std::fs::OpenOptions::new()
          .create(true)
          .append(true)
          .open(data.join("node.log"))
        {
          use std::io::Write;
          let _ = writeln!(f, "{line}");
        }
      }
    }
    if !running(data) {
      std::fs::create_dir_all(data).map_err(Error::io(data.display()))?;
      let log_path = data.join("node.log");
      let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(Error::io(log_path.display()))?;
      if detached {
        drop(log);
        launchd_start(data, exe, &log_path)?;
        for _ in 0..100 {
          tokio::time::sleep(Duration::from_millis(100)).await;
          if let Ok(c) = Client::connect(data).await {
            return Ok(c);
          }
        }
        return Client::connect(data).await;
      }
      let err = log.try_clone().map_err(Error::io(log_path.display()))?;
      let mut cmd = Command::new(exe);
      cmd
        .arg("--data")
        .arg(data)
        .arg("node")
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(err);
      #[cfg(unix)]
      std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
      let mut child = cmd.spawn().map_err(Error::io(exe.display()))?;
      for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Ok(c) = Client::connect(data).await {
          return Ok(c);
        }
        if let Ok(Some(status)) = child.try_wait() {
          let log = std::fs::read_to_string(&log_path).unwrap_or_default();
          let tail: Vec<&str> = log.lines().rev().take(5).collect();
          let tail: Vec<&str> = tail.into_iter().rev().collect();
          return Err(Error::Config(format!(
            "the node exited ({status}):\n{}",
            tail.join("\n")
          )));
        }
      }
    }
    // A node holding the lock but not serving yet (starting: it takes the
    // lock first, then checks its build and opens its store) is waited for.
    for _ in 0..300 {
      match Client::connect(data).await {
        Err(Error::NotRunning) if running(data) => {
          tokio::time::sleep(Duration::from_millis(100)).await;
        }
        other => return other,
      }
    }
    Client::connect(data).await
  }

  /// Sends a request and returns its single reply.
  pub async fn request(&mut self, req: &Request) -> Result<Reply> {
    api::write(&mut self.w, req).await?;
    match api::read::<Reply, _>(&mut self.r).await? {
      Some(Reply::Error(e)) => Err(Error::Remote(e)),
      Some(r) => Ok(r),
      None => Err(Error::Protocol("the node closed the connection".into())),
    }
  }

  /// Sends a chat; streamed replies go to `on_reply` until `ChatDone` (or
  /// `Unsent`: interrupted while it waited for another turn).
  /// For `Reply::Approve`, `on_reply` returns where the user's decision will
  /// arrive (`None` declines); it is sent back to the node.
  pub async fn chat(
    &mut self,
    req: &Request,
    on_reply: impl FnMut(&Reply) -> Option<tokio::sync::oneshot::Receiver<crate::tools::Decision>>,
  ) -> Result<Reply> {
    self.chat_with(req, None, on_reply).await
  }

  /// `chat`, and what comes on `outgoing` while it runs (`Interject`,
  /// `Retract`, `Interrupt`) goes to the node on the same connection.
  pub async fn chat_with(
    &mut self,
    req: &Request,
    mut outgoing: Option<tokio::sync::mpsc::UnboundedReceiver<Request>>,
    mut on_reply: impl FnMut(&Reply) -> Option<tokio::sync::oneshot::Receiver<crate::tools::Decision>>,
  ) -> Result<Reply> {
    api::write(&mut self.w, req).await?;
    // Approvals are answered on the side, so replies keep coming meanwhile.
    let (answers, mut answered) = tokio::sync::mpsc::unbounded_channel::<Request>();
    let (stop, mut stopping) = tokio::sync::watch::channel(false);
    let (r, w) = (&mut self.r, &mut self.w);
    let writer = async move {
      loop {
        let req = tokio::select! {
          Some(q) = answered.recv() => q,
          Some(q) = next(&mut outgoing) => q,
          _ = stopping.changed() => return Ok(()),
        };
        api::write(w, &req).await?;
      }
    };
    let reader = async move {
      loop {
        match api::read::<Reply, _>(r).await? {
          Some(done @ (Reply::ChatDone { .. } | Reply::Unsent)) => return Ok(done),
          Some(Reply::Error(e)) => return Err(Error::Remote(e)),
          Some(r @ Reply::Approve { id, .. }) => {
            // No answer here (none given, the prompt dropped) answers
            // nothing: the question waits on the attention list.
            if let Some(rx) = on_reply(&r) {
              let answers = answers.clone();
              tokio::spawn(async move {
                if let Ok(decision) = rx.await {
                  let _ = answers.send(Request::Approval {
                    id,
                    allow: decision.allow,
                    reason: decision.reason,
                    always: decision.always,
                  });
                }
              });
            }
          }
          Some(r) => {
            on_reply(&r);
          }
          None => return Err(Error::Protocol("the node closed the connection".into())),
        }
      }
    };
    tokio::pin!(reader, writer);
    let mut writing = true;
    let result = loop {
      tokio::select! {
        res = &mut reader => break res,
        res = &mut writer, if writing => {
          writing = false;
          if let Err(e) = res {
            break Err(e);
          }
        }
      }
    };
    // A request being written is finished, so the connection stays whole.
    if writing {
      let _ = stop.send(true);
      let _ = writer.await;
    }
    result
  }
}

impl Client {
  /// Attaches to a shell session (`<thread>/<session>`): what it shows
  /// goes to `on_output`, from its screen as it is; what comes on `input`
  /// is typed into it as the user. Ends when `input` closes (detached) or
  /// the session ends (`true`); the node going away is an error.
  pub async fn attach(
    &mut self,
    id: &str,
    mut input: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    mut on_output: impl FnMut(&[u8]),
  ) -> Result<bool> {
    api::write(&mut self.w, &Request::ShellAttach { id: id.to_owned() }).await?;
    let (r, w) = (&mut self.r, &mut self.w);
    let reader = async move {
      loop {
        match api::read::<Reply, _>(r).await? {
          Some(Reply::ShellOutput(bytes)) => on_output(&bytes),
          Some(Reply::ShellEnded) => return Ok(true),
          None => return Err(Error::Protocol("the node went away".into())),
          Some(Reply::Error(e)) => return Err(Error::Remote(e)),
          Some(_) => {}
        }
      }
    };
    let writer = async move {
      while let Some(bytes) = input.recv().await {
        api::write(w, &Request::ShellInput(bytes)).await?;
      }
      Ok::<bool, Error>(false)
    };
    tokio::select! {
      ended = reader => ended,
      detached = writer => detached,
    }
  }
}

/// The next request on `outgoing`; never when there is none.
async fn next(
  outgoing: &mut Option<tokio::sync::mpsc::UnboundedReceiver<Request>>,
) -> Option<Request> {
  match outgoing {
    Some(rx) => rx.recv().await,
    None => std::future::pending().await,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hellos_of_every_version_are_read() {
    #[derive(serde::Serialize)]
    enum Before {
      Hello { version: String, node: String },
    }
    let old = postcard::to_stdvec(&Before::Hello {
      version: "0.1.0/local-14".into(),
      node: "mbp".into(),
    })
    .unwrap();
    assert_eq!(
      hello_of(&old).unwrap(),
      ("0.1.0/local-14".into(), "mbp".into(), None)
    );
    let new = postcard::to_stdvec(&Reply::Hello {
      version: "0.1.0/local-15".into(),
      node: "mbp".into(),
      busy: vec!["thread x working".into()],
      restarting: false,
    })
    .unwrap();
    assert_eq!(
      hello_of(&new).unwrap(),
      (
        "0.1.0/local-15".into(),
        "mbp".into(),
        Some((vec!["thread x working".into()], false))
      )
    );
  }

  #[test]
  fn versions_order_by_crate_then_protocol() {
    assert!(api::newer("0.1.0/local-15", "0.1.0/local-14"));
    assert!(api::newer("0.2.0/local-1", "0.1.9/local-99"));
    assert!(!api::newer("0.1.0/local-14", "0.1.0/local-14"));
    assert!(!api::newer("0.1.0/local-9", "0.1.0/local-10"));
    assert!(!api::newer("dev", "0.1.0/local-10"));
  }

  #[test]
  fn launchd_plist_is_valid() {
    let data = Path::new("/Users/a b/.stitch");
    let label = launchd_label(data);
    assert_eq!(label, "dev.stitch.node.Users-a-b--stitch");
    let plist = launchd_plist(
      &label,
      data,
      Path::new("/bin/st<i>tch"),
      &data.join("node.log"),
    );
    assert!(plist.contains("<string>/bin/st&lt;i&gt;tch</string>"));
    #[cfg(target_os = "macos")]
    {
      let dir = tempfile::tempdir().unwrap();
      let path = dir.path().join("node.plist");
      std::fs::write(&path, &plist).unwrap();
      let ok = Command::new("plutil")
        .arg("-lint")
        .arg(&path)
        .status()
        .unwrap();
      assert!(ok.success(), "{plist}");
    }
  }
}
