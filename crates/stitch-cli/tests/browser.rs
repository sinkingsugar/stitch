//! Browser control end to end (design §2.18): a real Chrome for Testing,
//! headless, with Stitch's extension loaded unpacked and its native
//! messaging host pointing at this build; a throwaway node; the tools
//! called with `stitch browser call`. Skipped (passing) unless a Chrome for
//! Testing is found: `STITCH_TEST_CHROME`, or Playwright's in
//! `~/.cache/ms-playwright` (branded Chrome ignores `--load-extension`).
//!
//! The test's copy of the extension switches itself on: the real one is
//! switched on only by a click in its popup.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const FIXTURE: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>Fixture</title></head>
<body><h1>Order form</h1>
<form onsubmit="event.preventDefault(); document.getElementById('out').textContent = 'sent';">
<label>Name <input id="name" name="name"></label>
<label>Password <input id="pw" type="password" name="pw"></label>
<label><input id="agree" type="checkbox"> I agree</label>
<select id="size"><option>Small</option><option>Large</option></select>
<button id="go" type="button" onclick="document.getElementById('out').textContent = 'saved ' + document.getElementById('name').value">Save draft</button>
<button id="del" type="button">Delete everything</button>
<input id="file" type="file" aria-label="Attachment">
</form><p id="out"></p>
<a id="dl" href="/dl.txt">Get the notes</a>
<iframe id="inner" title="Inner frame" srcdoc="<button onclick='this.textContent=&quot;Pressed inside&quot;'>Press inside</button>"></iframe>
</body></html>"#;

fn chrome() -> Option<PathBuf> {
  if let Some(p) = std::env::var_os("STITCH_TEST_CHROME") {
    return Some(PathBuf::from(p));
  }
  let base = PathBuf::from(std::env::var_os("HOME")?).join(".cache/ms-playwright");
  let mut found: Vec<PathBuf> = std::fs::read_dir(&base)
    .ok()?
    .flatten()
    .filter(|e| e.file_name().to_string_lossy().starts_with("chromium-"))
    .map(|e| e.path().join("chrome-linux64/chrome"))
    .filter(|p| p.is_file())
    .collect();
  found.sort();
  found.pop()
}

/// Serves the fixture on a free port.
fn serve() -> u16 {
  let l = TcpListener::bind("127.0.0.1:0").unwrap();
  let port = l.local_addr().unwrap().port();
  std::thread::spawn(move || {
    for mut c in l.incoming().flatten() {
      let mut buf = [0u8; 4096];
      let n = c.read(&mut buf).unwrap_or(0);
      let head = String::from_utf8_lossy(&buf[..n]);
      if head.starts_with("GET /dl.txt") {
        let body = "the notes";
        let _ = write!(
          c,
          "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-disposition: attachment; filename=notes.txt\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
          body.len()
        );
        continue;
      }
      let _ = write!(
        c,
        "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{FIXTURE}",
        FIXTURE.len()
      );
    }
  });
  port
}

struct Run {
  data: PathBuf,
  chrome: Option<Child>,
}

impl Run {
  fn stitch(&self, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_stitch"))
      .arg("--data")
      .arg(&self.data)
      .args(args)
      .output()
      .unwrap();
    let text = format!(
      "{}{}",
      String::from_utf8_lossy(&out.stdout),
      String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
  }

  fn call(&self, tool: &str, input: &str) -> (bool, String) {
    let images = self.data.join("images");
    std::fs::create_dir_all(&images).unwrap();
    self.stitch(&[
      "browser",
      "call",
      tool,
      input,
      "--images",
      images.to_str().unwrap(),
    ])
  }
}

impl Drop for Run {
  fn drop(&mut self) {
    if let Some(c) = &mut self.chrome {
      let _ = c.kill();
      let _ = c.wait();
    }
    let _ = self.stitch(&["node", "stop"]);
  }
}

fn ref_of(snapshot: &str, needle: &str) -> String {
  let line = snapshot
    .lines()
    .find(|l| l.contains(needle))
    .unwrap_or_else(|| panic!("no {needle} in:\n{snapshot}"));
  let i = line.rfind("[ref_").unwrap();
  line[i + 1..line[i..].find(']').unwrap() + i].to_owned()
}

#[test]
fn the_tools_drive_a_real_chrome_through_the_extension() {
  let Some(exe) = chrome() else {
    eprintln!("no Chrome for Testing: skipped");
    return;
  };
  let tmp = tempfile::tempdir().unwrap();
  let mut run = Run {
    data: tmp.path().join("data"),
    chrome: None,
  };
  let (ok, out) = run.stitch(&["setup", "--name", "btest", "--local"]);
  assert!(ok, "{out}");
  let config = run.data.join("config.toml");
  let mut c = std::fs::read_to_string(&config).unwrap();
  c.push_str("\n[browser]\nsites = [\"127.0.0.1\"]\nresume_after_secs = 1\n");
  std::fs::write(&config, c).unwrap();
  let profile = tmp.path().join("profile");
  let (ok, out) = run.stitch(&[
    "browser",
    "install",
    "--profile-dir",
    profile.to_str().unwrap(),
  ]);
  assert!(ok, "{out}");
  // The test's copy switches itself on.
  let ext = tmp.path().join("ext");
  copy_dir(&run.data.join("browser/extension"), &ext);
  let mut sw = std::fs::read_to_string(ext.join("sw.js")).unwrap();
  sw.push_str("\nchrome.storage.local.set({ enabled: true, name: \"test\" }).then(hello);\n");
  std::fs::write(ext.join("sw.js"), sw).unwrap();
  run.chrome = Some(
    Command::new(exe)
      .arg("--headless=new")
      .arg(format!("--user-data-dir={}", profile.display()))
      .arg(format!("--load-extension={}", ext.display()))
      // Inside a sandbox (a fixer's shell: `no_new_privs`), Chrome's own
      // cannot start; this one only loads the test's pages.
      .args(in_a_sandbox().then_some("--no-sandbox"))
      .args([
        "--no-first-run",
        "--no-default-browser-check",
        "--window-size=1200,900",
        "about:blank",
      ])
      .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn()
      .unwrap(),
  );
  let start = Instant::now();
  loop {
    let (_, out) = run.stitch(&["browser", "profiles"]);
    if out.contains("chromium:test  enabled  connected") {
      break;
    }
    assert!(
      start.elapsed() < Duration::from_secs(30),
      "no browser connected: {out}"
    );
    std::thread::sleep(Duration::from_millis(500));
  }

  let port = serve();
  let url = format!("http://127.0.0.1:{port}/");
  let (ok, snap) = run.call(
    "browser_tabs",
    &format!(r#"{{"action":"new","url":"{url}"}}"#),
  );
  assert!(ok, "{snap}");
  assert!(snap.contains("<page origin=\"http://127.0.0.1:"), "{snap}");
  let name = ref_of(&snap, "textbox \"Name\"");
  let agree = ref_of(&snap, "checkbox");
  let save = ref_of(&snap, "Save draft");
  let pw = ref_of(&snap, "Password");
  let size = ref_of(&snap, "combobox");
  let del = ref_of(&snap, "Delete everything");

  // Typing and clicking by ref, as the user's input would.
  let (ok, out) = run.call(
    "browser_act",
    &format!(
      r#"{{"intent":"input","actions":[{{"type":"type","ref":"{name}","text":"Ada"}},{{"type":"click","ref":"{agree}"}},{{"type":"click","ref":"{save}"}}]}}"#
    ),
  );
  assert!(ok, "{out}");
  assert!(out.contains("value=\"Ada\""), "{out}");
  assert!(out.contains("checked"), "{out}");
  let (ok, text) = run.call("browser_snapshot", r#"{"text":true}"#);
  assert!(ok && text.contains("saved Ada"), "{text}");

  // Fields at once; a password never.
  let (ok, out) = run.call(
    "browser_fill",
    &format!(r#"{{"fields":[{{"ref":"{size}","value":"Large"}}]}}"#),
  );
  assert!(ok && out.contains("Large"), "{out}");
  let (ok, out) = run.call(
    "browser_act",
    &format!(r#"{{"intent":"input","actions":[{{"type":"type","ref":"{pw}","text":"hunter2"}}]}}"#),
  );
  assert!(!ok && out.contains("never types"), "{out}");

  // What looks consequential asks, whatever the declared intent; here
  // nobody can answer, so it is refused, and nothing ran.
  let (ok, out) = run.call(
    "browser_act",
    &format!(r#"{{"intent":"input","actions":[{{"type":"click","ref":"{del}"}}]}}"#),
  );
  assert!(!ok && out.contains("needs the user's approval"), "{out}");
  let (ok, out) = run.call(
    "browser_act",
    &format!(r#"{{"intent":"purchase","actions":[{{"type":"click","ref":"{save}"}}]}}"#),
  );
  assert!(!ok && out.contains("consequential"), "{out}");
  // Another site asks too.
  let (ok, out) = run.call(
    "browser_navigate",
    &format!(r#"{{"url":"http://localhost:{port}/"}}"#),
  );
  assert!(!ok && out.contains("another site"), "{out}");

  // A screenshot of the tab, in the background, in viewport coordinates.
  let (ok, out) = run.call("browser_screenshot", "{}");
  assert!(ok && out.contains("[image: "), "{out}");
  let (ok, out) = run.call("browser_screenshot", &format!(r#"{{"ref":"{save}"}}"#));
  assert!(ok && out.contains("8 px around it"), "{out}");

  // A same-origin frame is read and acted in as part of the page.
  let (ok, snap) = run.call("browser_snapshot", "{}");
  assert!(ok && snap.contains("iframe \"Inner frame\""), "{snap}");
  let inside = ref_of(&snap, "Press inside");
  let (ok, out) = run.call(
    "browser_act",
    &format!(r#"{{"intent":"input","actions":[{{"type":"click","ref":"{inside}"}}]}}"#),
  );
  assert!(ok && out.contains("Pressed inside"), "{out}");

  // Uploads: never a secret; otherwise asked (and nobody answers here).
  let file = ref_of(&snap, "Attachment");
  let work = tmp.path().join("work");
  std::fs::create_dir_all(&work).unwrap();
  std::fs::write(work.join(".env"), "KEY=1").unwrap();
  std::fs::write(work.join("cv.txt"), "me").unwrap();
  let upload = |name: &str| {
    let out = Command::new(env!("CARGO_BIN_EXE_stitch"))
      .current_dir(&work)
      .arg("--data")
      .arg(&run.data)
      .args(["browser", "call", "browser_upload"])
      .arg(format!(r#"{{"ref":"{file}","files":["{name}"]}}"#))
      .output()
      .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
  };
  assert!(upload(".env").contains("never uploaded"));
  assert!(upload("cv.txt").contains("needs the user's approval"));
  let (ok, out) = run.call(
    "browser_plan",
    r#"{"sites":["example.com"],"approach":"read"}"#,
  );
  assert!(!ok && out.contains("needs the user's approval"), "{out}");

  // A download the page starts is the session's.
  let dl = ref_of(&snap, "Get the notes");
  let (ok, out) = run.call(
    "browser_act",
    &format!(r#"{{"intent":"input","actions":[{{"type":"click","ref":"{dl}"}}],"observe":false}}"#),
  );
  assert!(ok, "{out}");
  let start = Instant::now();
  loop {
    let (_, out) = run.call("browser_downloads", "{}");
    if out.contains("dl.txt") {
      break;
    }
    assert!(
      start.elapsed() < Duration::from_secs(15),
      "no download: {out}"
    );
    std::thread::sleep(Duration::from_millis(500));
  }

  // The session's tabs; after a node restart, found again.
  let (ok, _) = run.stitch(&["node", "stop"]);
  assert!(ok);
  let start = Instant::now();
  loop {
    let (_, out) = run.stitch(&["browser", "profiles"]);
    if out.contains("connected") && !out.contains("not connected") {
      break;
    }
    assert!(
      start.elapsed() < Duration::from_secs(30),
      "the browser did not come back: {out}"
    );
    std::thread::sleep(Duration::from_millis(500));
  }
  let (ok, out) = run.call("browser_tabs", r#"{"action":"list"}"#);
  assert!(
    ok && out.contains("(current)") && out.contains(&url),
    "{out}"
  );
  let (ok, out) = run.call("browser_find", r#"{"text":"save"}"#);
  assert!(ok && out.contains("Save draft"), "{out}");
}

/// Whether this process runs in a sandbox (Linux's `NoNewPrivs`, which a
/// sandboxed shell's commands carry).
fn in_a_sandbox() -> bool {
  std::fs::read_to_string("/proc/self/status").is_ok_and(|s| {
    s.lines()
      .any(|l| l.starts_with("NoNewPrivs:") && l.trim_end().ends_with('1'))
  })
}

fn copy_dir(from: &Path, to: &Path) {
  std::fs::create_dir_all(to).unwrap();
  for e in std::fs::read_dir(from).unwrap().flatten() {
    std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
  }
}
