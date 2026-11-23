//! `~/.ssh/config`, as far as Stitch needs it: which hosts it names, and
//! for one, its address, port, user, jump host and identity files. As
//! OpenSSH reads it: every `Host` block whose patterns match, in order,
//! the first value of each keyword winning. `Match` blocks are skipped,
//! and `Include` is followed (relative to `~/.ssh`, globs of one `*`).

use std::path::{Path, PathBuf};

/// One host as the config says it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
  pub host_name: Option<String>,
  pub user: Option<String>,
  pub port: Option<u16>,
  pub proxy_jump: Option<String>,
  pub identity_files: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default)]
struct Block {
  patterns: Vec<String>,
  /// Keyword (lowercase), value; in order.
  options: Vec<(String, String)>,
}

/// A parsed config.
#[derive(Clone, Debug, Default)]
pub struct SshConfig {
  blocks: Vec<Block>,
  home: PathBuf,
}

impl SshConfig {
  /// `~/.ssh/config` (empty when there is none).
  pub fn load(home: &Path) -> SshConfig {
    let path = home.join(".ssh/config");
    let mut cfg = SshConfig {
      blocks: Vec::new(),
      home: home.to_owned(),
    };
    cfg.read(&path, 0);
    cfg
  }

  /// A config from its text (tests).
  pub fn parse(text: &str, home: &Path) -> SshConfig {
    let mut cfg = SshConfig {
      blocks: Vec::new(),
      home: home.to_owned(),
    };
    cfg.add(text, 0);
    cfg
  }

  fn read(&mut self, path: &Path, depth: u32) {
    if let Ok(text) = std::fs::read_to_string(path) {
      self.add(&text, depth);
    }
  }

  fn add(&mut self, text: &str, depth: u32) {
    // Options before the first `Host` apply to every host.
    let mut current = Block {
      patterns: vec!["*".into()],
      options: Vec::new(),
    };
    let mut skipping = false;
    for line in text.lines() {
      let line = line.trim();
      if line.is_empty() || line.starts_with('#') {
        continue;
      }
      let (key, value) = split_option(line);
      let key = key.to_ascii_lowercase();
      match key.as_str() {
        "host" => {
          self.blocks.push(std::mem::take(&mut current));
          current.patterns = words(&value);
          skipping = false;
        }
        "match" => {
          self.blocks.push(std::mem::take(&mut current));
          skipping = true;
        }
        _ if skipping => {}
        "include" if depth < 8 => {
          // What it includes belongs to the block it is in.
          let patterns = current.patterns.clone();
          self.blocks.push(std::mem::take(&mut current));
          for pattern in words(&value) {
            for path in self.included(&pattern) {
              self.read(&path, depth + 1);
            }
          }
          current = Block {
            patterns,
            options: Vec::new(),
          };
        }
        _ => current.options.push((key, unquote(&value))),
      }
    }
    self.blocks.push(current);
    self.blocks.retain(|b| !b.patterns.is_empty());
  }

  fn included(&self, pattern: &str) -> Vec<PathBuf> {
    let path = expand(pattern, &self.home);
    let path = if path.is_absolute() {
      path
    } else {
      self.home.join(".ssh").join(path)
    };
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
      return Vec::new();
    };
    if !name.contains('*') {
      return vec![path];
    }
    let Some(dir) = path.parent() else {
      return Vec::new();
    };
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
      .into_iter()
      .flatten()
      .flatten()
      .filter(|e| e.file_name().to_str().is_some_and(|n| matches(name, n)))
      .map(|e| e.path())
      .collect();
    out.sort();
    out
  }

  /// The names it defines (`Host` patterns without wildcards).
  pub fn hosts(&self) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for b in &self.blocks {
      for p in &b.patterns {
        if !p.contains(['*', '?', '!']) && !out.contains(p) {
          out.push(p.clone());
        }
      }
    }
    out
  }

  /// What it says of `host`; `None` when no block names it (the blocks
  /// of `*` alone do not count).
  pub fn entry(&self, host: &str) -> Option<Entry> {
    let mut named = false;
    let mut e = Entry::default();
    let mut seen: Vec<&str> = Vec::new();
    for b in &self.blocks {
      if !block_matches(&b.patterns, host) {
        continue;
      }
      named |= b.patterns.iter().any(|p| p == host);
      for (k, v) in &b.options {
        if k == "identityfile" {
          e.identity_files.push(expand(v, &self.home));
          continue;
        }
        if seen.contains(&k.as_str()) {
          continue;
        }
        seen.push(k);
        match k.as_str() {
          "hostname" => e.host_name = Some(v.replace("%h", host).replace("%%", "%")),
          "user" => e.user = Some(v.clone()),
          "port" => e.port = v.parse().ok(),
          "proxyjump" if v != "none" => e.proxy_jump = Some(v.clone()),
          _ => {}
        }
      }
    }
    named.then_some(e)
  }
}

fn split_option(line: &str) -> (String, String) {
  let line = line.trim();
  let end = line
    .find(|c: char| c.is_whitespace() || c == '=')
    .unwrap_or(line.len());
  let key = line[..end].to_owned();
  let rest = line[end..].trim_start();
  let rest = rest.strip_prefix('=').unwrap_or(rest).trim();
  (key, rest.to_owned())
}

fn words(value: &str) -> Vec<String> {
  value.split_whitespace().map(unquote).collect()
}

fn unquote(v: &str) -> String {
  v.trim().trim_matches('"').to_owned()
}

fn expand(path: &str, home: &Path) -> PathBuf {
  match path.strip_prefix("~/") {
    Some(rest) => home.join(rest),
    None if path == "~" => home.to_owned(),
    None => PathBuf::from(path.replace("%d", &home.display().to_string())),
  }
}

/// Whether a block's patterns take `host`: one matches, and no negated
/// one does.
fn block_matches(patterns: &[String], host: &str) -> bool {
  let mut any = false;
  for p in patterns {
    if let Some(neg) = p.strip_prefix('!') {
      if matches(neg, host) {
        return false;
      }
    } else if matches(p, host) {
      any = true;
    }
  }
  any
}

/// A glob of `*` and `?`.
fn matches(pattern: &str, text: &str) -> bool {
  let p: Vec<char> = pattern.chars().collect();
  let t: Vec<char> = text.chars().collect();
  let (mut pi, mut ti) = (0, 0);
  let (mut star, mut mark) = (None, 0);
  while ti < t.len() {
    if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
      pi += 1;
      ti += 1;
    } else if pi < p.len() && p[pi] == '*' {
      star = Some(pi);
      mark = ti;
      pi += 1;
    } else if let Some(s) = star {
      pi = s + 1;
      mark += 1;
      ti = mark;
    } else {
      return false;
    }
  }
  while pi < p.len() && p[pi] == '*' {
    pi += 1;
  }
  pi == p.len()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_host_is_read_as_openssh_reads_it() {
    let home = Path::new("/home/me");
    let cfg = SshConfig::parse(
      "User fallback\n\
       Host devbox-1 devbox\n  HostName 10.0.0.5\n  Port 2222\n  IdentityFile ~/.ssh/devbox\n\
       Host pi\n  HostName %h.local\n  ProxyJump devbox-1\n\
       Host *.lan !nope.lan\n  User lanuser\n\
       Match host foo\n  User matched\n\
       Host *\n  User star\n  IdentityFile ~/.ssh/id_ed25519\n",
      home,
    );
    assert_eq!(cfg.hosts(), ["devbox-1", "devbox", "pi"]);
    let e = cfg.entry("devbox-1").unwrap();
    assert_eq!(e.host_name.as_deref(), Some("10.0.0.5"));
    assert_eq!(e.port, Some(2222));
    // The first value wins: the one before any Host.
    assert_eq!(e.user.as_deref(), Some("fallback"));
    assert_eq!(
      e.identity_files,
      [
        PathBuf::from("/home/me/.ssh/devbox"),
        PathBuf::from("/home/me/.ssh/id_ed25519")
      ]
    );
    let pi = cfg.entry("pi").unwrap();
    assert_eq!(pi.host_name.as_deref(), Some("pi.local"));
    assert_eq!(pi.proxy_jump.as_deref(), Some("devbox-1"));
    assert!(cfg.entry("box.lan").is_none(), "only patterns name it");
    assert!(cfg.entry("unknown").is_none());
    assert!(matches("*.lan", "box.lan") && !matches("*.lan", "box.lanx"));
    assert!(block_matches(
      &["*.lan".into(), "!nope.lan".into()],
      "a.lan"
    ));
    assert!(!block_matches(
      &["*.lan".into(), "!nope.lan".into()],
      "nope.lan"
    ));
  }
}
