//! Tools written in Otter (design §2.17): scripts (`.otr`, `.otter` or
//! `.shs`) in a skill's `tools/` directory, compiled by Otter while the
//! node runs and offered to the model next to the built-in tools.
//!
//! - **Where they come from:** every skill a thread has on brings the
//!   scripts in its `tools/` directory: the home workspace's and the
//!   place's skills from their files, Stitch's own from the binary. Turning
//!   a skill off turns its tools off. A skill whose files are another
//!   node's brings none.
//! - **A script** declares functions and ends with its definition, a table
//!   its top-level code outputs: `{definition: {name description
//!   parameters} use: "Function"}` (`otter_lang::tool`). The function takes
//!   `{input env working-dir}` (the model's arguments, an empty table, the
//!   thread's directory) and outputs the result: text, or any value given
//!   as JSON. Its catalog is the `otter` command's: core, I/O (HTTP, files)
//!   and text shards, so a script `otter tool` runs runs here.
//! - **Compiled once, checked first:** a script is parsed, lowered and
//!   composed when a loop first offers it, and again when it, a file in its
//!   `tools/` directory or a file it includes changes; one that does not
//!   compile is not offered, its diagnostics in the node's log once.
//!   Compiled scripts are kept per node.
//! - **Names** are the definition's: `[A-Za-z0-9_-]`, at most 64, no `__`
//!   (MCP tools' separator), not a built-in tool's; the first by skill and
//!   file name wins a clash, the other is logged.
//! - **Calls ask** as an MCP call does (§2.13), unless the tool has no
//!   effects (it only computes: Otter infers that through everything it
//!   calls): they run without asking when `[otter] allow` patterns match
//!   the tool's name, when an "always" answer allowed it in the thread's
//!   place (`[otter.places."<place>"] allow`), or in yolo; a read-only
//!   thread is refused the rest. A call runs on a blocking thread, at most
//!   `[otter] timeout_secs` (120, at most 3600), and is cancelled when the
//!   user interrupts it. Its files are confined to the thread's directory
//!   (read only in a read-only thread).
//! - **On every node,** the phone included: there a thread's directory is
//!   its place's copy in the app's container, or the container itself for
//!   a global thread, and an approval is asked in the app.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub(crate) mod calls;
mod decide;
pub mod edge_talk;
mod host;
pub(crate) mod language;

pub use host::OtterTools;
pub(crate) use host::Scripts;

/// The catalog scripts compile against (a capability profile's shard rule
/// is checked over it).
#[cfg(test)]
pub(crate) fn host_catalog() -> Result<&'static otter_core::Catalog, String> {
  host::catalog()
}

/// The default and the longest a call may run, in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 3600;

/// `[otter]` in config.toml.
///
/// ```toml
/// [otter]
/// allow = ["google_flights_*"]        # tools that run without asking
/// ask = ["edge_talk_*"]               # tools that ask though they need not
/// timeout_secs = 120
///
/// [otter.places."project:p-42ada59c2195414f"]
/// allow = ["google_flights_search"]   # added there by "always" answers (or by hand)
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OtterSettings {
  /// Patterns over tool names (`*` matching any run of characters).
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub allow: Vec<String>,
  /// Patterns over the names of tools that run without asking by default
  /// (the built-in `edge-talk` skill's): these ask as any other does.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub ask: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout_secs: Option<u64>,
  /// Tools allowed in one place only (a project, a folder, `global`), by
  /// its key.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub places: BTreeMap<String, PlaceOtter>,
}

/// `[otter.places."<place>"]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceOtter {
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub allow: Vec<String>,
}

impl OtterSettings {
  pub fn timeout(&self) -> std::time::Duration {
    std::time::Duration::from_secs(
      self
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS),
    )
  }
}

/// Where a script is: a file here, or a built-in skill's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
  File(PathBuf),
  BuiltIn {
    skill: &'static str,
    path: &'static str,
  },
}

/// A script a skill brings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Script {
  pub skill: String,
  pub origin: Origin,
  /// Its skill's tools are lazy: not listed while `otter_run` can call
  /// them (`Tool.Call`).
  pub lazy: bool,
}

/// The extensions a script has: `.otr` for new ones, `.otter`, and `.shs`
/// (the language's before it was named Otter). Otter itself never looks at
/// a file's extension; these say which files in `tools/` are scripts.
pub const EXTENSIONS: [&str; 3] = ["otr", "otter", "shs"];

fn is_script(name: &str) -> bool {
  name
    .rsplit_once('.')
    .is_some_and(|(stem, ext)| !stem.is_empty() && EXTENSIONS.contains(&ext))
}

/// The scripts in a skill directory's `tools/`, by file name.
pub(crate) fn scripts_in(skill: &str, dir: &Path) -> Vec<Script> {
  let Ok(entries) = std::fs::read_dir(dir.join("tools")) else {
    return Vec::new();
  };
  let mut paths: Vec<PathBuf> = entries
    .flatten()
    .map(|e| e.path())
    .filter(|p| {
      p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(is_script)
        && p.is_file()
    })
    .collect();
  paths.sort();
  paths
    .into_iter()
    .map(|p| Script {
      skill: skill.to_owned(),
      origin: Origin::File(p),
      lazy: false,
    })
    .collect()
}

/// A built-in skill's tool scripts: its files `tools/*.otr` (or `.otter`,
/// `.shs`). A script that
/// defines no tool (a library others include) is skipped when compiled.
pub(crate) fn built_in_scripts(skill: &crate::apps::BuiltInSkill) -> Vec<Script> {
  skill
    .files
    .iter()
    .filter(|(p, _)| is_tool_path(p))
    .map(|(p, _)| Script {
      skill: skill.name.to_owned(),
      origin: Origin::BuiltIn {
        skill: skill.name,
        path: p,
      },
      lazy: skill.lazy,
    })
    .collect()
}

fn is_tool_path(p: &str) -> bool {
  p.strip_prefix("tools/")
    .is_some_and(|f| !f.contains('/') && is_script(f))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_skill_directory_brings_its_tools_folder() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("tools/sub")).unwrap();
    for f in [
      "b.shs",
      "a.shs",
      "c.otr",
      "d.otter",
      "notes.md",
      ".otr",
      "e.otr.txt",
      "sub/c.shs",
    ] {
      std::fs::write(dir.path().join("tools").join(f), "").unwrap();
    }
    let got: Vec<PathBuf> = scripts_in("demo", dir.path())
      .into_iter()
      .map(|s| match s.origin {
        Origin::File(p) => p,
        Origin::BuiltIn { .. } => unreachable!(),
      })
      .collect();
    assert_eq!(
      got,
      vec![
        dir.path().join("tools/a.shs"),
        dir.path().join("tools/b.shs"),
        dir.path().join("tools/c.otr"),
        dir.path().join("tools/d.otter"),
      ]
    );
    assert!(scripts_in("none", &dir.path().join("missing")).is_empty());
    for (path, tool) in [
      ("tools/a.otr", true),
      ("tools/a.otter", true),
      ("tools/a.shs", true),
      ("tools/a.md", false),
      ("tools/sub/a.otr", false),
      ("a.otr", false),
    ] {
      assert_eq!(is_tool_path(path), tool, "{path}");
    }
  }

  #[test]
  fn the_flights_skill_brings_its_scripts() {
    let flights = crate::apps::SKILLS
      .iter()
      .find(|b| b.name == "google-flights")
      .unwrap();
    let paths: Vec<&str> = built_in_scripts(flights)
      .iter()
      .map(|s| match s.origin {
        Origin::BuiltIn { path, .. } => path,
        Origin::File(_) => unreachable!(),
      })
      .collect();
    assert_eq!(
      paths,
      [
        "tools/flights.otr",
        "tools/search-flights.otr",
        "tools/cheap-dates.otr"
      ]
    );
    let leadsheet = crate::apps::SKILLS
      .iter()
      .find(|b| b.name == "leadsheet")
      .unwrap();
    assert!(built_in_scripts(leadsheet).is_empty());
  }
}
