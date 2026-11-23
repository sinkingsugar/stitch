//! `proxy.toml` in the proxy's directory (`<data>/proxy`):
//!
//! ```toml
//! listen = ["127.0.0.1:7178", "172.17.0.1:7178"]
//!
//! [upstreams.anthropic]        # built in, as are `openai` and `gemini`
//! kind = "anthropic"           # or "openai": OpenAI-compatible
//! base_url = "https://api.anthropic.com"
//! key = "anthropic"            # the secret holding its API key
//!
//! [tokens.discord-agent]
//! hash = "sha256:…"            # of the token; the token itself is shown once
//! upstreams = ["anthropic"]    # empty: every upstream
//! daily_tokens = 2000000
//! daily_tokens_per_user = 300000
//! requests_per_minute = 20
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const FILE: &str = "proxy.toml";
pub const PORT: u16 = 7178;
/// Where it listens unless `listen` says: this machine, and the gateway
/// of Docker's default bridge (containers reach it as
/// `host.docker.internal`); never every address.
pub const DEFAULT_LISTEN: &[&str] = &["127.0.0.1:7178", "172.17.0.1:7178"];
/// A token's requests a minute unless it says.
pub const DEFAULT_RPM: u32 = 30;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub listen: Vec<String>,
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub upstreams: BTreeMap<String, Upstream>,
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub tokens: BTreeMap<String, Token>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upstream {
  /// `anthropic` (`/v1/messages`, `/v1/messages/count_tokens`,
  /// `/v1/models`) or `openai` (`/chat/completions`, `/responses`,
  /// `/models` under its base URL).
  pub kind: String,
  pub base_url: String,
  /// The secret (provider key) on this machine; the upstream's name when
  /// unset.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub key: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
  /// `sha256:<hex>` of the token.
  pub hash: String,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub upstreams: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub daily_tokens: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub daily_tokens_per_user: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub requests_per_minute: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub created: Option<String>,
}

impl Config {
  pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
  }

  pub fn load(dir: &Path) -> Result<Config, String> {
    let path = Config::path(dir);
    match std::fs::read_to_string(&path) {
      Ok(t) => toml::from_str(&t).map_err(|e| format!("{}: {e}", path.display())),
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
      Err(e) => Err(format!("{}: {e}", path.display())),
    }
  }

  pub fn save(&self, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = toml::to_string(self).map_err(|e| e.to_string())?;
    let tmp = dir.join("proxy.toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, Config::path(dir)).map_err(|e| e.to_string())
  }

  /// The upstreams: the built-in ones, with the configured ones over them.
  pub fn upstreams(&self) -> BTreeMap<String, Upstream> {
    let mut all = BTreeMap::new();
    let mut put = |name: &str, kind: &str, base: &str| {
      all.insert(
        name.to_owned(),
        Upstream {
          kind: kind.into(),
          base_url: base.into(),
          key: None,
        },
      );
    };
    put("anthropic", "anthropic", "https://api.anthropic.com");
    put("openai", "openai", "https://api.openai.com/v1");
    put(
      "gemini",
      "openai",
      "https://generativelanguage.googleapis.com/v1beta/openai",
    );
    for (k, v) in &self.upstreams {
      all.insert(k.clone(), v.clone());
    }
    all
  }

  /// The token named by its clear text, if one matches.
  pub fn token_of(&self, clear: &str) -> Option<(&String, &Token)> {
    let h = hash(clear);
    self.tokens.iter().find(|(_, t)| t.hash == h)
  }
}

pub fn hash(clear: &str) -> String {
  let d = Sha256::digest(clear.as_bytes());
  let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
  format!("sha256:{hex}")
}

/// A new token's clear text: `stp_` and 32 random bytes.
pub fn new_token() -> String {
  let bytes: [u8; 32] = rand::random();
  format!(
    "stp_{}",
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
  )
}
