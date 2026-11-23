//! Against an SSH server in the test's own process (russh's server): it
//! runs what it is asked with this machine's `sh`, on a PTY when one was
//! asked for, with a home and a tmux socket directory of the test's own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{self, Auth as ServerAuth, Msg, Server as _, Session as ServerSession};
use russh::{Channel, ChannelId};
use stitch_shell::{Status, StopSignal, Until, Wait};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::client::{Auth, HostKeys, MemoryHostKeys, Target};
use crate::{Error, Pool, ShellOptions};

enum Input {
  Data(Vec<u8>),
  Eof,
  Resize(u16, u16),
}

#[derive(Default)]
struct Chan {
  pty: Option<(u16, u16)>,
  tx: Option<mpsc::UnboundedSender<Input>>,
}

pub struct TestServer {
  pub addr: std::net::SocketAddr,
  pub key: PrivateKey,
  _home: tempfile::TempDir,
  tmux_dir: tempfile::TempDir,
  pub authorized: Arc<Mutex<Vec<PublicKey>>>,
  handles: Arc<Mutex<Vec<server::Handle>>>,
  task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct Factory {
  home: std::path::PathBuf,
  tmux_dir: std::path::PathBuf,
  authorized: Arc<Mutex<Vec<PublicKey>>>,
  handles: Arc<Mutex<Vec<server::Handle>>>,
}

struct Handler {
  f: Factory,
  chans: HashMap<ChannelId, Chan>,
  registered: bool,
}

impl server::Server for Factory {
  type Handler = Handler;
  fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Handler {
    Handler {
      f: self.clone(),
      chans: HashMap::new(),
      registered: false,
    }
  }
}

impl Handler {
  fn spawn(&mut self, channel: ChannelId, command: String, session: &mut ServerSession) {
    let handle = session.handle();
    let (tx, mut rx) = mpsc::unbounded_channel::<Input>();
    let chan = self.chans.entry(channel).or_default();
    chan.tx = Some(tx);
    let env = [
      ("HOME", self.f.home.display().to_string()),
      ("TMUX_TMPDIR", self.f.tmux_dir.display().to_string()),
      ("PATH", std::env::var("PATH").unwrap_or_default()),
    ];
    let dir = self.f.home.clone();
    match chan.pty {
      Some((rows, cols)) => {
        tokio::spawn(async move {
          let (pty, pts) = pty_process::open().unwrap();
          pty.resize(pty_process::Size::new(rows, cols)).unwrap();
          let mut child = pty_process::Command::new("sh")
            .args(["-c", &command])
            .env_clear()
            .envs(env)
            .env("TERM", "xterm-256color")
            .current_dir(&dir)
            .spawn(pts)
            .unwrap();
          let (mut r, mut w) = pty.into_split();
          let h = handle.clone();
          let reader = tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            loop {
              match r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                  if h.data(channel, buf[..n].to_vec()).await.is_err() {
                    break;
                  }
                }
              }
            }
          });
          let writer = tokio::spawn(async move {
            while let Some(i) = rx.recv().await {
              match i {
                Input::Data(d) => {
                  let _ = w.write_all(&d).await;
                }
                Input::Resize(r, c) => {
                  let _ = w.resize(pty_process::Size::new(r, c));
                }
                Input::Eof => {}
              }
            }
          });
          let status = child
            .wait()
            .await
            .map(|s| s.code().unwrap_or(255))
            .unwrap_or(255);
          let _ = reader.await;
          writer.abort();
          let _ = handle.exit_status_request(channel, status as u32).await;
          let _ = handle.eof(channel).await;
          let _ = handle.close(channel).await;
        });
      }
      None => {
        tokio::spawn(async move {
          let mut child = tokio::process::Command::new("sh")
            .args(["-c", &command])
            .env_clear()
            .envs(env)
            .current_dir(&dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
          let mut stdin = child.stdin.take();
          let mut out = child.stdout.take().unwrap();
          let mut err = child.stderr.take().unwrap();
          let h = handle.clone();
          let o = tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            while let Ok(n) = out.read(&mut buf).await {
              if n == 0 || h.data(channel, buf[..n].to_vec()).await.is_err() {
                break;
              }
            }
          });
          let h = handle.clone();
          let e = tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            while let Ok(n) = err.read(&mut buf).await {
              if n == 0
                || h
                  .extended_data(channel, 1, buf[..n].to_vec())
                  .await
                  .is_err()
              {
                break;
              }
            }
          });
          let writer = tokio::spawn(async move {
            while let Some(i) = rx.recv().await {
              match i {
                Input::Data(d) => {
                  if let Some(s) = stdin.as_mut() {
                    let _ = s.write_all(&d).await;
                  }
                }
                Input::Eof => stdin = None,
                Input::Resize(..) => {}
              }
            }
          });
          let status = child
            .wait()
            .await
            .map(|s| s.code().unwrap_or(255))
            .unwrap_or(255);
          let _ = o.await;
          let _ = e.await;
          writer.abort();
          let _ = handle.exit_status_request(channel, status as u32).await;
          let _ = handle.eof(channel).await;
          let _ = handle.close(channel).await;
        });
      }
    }
  }
}

impl server::Handler for Handler {
  type Error = russh::Error;

  async fn auth_password(&mut self, _: &str, password: &str) -> Result<ServerAuth, Self::Error> {
    Ok(if password == "pw" {
      ServerAuth::Accept
    } else {
      ServerAuth::reject()
    })
  }

  async fn auth_publickey(
    &mut self,
    _: &str,
    key: server::VerifiedPublicKey<'_>,
  ) -> Result<ServerAuth, Self::Error> {
    let key = key.public_key;
    let ok = self
      .f
      .authorized
      .lock()
      .unwrap()
      .iter()
      .any(|k| k.key_data() == key.key_data());
    Ok(if ok {
      ServerAuth::Accept
    } else {
      ServerAuth::reject()
    })
  }

  async fn channel_open_session(
    &mut self,
    channel: Channel<Msg>,
    reply: server::ChannelOpenHandle,
    session: &mut ServerSession,
  ) -> Result<(), Self::Error> {
    if !self.registered {
      self.registered = true;
      self.f.handles.lock().unwrap().push(session.handle());
    }
    self.chans.insert(channel.id(), Chan::default());
    reply.accept().await;
    Ok(())
  }

  async fn pty_request(
    &mut self,
    channel: ChannelId,
    _: &str,
    cols: u32,
    rows: u32,
    _: u32,
    _: u32,
    _: &[(russh::Pty, u32)],
    session: &mut ServerSession,
  ) -> Result<(), Self::Error> {
    self.chans.entry(channel).or_default().pty = Some((rows as u16, cols as u16));
    session.channel_success(channel)?;
    Ok(())
  }

  async fn exec_request(
    &mut self,
    channel: ChannelId,
    data: &[u8],
    session: &mut ServerSession,
  ) -> Result<(), Self::Error> {
    self.spawn(channel, String::from_utf8_lossy(data).into_owned(), session);
    session.channel_success(channel)?;
    Ok(())
  }

  async fn data(
    &mut self,
    channel: ChannelId,
    data: &[u8],
    _: &mut ServerSession,
  ) -> Result<(), Self::Error> {
    if let Some(tx) = self.chans.get(&channel).and_then(|c| c.tx.as_ref()) {
      let _ = tx.send(Input::Data(data.to_vec()));
    }
    Ok(())
  }

  async fn channel_eof(
    &mut self,
    channel: ChannelId,
    _: &mut ServerSession,
  ) -> Result<(), Self::Error> {
    if let Some(tx) = self.chans.get(&channel).and_then(|c| c.tx.as_ref()) {
      let _ = tx.send(Input::Eof);
    }
    Ok(())
  }

  async fn window_change_request(
    &mut self,
    channel: ChannelId,
    cols: u32,
    rows: u32,
    _: u32,
    _: u32,
    _: &mut ServerSession,
  ) -> Result<(), Self::Error> {
    if let Some(tx) = self.chans.get(&channel).and_then(|c| c.tx.as_ref()) {
      let _ = tx.send(Input::Resize(rows as u16, cols as u16));
    }
    Ok(())
  }
}

impl TestServer {
  pub async fn start() -> TestServer {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    // A short path: tmux's socket path must fit a sockaddr_un.
    let tmux_dir = tempfile::Builder::new()
      .prefix("tx")
      .tempdir_in("/tmp")
      .unwrap();
    let home = tempfile::tempdir().unwrap();
    let f = Factory {
      home: home.path().to_owned(),
      tmux_dir: tmux_dir.path().to_owned(),
      authorized: Arc::new(Mutex::new(Vec::new())),
      handles: Arc::new(Mutex::new(Vec::new())),
    };
    let config = Arc::new(server::Config {
      keys: vec![key.clone()],
      auth_rejection_time: Duration::from_millis(10),
      auth_rejection_time_initial: Some(Duration::ZERO),
      inactivity_timeout: None,
      ..Default::default()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut factory = f.clone();
    let task = tokio::spawn(async move {
      let _ = factory.run_on_socket(config, &listener).await;
    });
    TestServer {
      addr,
      key,
      _home: home,
      tmux_dir,
      authorized: f.authorized.clone(),
      handles: f.handles.clone(),
      task,
    }
  }

  /// The home of the user it runs commands as.
  pub fn home(&self) -> &std::path::Path {
    self._home.path()
  }

  pub fn target(&self, auth: Vec<Auth>) -> Target {
    Target {
      name: "testhost".into(),
      address: "127.0.0.1".into(),
      port: self.addr.port(),
      user: "me".into(),
      jump: None,
      auth,
    }
  }

  /// Every connection dropped, as a network change drops them.
  pub async fn drop_connections(&self) {
    let handles: Vec<server::Handle> = self.handles.lock().unwrap().drain(..).collect();
    for h in handles {
      let _ = h
        .disconnect(
          russh::Disconnect::ByApplication,
          "dropped".into(),
          "en".into(),
        )
        .await;
    }
  }

  pub fn tmux(&self, args: &[&str]) -> std::process::Output {
    std::process::Command::new("tmux")
      .args(args)
      .env("TMUX_TMPDIR", self.tmux_dir.path())
      .env_remove("TMUX")
      .output()
      .unwrap()
  }
}

impl Drop for TestServer {
  fn drop(&mut self) {
    // Without tmux installed there is no server to stop: the tests of a
    // plain shell run all the same.
    let _ = std::process::Command::new("tmux")
      .arg("kill-server")
      .env("TMUX_TMPDIR", self.tmux_dir.path())
      .env_remove("TMUX")
      .output();
    self.task.abort();
  }
}

fn opts(name: &str, tmux: bool) -> ShellOptions {
  ShellOptions {
    name: crate::session_name("t1", name),
    rows: 24,
    cols: 100,
    max_output: 30_000,
    redact: None,
    tmux,
  }
}

/// Whether `ps` works here (not in macOS's sandbox): without it (and
/// without `/proc`), what holds a remote terminal is not known there.
fn ps_works() -> bool {
  std::path::Path::new("/proc/self/stat").exists()
    || std::process::Command::new("ps")
      .args(["-o", "pid=", "-p", "1"])
      .output()
      .is_ok_and(|o| o.status.success())
}

fn has_tmux() -> bool {
  std::process::Command::new("tmux")
    .arg("-V")
    .output()
    .is_ok_and(|o| o.status.success())
}

const W: Duration = Duration::from_secs(20);

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_shell_runs_commands_as_a_local_one_does() {
  let server = TestServer::start().await;
  let keys: Arc<dyn HostKeys> = Arc::new(MemoryHostKeys::default());
  let pool = Pool::default();
  let (conn, new) = pool
    .get(
      &server.target(vec![Auth::Password("pw".into())]),
      keys.clone(),
      None,
    )
    .await
    .unwrap();
  assert!(new);
  let o = crate::open(conn.clone(), opts("plain", false))
    .await
    .unwrap();
  assert!(!o.link.in_tmux());
  let s = o.session;
  assert!(s.is_remote());
  let out = s
    .run("echo hello; (exit 3)", Wait::prompt(W))
    .await
    .unwrap();
  assert_eq!(out.output, "hello");
  assert_eq!(out.status, Status::AtPrompt { exit: 3 });
  // The directory persists, as a session's does.
  s.run("cd /", Wait::prompt(W)).await.unwrap();
  assert_eq!(s.run("pwd", Wait::prompt(W)).await.unwrap().output, "/");
  // A long command is sourced from a file put there.
  let long = format!("echo {}", "x".repeat(2000));
  let out = s.run(&long, Wait::prompt(W)).await.unwrap();
  assert_eq!(out.output, "x".repeat(2000));
  // A program that stops to ask: idle, answered with send.
  let out = s
    .run(
      "read -p 'name? ' n; echo \"hi $n\"",
      Wait {
        timeout: W,
        idle: Some(Duration::from_millis(500)),
        screen: None,
      },
    )
    .await
    .unwrap();
  assert!(matches!(out.status, Status::Running { .. }), "{out:?}");
  let out = s.send(b"bob\r", Wait::prompt(W)).await.unwrap();
  assert!(out.output.contains("hi bob"), "{out:?}");
  // A full-screen program: its screen, waited on.
  s.resize(30, 90).unwrap();
  let out = s
    .run(
      "printf '\\033[?1049h\\033[HFULL SCREEN\\n'; read x; printf '\\033[?1049l'",
      Wait {
        timeout: W,
        idle: None,
        screen: Some(regex_lite("FULL SCREEN")),
      },
    )
    .await
    .unwrap();
  assert!(out.screen.unwrap_or_default().contains("FULL SCREEN"));
  s.send(b"\r", Wait::prompt(W)).await.unwrap();
  // tput reads the terminal's size as resized.
  let cols = s.run("stty size", Wait::prompt(W)).await.unwrap();
  assert_eq!(cols.output, "30 90");
  // A command stopped by a signal there.
  let out = s
    .run("sleep 60", Wait::prompt(Duration::from_millis(300)))
    .await
    .unwrap();
  assert!(matches!(out.status, Status::Running { .. }));
  if ps_works() {
    let g = s.signal_remote(StopSignal::Term).await;
    assert!(g.is_some_and(|g| g > 1), "{g:?}");
    let w = s.wait_for(&Until::Done, Some(W)).await.unwrap();
    assert!(w.done);
    assert!(matches!(w.outcome.status, Status::AtPrompt { exit: 143 }));
  } else {
    eprintln!("ps does not work here (a sandbox): signals not checked");
    s.send(b"\x03", Wait::prompt(W)).await.unwrap();
  }
  s.close().await;
  assert_eq!(s.status(), Status::Closed);
  assert!(!o.link.dropped());
}

fn regex_lite(re: &str) -> stitch_shell::Regex {
  stitch_shell::Regex::new(re).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn host_keys_are_trusted_on_first_use_and_a_changed_one_is_refused() {
  let server = TestServer::start().await;
  let keys = Arc::new(MemoryHostKeys::default());
  let t = server.target(vec![Auth::Password("pw".into())]);
  crate::Connection::connect(&t, keys.clone(), None)
    .await
    .unwrap();
  let known = keys.known(&t.key_host());
  assert_eq!(known, [server.key.public_key().clone()]);
  assert_eq!(t.key_host(), format!("[127.0.0.1]:{}", server.addr.port()));
  // Known: connects again, nothing added.
  crate::Connection::connect(&t, keys.clone(), None)
    .await
    .unwrap();
  assert_eq!(keys.known(&t.key_host()).len(), 1);
  // Another key on record for that address: refused, both named.
  let other = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
  let changed = Arc::new(MemoryHostKeys::default());
  changed.trust(&t.key_host(), "testhost", other.public_key());
  let e = crate::Connection::connect(&t, changed, None)
    .await
    .err()
    .unwrap();
  match &e {
    Error::HostKeyChanged { was, now, .. } => {
      assert_eq!(was, &[crate::client::fingerprint(other.public_key())]);
      assert_eq!(now, &crate::client::fingerprint(server.key.public_key()));
    }
    e => panic!("{e}"),
  }
  assert!(e.to_string().contains("has changed"), "{e}");
  // As the user's known_hosts says it, too.
  let kh = tempfile::NamedTempFile::new().unwrap();
  std::fs::write(
    kh.path(),
    format!(
      "[127.0.0.1]:{} {}\n",
      server.addr.port(),
      other.public_key().to_openssh().unwrap()
    ),
  )
  .unwrap();
  let e = crate::Connection::connect(
    &t,
    Arc::new(MemoryHostKeys::default()),
    Some(kh.path().to_owned()),
  )
  .await
  .err()
  .unwrap();
  assert!(matches!(e, Error::HostKeyChanged { .. }), "{e}");
}

/// A P-256 key signing as the Secure Enclave would (through `DeviceSigner`).
struct SoftSigner(PrivateKey);

impl crate::DeviceSigner for SoftSigner {
  fn public_key(&self) -> PublicKey {
    self.0.public_key().clone()
  }

  fn sign(&self, data: &[u8]) -> Result<Vec<u8>, String> {
    use russh::keys::signature::Signer;
    use russh::keys::ssh_encoding::Encode;
    let sig: russh::keys::ssh_key::Signature = self.0.try_sign(data).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    sig.encode(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn keys_and_passwords_authenticate_and_a_refusal_says_what_was_tried() {
  let server = TestServer::start().await;
  let keys: Arc<dyn HostKeys> = Arc::new(MemoryHostKeys::default());
  let e = crate::Connection::connect(
    &server.target(vec![Auth::Password("wrong".into()), Auth::Agent]),
    keys.clone(),
    None,
  )
  .await
  .err()
  .unwrap();
  assert!(matches!(e, Error::Auth { .. }), "{e}");
  assert!(e.to_string().contains("a password"), "{e}");
  // This device's key, once the host has it.
  let device = crate::device::software_key("stitch@test").unwrap();
  let auth = vec![Auth::Key(Arc::new(device.clone()))];
  assert!(
    crate::Connection::connect(&server.target(auth.clone()), keys.clone(), None)
      .await
      .is_err()
  );
  server
    .authorized
    .lock()
    .unwrap()
    .push(device.public_key().clone());
  let c = crate::Connection::connect(&server.target(auth), keys.clone(), None)
    .await
    .unwrap();
  let out = c.exec("echo $HOME", None, W).await.unwrap();
  assert!(out.ok());
  // An ECDSA key that signs elsewhere.
  let p256 = PrivateKey::random(
    &mut rand::rng(),
    Algorithm::Ecdsa {
      curve: russh::keys::EcdsaCurve::NistP256,
    },
  )
  .unwrap();
  server
    .authorized
    .lock()
    .unwrap()
    .push(p256.public_key().clone());
  let signer = Arc::new(SoftSigner(p256));
  crate::Connection::connect(&server.target(vec![Auth::Signer(signer)]), keys, None)
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tmux_session_outlives_a_dropped_connection_and_is_picked_up() {
  if !has_tmux() {
    eprintln!("no tmux here: skipped");
    return;
  }
  let server = TestServer::start().await;
  let keys: Arc<dyn HostKeys> = Arc::new(MemoryHostKeys::default());
  let pool = Pool::default();
  let target = server.target(vec![Auth::Password("pw".into())]);
  let (conn, _) = pool.get(&target, keys.clone(), None).await.unwrap();
  let o = crate::open(conn, opts("tm", true)).await.unwrap();
  assert!(o.link.in_tmux() && !o.link.resumed());
  let s = o.session;
  let out = s.run("echo one; (exit 2)", Wait::prompt(W)).await.unwrap();
  assert_eq!(out.output, "one");
  assert_eq!(out.status, Status::AtPrompt { exit: 2 });
  let name = crate::session_name("t1", "tm");
  assert!(server.tmux(&["has-session", "-t", &name]).status.success());
  // A long command, and a full-screen program's screen, through tmux.
  let long = format!("echo {}", "y".repeat(1500));
  assert_eq!(
    s.run(&long, Wait::prompt(W)).await.unwrap().output,
    "y".repeat(1500)
  );
  // Running when the connection drops.
  let out = s
    .run(
      "sleep 2; echo after-the-drop",
      Wait::prompt(Duration::from_millis(300)),
    )
    .await
    .unwrap();
  assert!(matches!(out.status, Status::Running { .. }));
  server.drop_connections().await;
  s.ended().await;
  assert_eq!(s.status(), Status::Closed);
  assert!(o.link.dropped(), "a drop, not an exit");
  // Picked up: the same tmux session, busy with what ran.
  let (conn, new) = pool.get(&target, keys.clone(), None).await.unwrap();
  assert!(new, "a new connection");
  let o = crate::open(conn, opts("tm", true)).await.unwrap();
  assert!(o.link.resumed());
  let s = o.session;
  if ps_works() {
    assert!(
      matches!(s.status(), Status::Running { ref program, .. } if program.contains("sleep")),
      "{:?}",
      s.status()
    );
    let w = s.wait_for(&Until::Done, Some(W)).await.unwrap();
    assert!(w.done, "{w:?}");
  } else {
    eprintln!("ps does not work here (a sandbox): taken to be at its prompt");
    s.settle(Duration::from_secs(3), W).await;
  }
  assert!(
    s.screen().text.contains("after-the-drop"),
    "{}",
    s.screen().text
  );
  // At its prompt, it runs the next command as before.
  let out = s.run("echo two", Wait::prompt(W)).await.unwrap();
  assert_eq!(out.output, "two");
  // Closed: its tmux session goes too.
  s.close().await;
  assert!(!o.link.dropped());
  assert!(!server.tmux(&["has-session", "-t", &name]).status.success());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_shell_after_a_drop_is_a_new_one() {
  let server = TestServer::start().await;
  let keys: Arc<dyn HostKeys> = Arc::new(MemoryHostKeys::default());
  let pool = Pool::default();
  let target = server.target(vec![Auth::Password("pw".into())]);
  let (conn, _) = pool.get(&target, keys.clone(), None).await.unwrap();
  let o = crate::open(conn, opts("pl", false)).await.unwrap();
  o.session.run("X=kept", Wait::prompt(W)).await.unwrap();
  server.drop_connections().await;
  o.session.ended().await;
  assert!(o.link.dropped());
  let (conn, _) = pool.get(&target, keys, None).await.unwrap();
  let o = crate::open(conn, opts("pl", false)).await.unwrap();
  assert!(!o.link.resumed());
  let out = o
    .session
    .run("echo \"[$X]\"", Wait::prompt(W))
    .await
    .unwrap();
  assert_eq!(out.output, "[]");
  o.session.close().await;
}

/// A real `sshd` on this machine (the user's keys through the agent):
/// `cargo test -p stitch-ssh -- --ignored localhost`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs sshd on localhost and the user's key authorized there"]
async fn localhost_sshd() {
  let user = std::env::var("USER").unwrap();
  let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
  let mut auth = vec![Auth::Agent];
  for f in ["id_ed25519", "id_ecdsa", "id_rsa"] {
    auth.push(Auth::KeyFile(home.join(".ssh").join(f)));
  }
  // Another host than this one: `STITCH_SSH_TEST_HOST` (as `user@host`).
  let (user, address) = match std::env::var("STITCH_SSH_TEST_HOST") {
    Ok(h) => match h.split_once('@') {
      Some((u, a)) => (u.to_owned(), a.to_owned()),
      None => (user, h),
    },
    Err(_) => (user, "127.0.0.1".to_owned()),
  };
  let target = Target {
    name: address.clone(),
    address,
    port: 22,
    user,
    jump: None,
    auth,
  };
  let keys: Arc<dyn HostKeys> = Arc::new(MemoryHostKeys::default());
  let conn = crate::Connection::connect(&target, keys, Some(home.join(".ssh/known_hosts")))
    .await
    .unwrap();
  // Read-only: a plain shell (no tmux session left there), `uname -a`.
  let o = crate::open(conn, opts("probe", false)).await.unwrap();
  let out = o.session.run("uname -a", Wait::prompt(W)).await.unwrap();
  eprintln!("{}: {}", target.name, out.output);
  assert!(!out.output.is_empty());
  o.session.close().await;
}
