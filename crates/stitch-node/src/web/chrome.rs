//! The node's headless Chromium: found (or installed with `stitch browser
//! install`), started on the first web call, shared by the calls after it,
//! and stopped once unused for a while. Each call gets its own browser
//! context, so calls share no cookies, storage or cache.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chromiumoxide::{Browser, BrowserConfig};
use futures_util::StreamExt;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::config::BrowserSettings;

/// Where `stitch browser install` puts Chromium, in the data directory.
const INSTALL_DIR: &str = "browser";

struct Live {
  browser: Arc<Browser>,
  handler: JoinHandle<()>,
  used: Instant,
  /// The profile directory, removed with the browser.
  _profile: tempfile::TempDir,
}

pub(crate) struct Chrome {
  data: PathBuf,
  settings: BrowserSettings,
  live: Arc<Mutex<Option<Live>>>,
  /// The user agent to launch with, by executable: `None` when its own
  /// is already a windowed browser's.
  agent: Mutex<Option<(PathBuf, Option<String>)>>,
}

/// The page area pages are read and shot in: a laptop's.
const VIEW: (u32, u32) = (1280, 900);
/// The window around it: as tall as the view plus the toolbars a real
/// window has, so the page's window is not smaller than its page.
const WINDOW: (u32, u32) = (1280, 987);

/// Chrome's switches: chromiumoxide's defaults (Puppeteer's) without
/// `--enable-automation`, which sets `navigator.webdriver` and tells every
/// bot check it is driven, and with what makes the headless browser look
/// like one with a window: no automation flag in Blink, a desktop's
/// screen around the window (headless Chrome's is 800×600 otherwise,
/// smaller than its own window).
const ARGS: &[&str] = &[
  "disable-background-networking",
  "enable-features=NetworkService,NetworkServiceInProcess",
  "disable-background-timer-throttling",
  "disable-backgrounding-occluded-windows",
  "disable-breakpad",
  "disable-client-side-phishing-detection",
  "disable-component-extensions-with-background-pages",
  "disable-default-apps",
  "disable-dev-shm-usage",
  "disable-features=TranslateUI",
  "disable-hang-monitor",
  "disable-ipc-flooding-protection",
  "disable-popup-blocking",
  "disable-prompt-on-repost",
  "disable-renderer-backgrounding",
  "disable-sync",
  "force-color-profile=srgb",
  "metrics-recording-only",
  "no-first-run",
  "password-store=basic",
  "use-mock-keychain",
  "lang=en-US",
  "disable-blink-features=AutomationControlled",
  "screen-info={1920x1080}",
];

/// Starts Chrome headless in a fresh profile, under `agent` when given.
async fn launch(
  exe: &Path,
  agent: Option<&str>,
) -> Result<(Browser, JoinHandle<()>, tempfile::TempDir), String> {
  let profile = tempfile::Builder::new()
    .prefix("stitch-chrome-")
    .tempdir()
    .map_err(|e| format!("could not make a browser profile: {e}"))?;
  let mut args: Vec<String> = ARGS.iter().map(|a| (*a).to_owned()).collect();
  if let Some(agent) = agent {
    args.push(format!("user-agent={agent}"));
  }
  let config = BrowserConfig::builder()
    .chrome_executable(exe)
    .user_data_dir(profile.path())
    .disable_default_args()
    .args(args)
    .window_size(WINDOW.0, WINDOW.1)
    .viewport(chromiumoxide::handler::viewport::Viewport {
      width: VIEW.0,
      height: VIEW.1,
      device_scale_factor: Some(1.0),
      ..Default::default()
    })
    .launch_timeout(Duration::from_secs(30))
    .request_timeout(Duration::from_secs(60))
    .build()?;
  let (browser, mut handler) = Browser::launch(config)
    .await
    .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
  let handler = tokio::spawn(async move {
    // Errors here are about single messages; the stream ends with Chrome.
    while handler.next().await.is_some() {}
  });
  Ok((browser, handler, profile))
}

/// The agent a windowed browser of the same build sends, when `agent` is
/// headless Chrome's.
pub(super) fn windowed_agent(agent: &str) -> Option<String> {
  agent
    .contains("HeadlessChrome/")
    .then(|| agent.replace("HeadlessChrome/", "Chrome/"))
}

impl Chrome {
  pub fn new(data: &Path, settings: BrowserSettings) -> Chrome {
    Chrome {
      data: data.to_owned(),
      settings,
      live: Arc::new(Mutex::new(None)),
      agent: Mutex::new(None),
    }
  }

  fn idle(&self) -> Duration {
    Duration::from_secs(60 * self.settings.idle_minutes.unwrap_or(5).max(1))
  }

  /// The running browser, started if need be.
  pub async fn browser(&self) -> Result<Arc<Browser>, String> {
    let mut live = self.live.lock().await;
    if let Some(l) = live.as_mut()
      && !l.handler.is_finished()
    {
      l.used = Instant::now();
      return Ok(l.browser.clone());
    }
    let exe = self.executable()?;
    // Headless Chrome names itself `HeadlessChrome` in its user agent,
    // which sites refuse outright (CloudFront's bot rules, among others).
    // Its own agent is only known once it runs: the first launch reads it,
    // and when it says headless, Chrome is started again under the agent
    // a browser with a window sends. Kept for later launches of the same
    // executable.
    let known = self
      .agent
      .lock()
      .await
      .as_ref()
      .filter(|(p, _)| *p == exe)
      .map(|(_, a)| a.clone());
    let started = match known {
      Some(agent) => launch(&exe, agent.as_deref()).await?,
      None => {
        let (mut first, handler, profile) = launch(&exe, None).await?;
        let agent = first
          .version()
          .await
          .ok()
          .and_then(|v| windowed_agent(&v.user_agent));
        *self.agent.lock().await = Some((exe.clone(), agent.clone()));
        match agent {
          None => (first, handler, profile),
          Some(agent) => {
            let _ = first.close().await;
            let _ = first.wait().await;
            handler.abort();
            drop(profile);
            launch(&exe, Some(&agent)).await?
          }
        }
      }
    };
    let (browser, handler, profile) = started;
    let browser = Arc::new(browser);
    *live = Some(Live {
      browser: browser.clone(),
      handler,
      used: Instant::now(),
      _profile: profile,
    });
    self.reap_when_idle();
    Ok(browser)
  }

  /// Stops the browser once it has been unused for `idle` and no call
  /// holds it.
  fn reap_when_idle(&self) {
    let (live, idle) = (Arc::downgrade(&self.live), self.idle());
    tokio::spawn(async move {
      loop {
        tokio::time::sleep(idle.min(Duration::from_secs(60))).await;
        let Some(live) = live.upgrade() else { return };
        let mut guard = live.lock().await;
        let Some(l) = guard.as_ref() else { return };
        if l.handler.is_finished() {
          *guard = None;
          return;
        }
        if l.used.elapsed() >= idle && Arc::strong_count(&l.browser) == 1 {
          stop(guard.take());
          return;
        }
      }
    });
  }

  /// Stops the browser now (the node is stopping).
  pub async fn shutdown(&self) {
    stop(self.live.lock().await.take());
  }

  /// The browser to run: configured, installed here, or found on the
  /// system.
  fn executable(&self) -> Result<PathBuf, String> {
    if let Some(p) = &self.settings.chrome {
      return if p.is_file() {
        Ok(p.clone())
      } else {
        Err(format!(
          "browser.chrome is {}, which is not a file",
          p.display()
        ))
      };
    }
    if let Some(p) = installed(&self.data) {
      return Ok(p);
    }
    if let Ok(p) = chromiumoxide::detection::default_executable(
      chromiumoxide::detection::DetectionOptions::default(),
    ) {
      return Ok(p);
    }
    let brave = [
      "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
      "/usr/bin/brave-browser",
      "/usr/bin/brave",
    ];
    brave.iter().map(PathBuf::from).find(|p| p.is_file()).ok_or_else(|| {
      "no Chrome or Chromium here: install one, or run `stitch browser install`, or set browser.chrome in config.toml".to_owned()
    })
  }
}

fn stop(live: Option<Live>) {
  if let Some(l) = live {
    l.handler.abort();
    // Dropping the last handle kills Chrome; its profile goes with `l`.
  }
}

/// The newest Chromium `stitch browser install` put here.
fn installed(data: &Path) -> Option<PathBuf> {
  let mut versions: Vec<PathBuf> = std::fs::read_dir(data.join(INSTALL_DIR))
    .ok()?
    .flatten()
    .map(|e| e.path())
    .collect();
  versions.sort();
  versions.into_iter().rev().find_map(|v| executable_in(&v))
}

fn executable_in(dir: &Path) -> Option<PathBuf> {
  let entries = std::fs::read_dir(dir).ok()?;
  for e in entries.flatten() {
    let name = e.file_name().to_string_lossy().into_owned();
    if name.starts_with("chrome-headless-shell-") {
      let exe = e.path().join(if cfg!(windows) {
        "chrome-headless-shell.exe"
      } else {
        "chrome-headless-shell"
      });
      if exe.is_file() {
        return Some(exe);
      }
    }
  }
  None
}

/// Chrome for Testing's name for this platform.
fn platform() -> Result<&'static str, String> {
  match (std::env::consts::OS, std::env::consts::ARCH) {
    ("macos", "aarch64") => Ok("mac-arm64"),
    ("macos", "x86_64") => Ok("mac-x64"),
    ("linux", "x86_64") => Ok("linux64"),
    ("windows", "x86_64") => Ok("win64"),
    (os, arch) => Err(format!(
      "no Chromium build for {os} {arch}: install a browser and set browser.chrome"
    )),
  }
}

/// Downloads the stable headless Chromium (Chrome for Testing's
/// `chrome-headless-shell`) into the data directory; its executable.
pub async fn install(data: &Path, http: &reqwest::Client) -> Result<PathBuf, String> {
  const INDEX: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";
  let platform = platform()?;
  let index: serde_json::Value = http
    .get(INDEX)
    .send()
    .await
    .and_then(reqwest::Response::error_for_status)
    .map_err(|e| e.to_string())?
    .json()
    .await
    .map_err(|e| e.to_string())?;
  let stable = &index["channels"]["Stable"];
  let version = stable["version"]
    .as_str()
    .ok_or("no stable version listed")?;
  let url = stable["downloads"]["chrome-headless-shell"]
    .as_array()
    .into_iter()
    .flatten()
    .find(|d| d["platform"] == platform)
    .and_then(|d| d["url"].as_str())
    .ok_or_else(|| format!("no chrome-headless-shell for {platform}"))?;
  let dir = data.join(INSTALL_DIR).join(version);
  if let Some(exe) = executable_in(&dir) {
    return Ok(exe);
  }
  let mut response = http
    .get(url)
    .send()
    .await
    .and_then(reqwest::Response::error_for_status)
    .map_err(|e| e.to_string())?;
  let mut zip = tempfile::tempfile().map_err(|e| e.to_string())?;
  while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
    std::io::Write::write_all(&mut zip, &chunk).map_err(|e| e.to_string())?;
  }
  let dest = dir.clone();
  tokio::task::spawn_blocking(move || -> Result<(), String> {
    let partial = dest.with_extension("partial");
    let _ = std::fs::remove_dir_all(&partial);
    zip::ZipArchive::new(zip)
      .and_then(|mut a| a.extract(&partial))
      .map_err(|e| format!("could not unpack Chromium: {e}"))?;
    std::fs::rename(&partial, &dest).map_err(|e| e.to_string())
  })
  .await
  .map_err(|e| e.to_string())??;
  executable_in(&dir).ok_or_else(|| format!("no executable in {}", dir.display()))
}
