//! What each token used, by UTC day and by end user: `usage.json` (kept
//! across restarts, the last `KEEP_DAYS` days), and a line per request in
//! `usage.log` (JSON Lines).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::meter::Counts;

pub const FILE: &str = "usage.json";
pub const LOG: &str = "usage.log";
const KEEP_DAYS: usize = 40;
/// The end user of requests that name none.
pub const NOBODY: &str = "-";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TokenDay {
  #[serde(default)]
  pub total: Counts,
  #[serde(default)]
  pub users: BTreeMap<String, Counts>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Book {
  /// `YYYY-MM-DD` (UTC) → token → its day.
  #[serde(default)]
  pub days: BTreeMap<String, BTreeMap<String, TokenDay>>,
}

pub fn today() -> String {
  jiff::Timestamp::now()
    .to_zoned(jiff::tz::TimeZone::UTC)
    .strftime("%Y-%m-%d")
    .to_string()
}

/// Seconds until the next UTC midnight, when daily budgets start again.
pub fn secs_to_midnight() -> i64 {
  let now = jiff::Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC);
  let since = i64::from(now.hour()) * 3600 + i64::from(now.minute()) * 60 + i64::from(now.second());
  86_400 - since
}

impl Book {
  pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
  }

  pub fn load(dir: &Path) -> Book {
    std::fs::read(Book::path(dir))
      .ok()
      .and_then(|b| serde_json::from_slice(&b).ok())
      .unwrap_or_default()
  }

  pub fn save(&self, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("usage.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
    std::fs::rename(&tmp, Book::path(dir))
  }

  pub fn day(&self, day: &str, token: &str) -> Option<&TokenDay> {
    self.days.get(day).and_then(|d| d.get(token))
  }

  pub fn record(&mut self, day: &str, token: &str, user: &str, c: &Counts) {
    let d = self
      .days
      .entry(day.to_owned())
      .or_default()
      .entry(token.to_owned())
      .or_default();
    d.total.add(c);
    d.users.entry(user.to_owned()).or_default().add(c);
    while self.days.len() > KEEP_DAYS {
      let first = self.days.keys().next().cloned().expect("not empty");
      self.days.remove(&first);
    }
  }
}

/// A line of `usage.log`.
#[derive(Debug, Serialize, Deserialize)]
pub struct Line {
  pub at: String,
  pub token: String,
  pub user: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub thread: Option<String>,
  pub upstream: String,
  pub path: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
  pub status: u16,
  pub ms: u64,
  /// The client went away before the response ended.
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub cut: bool,
  #[serde(flatten)]
  pub counts: Counts,
}

pub fn log(dir: &Path, line: &Line) {
  let Ok(text) = serde_json::to_string(line) else {
    return;
  };
  if let Ok(mut f) = std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(dir.join(LOG))
  {
    let _ = writeln!(f, "{text}");
  }
}
