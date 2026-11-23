//! A place's own settings and skills (design §2.14), for a project's
//! checkout and a folder alike: `<place>/.stitch/settings.toml` and
//! `<place>/.stitch/skills/<name>/SKILL.md`.
//!
//! ```toml
//! enable = ["mail"]          # tools: groups, MCP servers, patterns
//! disable = ["web"]
//! memory = false             # its threads out of memory (design §2.9)
//!
//! [models]                   # over the user's (design §2.7)
//! default = "anthropic/claude-opus-5-5"
//! quick = "anthropic/claude-sonnet-5-5"
//!
//! [skills]
//! disable = ["release"]      # the home workspace's skills, here
//!
//! [env]                      # what its shells get (design §2.19)
//! OPENAI_API_KEY = "secret:openai"
//! HF_HOME = "var:hf_home"
//! ```
//!
//! A node that has the place's directory reads its files at each turn and,
//! when they differ from what it last published, writes them as a `place`
//! record in the home workspace, so a node without the directory (the
//! phone) knows them: it uses the latest record of any node.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use stitch_records::{Author, Draft, NodeName, kinds};
use stitch_store::{Index, RecordRoot};

use crate::context::Skill;
use crate::toolset::Entry;
use crate::{Error, Result};

/// A skill's text, at most, in a record.
const MAX_SKILL: usize = 64 * 1024;

/// `settings.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub enable: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub disable: Vec<String>,
  /// `false`: the place's threads start out of memory, as with
  /// `/memory off` (design §2.9); `/memory on` brings one back.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub memory: Option<bool>,
  /// Its own default and quick model (design §2.7), over the user's.
  #[serde(default, skip_serializing_if = "PlaceModels::is_empty")]
  pub models: PlaceModels,
  #[serde(default)]
  pub skills: SkillSettings,
  /// What the place's shells get as environment variables (design
  /// §2.19): `OPENAI_API_KEY = "secret:openai"` (the place's secret of
  /// that name, else the global one), `"var:<name>"` (a variable), or a
  /// value as written. Only references: the values never are in the file.
  #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
  pub env: std::collections::BTreeMap<String, String>,
  /// The approval mode of the place's threads (design §2.28), over the
  /// node's (`stitch mode`): stricter or looser. Absent: the node's.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub approval: Option<PlaceApproval>,
}

/// `approval = "ask" | "auto" | "yolo"` in a place's settings, the values
/// `stitch mode` takes (`default` for `ask`, `allow` for `yolo`, as
/// config.toml says it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaceApproval {
  #[serde(alias = "default")]
  Ask,
  Auto,
  #[serde(alias = "allow")]
  Yolo,
}

impl PlaceApproval {
  pub fn parse(s: &str) -> Option<Self> {
    match s.trim() {
      "ask" | "default" => Some(Self::Ask),
      "auto" => Some(Self::Auto),
      "yolo" | "allow" => Some(Self::Yolo),
      _ => None,
    }
  }

  pub fn as_str(self) -> &'static str {
    match self {
      Self::Ask => "ask",
      Self::Auto => "auto",
      Self::Yolo => "yolo",
    }
  }

  /// The mode the rules take in the place, where the node's is `node`
  /// (and the node's default ask would be `default_ask`): `ask` keeps the
  /// node's own ask (`sandbox` or `ask`), else the default; an `off`
  /// shell stays off (it is the node's, not a mode of approval).
  pub fn effective(self, node: &str, default_ask: &str) -> String {
    match (self, node) {
      (_, "off") => "off".into(),
      (Self::Ask, "sandbox" | "ask") => node.into(),
      (Self::Ask, _) => default_ask.into(),
      (Self::Auto, _) => crate::rules::AUTO.into(),
      (Self::Yolo, _) => "allow".into(),
    }
  }
}

/// `[models]`: `provider/model` each.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceModels {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub default: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub quick: Option<String>,
}

impl PlaceModels {
  fn is_empty(&self) -> bool {
    self.default.is_none() && self.quick.is_none()
  }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillSettings {
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub disable: Vec<String>,
}

/// A place skill as a record carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceSkill {
  pub name: String,
  pub description: String,
  pub text: String,
  /// Its file, on the node that read it.
  pub path: Option<PathBuf>,
}

/// What a place says: its settings and skills, and where they come from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaceFiles {
  pub settings: Settings,
  pub skills: Vec<PlaceSkill>,
  /// The node whose record this is, when the files are not here.
  pub from: Option<String>,
}

impl PlaceFiles {
  fn is_empty(&self) -> bool {
    self.settings == Settings::default() && self.skills.is_empty()
  }

  /// Its tool entries: `enable` on, `disable` off.
  pub fn tool_entries(&self) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for (list, on) in [
      (&self.settings.enable, true),
      (&self.settings.disable, false),
    ] {
      for e in list {
        match Entry::parse(e) {
          Ok(mut e) => {
            e.on = on && e.on;
            out.push(e);
          }
          Err(err) => eprintln!("place settings: {err}"),
        }
      }
    }
    out
  }

  /// Its skill entries: `[skills] disable` off.
  pub fn skill_entries(&self) -> Vec<Entry> {
    self
      .settings
      .skills
      .disable
      .iter()
      .filter_map(|s| Entry::parse(s).ok())
      .map(|e| Entry::off(&e.pattern))
      .collect()
  }

  /// Its skills as the context lists them: with their file here, else
  /// to be read with `read_skill`.
  pub fn listed(&self) -> Vec<Skill> {
    self
      .skills
      .iter()
      .map(|k| Skill {
        name: k.name.clone(),
        description: k.description.clone(),
        path: k.path.clone().unwrap_or_default(),
        from: match &k.path {
          Some(_) => None,
          None => Some(self.from.clone().unwrap_or_default()),
        },
        lazy: crate::context::lazy_tools(&k.text),
        lazy_tools: Vec::new(),
      })
      .collect()
  }
}

/// The settings and skills in `dir/.stitch`: `Ok(None)` without that
/// directory.
pub fn read(dir: &Path) -> Result<Option<PlaceFiles>> {
  let base = dir.join(".stitch");
  if !base.is_dir() {
    return Ok(None);
  }
  let path = base.join("settings.toml");
  let settings = match std::fs::read_to_string(&path) {
    Ok(text) => {
      toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?
    }
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
    Err(e) => return Err(Error::io(path.display())(e)),
  };
  let skills = crate::context::skills_in(&base.join("skills"))
    .into_iter()
    .filter_map(|s| {
      let text = std::fs::read_to_string(&s.path).ok()?;
      Some(PlaceSkill {
        name: s.name,
        description: s.description,
        text: cap(text),
        path: Some(s.path),
      })
    })
    .collect();
  Ok(Some(PlaceFiles {
    settings,
    skills,
    from: None,
  }))
}

fn cap(mut text: String) -> String {
  if text.len() > MAX_SKILL {
    let mut end = MAX_SKILL;
    while !text.is_char_boundary(end) {
      end -= 1;
    }
    text.truncate(end);
    text.push_str("\n\n[… cut]");
  }
  text
}

/// What the `place` records say about `key`: the latest of `node`'s when
/// given, else the latest of any node's.
fn recorded(index: &Index, key: &str, node: Option<&str>) -> Option<(String, PlaceFiles)> {
  // Shared: a list asks once per place it counts, and these records hold
  // their skills whole (5.6 MB on gamer, read past SQLite's cache each time).
  let all = index.records_shared(kinds::PLACE).unwrap_or_default();
  let mut records: Vec<_> = all
    .iter()
    .filter(|r| r.fields.get("place").and_then(Value::as_str) == Some(key))
    .filter(|r| node.is_none_or(|n| r.node == n))
    .collect();
  records.sort_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
  let r = records.pop()?;
  let strings = |v: Option<&Value>| -> Vec<String> {
    v.and_then(Value::as_array)
      .map(|a| {
        a.iter()
          .filter_map(Value::as_str)
          .map(str::to_owned)
          .collect()
      })
      .unwrap_or_default()
  };
  let skills = r
    .fields
    .get("skills")
    .and_then(Value::as_array)
    .map(|a| {
      a.iter()
        .filter_map(|s| {
          Some(PlaceSkill {
            name: s.get("name")?.as_str()?.to_owned(),
            description: s
              .get("description")
              .and_then(Value::as_str)
              .unwrap_or("")
              .to_owned(),
            text: s
              .get("text")
              .and_then(Value::as_str)
              .unwrap_or("")
              .to_owned(),
            path: None,
          })
        })
        .collect()
    })
    .unwrap_or_default();
  Some((
    r.node.clone(),
    PlaceFiles {
      settings: Settings {
        enable: strings(r.fields.get("enable")),
        disable: strings(r.fields.get("disable")),
        memory: r.fields.get("memory").and_then(Value::as_bool),
        models: PlaceModels {
          default: r
            .fields
            .get("default_model")
            .and_then(Value::as_str)
            .map(str::to_owned),
          quick: r
            .fields
            .get("quick_model")
            .and_then(Value::as_str)
            .map(str::to_owned),
        },
        skills: SkillSettings {
          disable: strings(r.fields.get("skills_disable")),
        },
        env: r
          .fields
          .get("env")
          .and_then(Value::as_object)
          .map(|m| {
            m.iter()
              .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
              .collect()
          })
          .unwrap_or_default(),
        approval: r
          .fields
          .get("approval")
          .and_then(Value::as_str)
          .and_then(PlaceApproval::parse),
      },
      skills,
      from: Some(r.node.clone()),
    },
  ))
}

/// Whether two readings say the same (where each came from aside).
fn same(a: &PlaceFiles, b: &PlaceFiles) -> bool {
  let strip = |f: &PlaceFiles| {
    (
      f.settings.clone(),
      f.skills
        .iter()
        .map(|s| (s.name.clone(), s.description.clone(), s.text.clone()))
        .collect::<Vec<_>>(),
    )
  };
  strip(a) == strip(b)
}

/// The settings and skills of the place `key`, whose directory here is
/// `dir`: read from its files when it is here (and published when they
/// changed since this node last did), else from the latest record.
pub fn of(
  home: Option<&RecordRoot>,
  index: &std::sync::Mutex<Index>,
  node: &NodeName,
  key: &str,
  dir: Option<&Path>,
) -> PlaceFiles {
  let local = match dir.map(|d| (d, read(d))) {
    Some((_, Ok(Some(files)))) => Some(files),
    // The place is here without its own settings (`.stitch` never made,
    // or deleted): none, published as none when they were some.
    Some((d, Ok(None))) => d.is_dir().then(PlaceFiles::default),
    Some((_, Err(e))) => {
      eprintln!("place {key}: {e}");
      None
    }
    None => None,
  };
  match (local, home) {
    (Some(files), Some(home)) => {
      let mine = recorded(&index.lock().expect("index lock"), key, Some(node.as_str()));
      let changed = match &mine {
        Some((_, before)) => !same(before, &files),
        None => !files.is_empty(),
      };
      if changed && let Err(e) = publish(home, index, node, key, &files) {
        eprintln!("place {key}: not published: {e}");
      }
      files
    }
    (Some(files), None) => files,
    (None, _) => recorded(&index.lock().expect("index lock"), key, None)
      .map(|(_, f)| f)
      .unwrap_or_default(),
  }
}

fn publish(
  home: &RecordRoot,
  index: &std::sync::Mutex<Index>,
  node: &NodeName,
  key: &str,
  files: &PlaceFiles,
) -> Result<()> {
  let list = |l: &[String]| toml::Value::Array(l.iter().map(|s| s.as_str().into()).collect());
  let skills = toml::Value::Array(
    files
      .skills
      .iter()
      .map(|s| {
        let mut t = toml::Table::new();
        t.insert("name".into(), s.name.as_str().into());
        t.insert("description".into(), s.description.as_str().into());
        t.insert("text".into(), s.text.as_str().into());
        toml::Value::Table(t)
      })
      .collect(),
  );
  let mut draft = Draft::new(kinds::PLACE, Author::node(node.clone()))?
    .field("place", key)?
    .field("node", node.as_str())?
    .field("enable", list(&files.settings.enable))?
    .field("disable", list(&files.settings.disable))?;
  if let Some(on) = files.settings.memory {
    draft = draft.field("memory", on)?;
  }
  if let Some(m) = &files.settings.models.default {
    draft = draft.field("default_model", m.as_str())?;
  }
  if let Some(m) = &files.settings.models.quick {
    draft = draft.field("quick_model", m.as_str())?;
  }
  if let Some(a) = files.settings.approval {
    draft = draft.field("approval", a.as_str())?;
  }
  if !files.settings.env.is_empty() {
    let env: toml::Table = files
      .settings
      .env
      .iter()
      .map(|(k, v)| (k.clone(), toml::Value::from(v.as_str())))
      .collect();
    draft = draft.field("env", toml::Value::Table(env))?;
  }
  let draft = draft
    .field("skills_disable", list(&files.settings.skills.disable))?
    .field("skills", skills)?
    .body(format!("# The settings of {key}, as {node} has them\n"));
  home.publish(node, draft)?;
  index.lock().expect("index lock").refresh(home)?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_places_files_are_read_into_entries_and_skills() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read(dir.path()).unwrap(), None, "no .stitch: nothing");
    let base = dir.path().join(".stitch");
    std::fs::create_dir_all(base.join("skills/mail")).unwrap();
    std::fs::write(
      base.join("settings.toml"),
      "enable = [\"mail\", \"-github__create_*\"]\ndisable = [\"web\"]\n[skills]\ndisable = [\"release\"]\n",
    )
    .unwrap();
    std::fs::write(
      base.join("skills/mail/SKILL.md"),
      "---\nname: mail\ndescription: Handle mail.\n---\nArchive what is read.\n",
    )
    .unwrap();
    let files = read(dir.path()).unwrap().unwrap();
    assert_eq!(files.tool_entries(), ["mail", "-github__create_*", "-web"]);
    assert_eq!(files.skill_entries(), ["-release"]);
    let listed = files.listed();
    assert_eq!(listed.len(), 1);
    assert_eq!(
      (
        listed[0].name.as_str(),
        listed[0].description.as_str(),
        &listed[0].from
      ),
      ("mail", "Handle mail.", &None)
    );
    assert!(files.skills[0].text.contains("Archive what is read."));
    // A setting it does not know is an error, not ignored.
    std::fs::write(base.join("settings.toml"), "enabled = [\"mail\"]\n").unwrap();
    assert!(read(dir.path()).is_err());
  }

  #[test]
  fn a_places_approval_is_read_and_wins_over_the_nodes_both_ways() {
    let parse = |t: &str| toml::from_str::<Settings>(t).map(|s| s.approval);
    assert_eq!(parse("").unwrap(), None, "absent: the node's");
    assert_eq!(
      parse("approval = \"ask\"").unwrap(),
      Some(PlaceApproval::Ask)
    );
    assert_eq!(
      parse("approval = \"default\"").unwrap(),
      Some(PlaceApproval::Ask)
    );
    assert_eq!(
      parse("approval = \"auto\"").unwrap(),
      Some(PlaceApproval::Auto)
    );
    assert_eq!(
      parse("approval = \"yolo\"").unwrap(),
      Some(PlaceApproval::Yolo)
    );
    assert_eq!(
      parse("approval = \"allow\"").unwrap(),
      Some(PlaceApproval::Yolo)
    );
    assert!(parse("approval = \"sometimes\"").is_err());
    for a in [PlaceApproval::Ask, PlaceApproval::Auto, PlaceApproval::Yolo] {
      assert_eq!(PlaceApproval::parse(a.as_str()), Some(a));
    }
    let eff = |a: PlaceApproval, node: &str| a.effective(node, "sandbox");
    // Looser than the node.
    assert_eq!(eff(PlaceApproval::Yolo, "sandbox"), "allow");
    assert_eq!(eff(PlaceApproval::Auto, "ask"), crate::rules::AUTO);
    // Stricter: the node's own ask, else its default one.
    assert_eq!(eff(PlaceApproval::Ask, "allow"), "sandbox");
    assert_eq!(eff(PlaceApproval::Ask, crate::rules::AUTO), "sandbox");
    assert_eq!(eff(PlaceApproval::Ask, "ask"), "ask");
    assert_eq!(eff(PlaceApproval::Auto, "allow"), crate::rules::AUTO);
    // A shell the node turned off stays off.
    assert_eq!(eff(PlaceApproval::Yolo, "off"), "off");
  }

  #[test]
  fn deleting_a_places_settings_publishes_none() {
    let dir = tempfile::tempdir().unwrap();
    let home = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      stitch_store::Role::Home,
    )
    .unwrap();
    let index = std::sync::Mutex::new(Index::open_in_memory().unwrap());
    let node: NodeName = "mac".parse().unwrap();
    let place = dir.path().join("Mail");
    std::fs::create_dir_all(place.join(".stitch")).unwrap();
    std::fs::write(place.join(".stitch/settings.toml"), "enable = [\"mail\"]\n").unwrap();
    let count = || {
      index
        .lock()
        .unwrap()
        .records(Some(kinds::PLACE))
        .unwrap()
        .len()
    };
    let files = of(Some(&home), &index, &node, "folder:~/Mail", Some(&place));
    assert_eq!(files.tool_entries(), ["mail"]);
    assert_eq!(count(), 1);
    // Records are ordered by when they were made, to the millisecond:
    // written apart, so "the latest" is never a tie.
    let apart = || std::thread::sleep(std::time::Duration::from_millis(5));
    apart();
    // `.stitch` deleted: the place has no settings here, and says so.
    std::fs::remove_dir_all(place.join(".stitch")).unwrap();
    let files = of(Some(&home), &index, &node, "folder:~/Mail", Some(&place));
    assert!(files.is_empty(), "{files:?}");
    assert_eq!(count(), 2);
    apart();
    of(Some(&home), &index, &node, "folder:~/Mail", Some(&place));
    assert_eq!(count(), 2, "none is published once");
    // A node without the directory goes by the latest record: none.
    let elsewhere = of(None, &index, &node, "folder:~/Mail", None);
    assert!(elsewhere.is_empty(), "{elsewhere:?}");
  }
}
