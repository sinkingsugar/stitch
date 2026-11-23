//! Otter tools: compiling, caching, offering and calling them
//! (`crate::otter`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime};

use otter_core::fs_access::FsAccess;
use otter_core::signature::Effects;
use otter_core::{CancelToken, Catalog};
use otter_lang::{CallOptions, FsFiles, MemoryFiles, Source, Tool, ToolError, ToolInput};
use serde_json::{Value, json};
use stitch_providers::ToolDef;
use stitch_records::RecordId;

use super::calls::{Dispatch, ToolCalls};
use super::{Origin, OtterSettings, Script};
use crate::tools::{ApprovalRequest, Approver, BoxFuture, ToolOutcome};

/// The longest description given to the model, as for MCP tools.
const MAX_DESCRIPTION: usize = 4_000;

/// What scripts compile against: the `otter` command's catalog (built
/// with its `sqlite` feature): core, I/O, text and `Db.Query`, and
/// Stitch's own `Tool.Call` (`super::calls`).
static CATALOG: LazyLock<Result<Catalog, String>> = LazyLock::new(|| {
  Catalog::new(&[
    otter_core::shards::CATALOG,
    otter_io::CATALOG,
    otter_text::CATALOG,
    otter_sqlite::CATALOG,
    super::calls::CATALOG,
  ])
});

/// The same, with a `Tool.Call` of no effects: what `otter_run`'s approval
/// compiles against, since each call is approved as its tool is.
static APPROVAL_CATALOG: LazyLock<Result<Catalog, String>> = LazyLock::new(|| {
  Catalog::new(&[
    otter_core::shards::CATALOG,
    otter_io::CATALOG,
    otter_text::CATALOG,
    otter_sqlite::CATALOG,
    super::calls::APPROVAL_CATALOG,
  ])
});

/// Otter's I/O shards but its network (`Http.*`): what an app's thread's
/// scripts compile against (design §2.22), so a script cannot reach past
/// the web's filter.
static IO_NO_NETWORK: LazyLock<Vec<&'static otter_core::ShardType>> = LazyLock::new(|| {
  otter_io::CATALOG
    .iter()
    .copied()
    .filter(|t| !t.name().starts_with("Http."))
    .collect()
});

/// The catalog of an app's thread's scripts: no network.
static FAMILY_CATALOG: LazyLock<Result<Catalog, String>> = LazyLock::new(|| {
  Catalog::new(&[
    otter_core::shards::CATALOG,
    &IO_NO_NETWORK[..],
    otter_text::CATALOG,
    otter_sqlite::CATALOG,
    super::calls::CATALOG,
  ])
});

/// The catalog, or why it could not be built.
pub(super) fn catalog() -> Result<&'static Catalog, String> {
  CATALOG
    .as_ref()
    .map_err(|e| format!("the Otter catalog: {e}"))
}

/// The catalog `otter_run`'s approval compiles against.
pub(super) fn approval_catalog() -> Result<&'static Catalog, String> {
  APPROVAL_CATALOG
    .as_ref()
    .map_err(|e| format!("the Otter catalog: {e}"))
}

/// A file as it was when read: its modification time and length.
type Stamp = (PathBuf, Option<SystemTime>, u64);

fn stamp(path: &Path) -> Stamp {
  let meta = std::fs::metadata(path).ok();
  (
    path.to_owned(),
    meta.as_ref().and_then(|m| m.modified().ok()),
    meta.map(|m| m.len()).unwrap_or(0),
  )
}

/// A script as compiled, or why it is not offered (`None`: a library,
/// which defines no tool).
type Compiled = Result<Tool, Option<String>>;

struct Entry {
  /// What it read (none for a built-in: it never changes).
  stamps: Vec<Stamp>,
  tool: Compiled,
}

/// The node's compiled scripts, by where they are.
#[derive(Default)]
pub(crate) struct Scripts {
  compiled: Mutex<HashMap<String, Entry>>,
}

impl Scripts {
  /// The script compiled, again if what it read changed; `family`: for an
  /// app's thread (no network, includes kept to its skill's directory).
  fn tool(&self, script: &Script, family: bool) -> Compiled {
    let key = match &script.origin {
      Origin::File(p) => p.display().to_string(),
      Origin::BuiltIn { skill, path } => format!("{}:{skill}/{path}", crate::apps::BUILT_IN),
    };
    let key = if family { format!("app:{key}") } else { key };
    if let Some(e) = self.compiled.lock().expect("scripts lock").get(&key)
      && e.stamps.iter().all(|s| stamp(&s.0) == *s)
    {
      return e.tool.clone();
    }
    let (stamps, tool) = compile(&script.origin, family);
    if let Err(Some(e)) = &tool {
      eprintln!("otter: {key} is not offered:\n{e}");
    }
    self.compiled.lock().expect("scripts lock").insert(
      key,
      Entry {
        stamps,
        tool: tool.clone(),
      },
    );
    tool
  }
}

/// Loads a tool; a script with no top-level code is a library (`None`).
fn load(source: Source, files: &dyn otter_lang::Files, family: bool) -> Compiled {
  let catalog = if family {
    FAMILY_CATALOG
      .as_ref()
      .map_err(|e| Some(format!("the Otter catalog: {e}")))?
  } else {
    catalog().map_err(Some)?
  };
  Tool::load_with(source, catalog, &HashMap::new(), files).map_err(|e| match e {
    // Otter says so in words only.
    ToolError::Definition(m) if m.starts_with("the script has no top-level code") => None,
    e => Some(e.to_string()),
  })
}

fn compile(origin: &Origin, family: bool) -> (Vec<Stamp>, Compiled) {
  match origin {
    Origin::File(path) => {
      // Its `tools/` directory and every file in it: a script there that
      // does not compile yet may include one being written.
      let mut stamps = vec![stamp(path)];
      stamps.extend(path.parent().map(stamp));
      if let Some(Ok(entries)) = path.parent().map(std::fs::read_dir) {
        let mut others: Vec<PathBuf> = entries
          .flatten()
          .map(|e| e.path())
          .filter(|p| p != path)
          .collect();
        others.sort();
        stamps.extend(others.iter().map(|p| stamp(p)));
      }
      let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return (stamps, Err(Some(format!("{}: {e}", path.display())))),
      };
      // An app's thread's: `@include` and `@read` within its skill's
      // directory (`<skill>/tools/<script>`), never a file of the user's.
      let tool = if family {
        let skill = path.parent().and_then(Path::parent).unwrap_or(path);
        load(
          Source::new(path.display().to_string(), text),
          &super::language::Confined::new(skill),
          true,
        )
      } else {
        load(
          Source::new(path.display().to_string(), text),
          &FsFiles::default(),
          false,
        )
      };
      if let Ok(t) = &tool {
        for f in t.files() {
          let s = stamp(Path::new(f));
          if !stamps.iter().any(|o| o.0 == s.0) {
            stamps.push(s);
          }
        }
      }
      (stamps, tool)
    }
    Origin::BuiltIn { skill, path } => {
      let found = crate::apps::SKILLS
        .iter()
        .find(|b| b.name == *skill)
        .and_then(|b| Some((b, b.files.iter().find(|(p, _)| p == path)?.1)));
      let Some((b, text)) = found else {
        return (Vec::new(), Err(Some(format!("no built-in {skill}/{path}"))));
      };
      let mut files = MemoryFiles::new();
      for (p, t) in b.files {
        files.insert(p, t.as_bytes());
      }
      (Vec::new(), load(Source::new(*path, text), &files, family))
    }
  }
}

/// A script's code checked against a capability profile's shard rule
/// (`language::load_held`): `Err` with its diagnostics when it uses a shard
/// the rule denies.
fn held_check(
  origin: &Origin,
  family: bool,
  rule: &crate::profile::ShardRule,
) -> Result<(), String> {
  let base = if family {
    FAMILY_CATALOG
      .as_ref()
      .map_err(|e| format!("the Otter catalog: {e}"))?
  } else {
    catalog()?
  };
  let loaded = match origin {
    Origin::File(path) => {
      let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
      let source = Source::new(path.display().to_string(), text);
      if family {
        let skill = path.parent().and_then(Path::parent).unwrap_or(path);
        super::language::load_held(
          source,
          base,
          Some(rule),
          &super::language::Confined::new(skill),
        )
      } else {
        super::language::load_held(source, base, Some(rule), &FsFiles::default())
      }
    }
    Origin::BuiltIn { skill, path } => {
      let found = crate::apps::SKILLS
        .iter()
        .find(|b| b.name == *skill)
        .and_then(|b| Some((b, b.files.iter().find(|(p, _)| p == path)?.1)));
      let Some((b, text)) = found else {
        return Err(format!("no built-in {skill}/{path}"));
      };
      let mut files = MemoryFiles::new();
      for (p, t) in b.files {
        files.insert(p, t.as_bytes());
      }
      super::language::load_held(Source::new(*path, text), base, Some(rule), &files)
    }
  };
  match loaded {
    Ok(_) => Ok(()),
    Err((source, ds)) => Err(
      ds.iter()
        .map(|d| otter_lang::render(d, &source))
        .collect::<String>()
        .trim_end()
        .to_owned(),
    ),
  }
}

/// Whether a script's tool name can be offered: what providers take, and
/// not an MCP tool's or a built-in's.
fn valid_name(name: &str) -> bool {
  !name.is_empty()
    && name.len() <= 64
    && !name.contains("__")
    && name
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    && crate::toolset::group_of(name).is_none()
    && !matches!(name, "read_skill" | "queue_call")
    && !super::language::NAMES.contains(&name)
}

/// Whether `[otter] allow` or the place's grants (`granted`) let `tool`
/// run without asking.
fn allowed(settings: &OtterSettings, granted: &[String], tool: &str) -> bool {
  settings
    .allow
    .iter()
    .chain(granted)
    .any(|p| crate::mcp::glob(p, tool))
}

/// Whether `[otter] ask` makes a tool that need not ask ask.
fn asks(settings: &OtterSettings, tool: &str) -> bool {
  settings.ask.iter().any(|p| crate::mcp::glob(p, tool))
}

#[derive(Clone)]
struct Listed {
  def: ToolDef,
  skill: String,
  tool: Tool,
  /// From one of Stitch's own skills (compiled into the binary).
  built_in: bool,
  /// Its skill's tools are lazy: not listed while `otter_run` is there.
  lazy: bool,
  /// Its skill's SKILL.md, for a skill whose files are here.
  skill_file: Option<PathBuf>,
  /// Where it was compiled from, and for an app's thread: checked again
  /// against a capability profile's shard rule when one holds the turn.
  origin: Origin,
  family: bool,
}

impl Listed {
  /// One of the built-in `edge-talk` skill's tools: it reads edge talk's
  /// database, and only it may.
  fn edge_talk(&self) -> bool {
    self.built_in && self.skill == super::edge_talk::SKILL
  }
}

/// What a call gets beyond its arguments: its `env`, and files it may read
/// outside its directory.
#[derive(Default)]
pub(crate) struct Extra {
  env: Vec<(&'static str, String)>,
  read: Vec<PathBuf>,
}

/// The Otter tools of a loop: its skills' scripts, and the language's own
/// (`otter_search`, `otter_describe`, `otter_check`, `otter_run`) when
/// the `otter` skill is on.
#[derive(Default)]
pub struct OtterTools {
  listed: Vec<Listed>,
  /// The language's own tools are offered.
  pub language: bool,
  settings: OtterSettings,
  /// Every call runs without asking (yolo).
  pub yolo: bool,
  /// The thread is read-only: calls that would ask are refused, and files
  /// are only read.
  pub read_only: bool,
  pub live: crate::rules::Live,
  pub approver: Option<Approver>,
  /// The thread's directory: the tools' `working-dir`, which their files
  /// are confined to.
  pub cwd: PathBuf,
  /// Where long results are saved.
  pub scratch: PathBuf,
  /// Edge talk's database on this node, which the `edge-talk` skill's
  /// tools (and only they) read.
  pub edge_talk: Option<PathBuf>,
  /// The loop's capability profiles (docs/events.md): its code compiles
  /// without the shards they deny.
  pub ceiling: Option<crate::profile::Ceiling>,
}

impl OtterTools {
  /// The tools of `scripts`, compiled (or taken compiled) by `compiled`.
  #[cfg(test)]
  pub(crate) fn new(compiled: &Scripts, scripts: &[Script], settings: OtterSettings) -> OtterTools {
    Self::new_for(compiled, scripts, settings, false)
  }

  /// `new`; `family`: for an app's thread (design §2.22).
  pub(crate) fn new_for(
    compiled: &Scripts,
    scripts: &[Script],
    settings: OtterSettings,
    family: bool,
  ) -> OtterTools {
    let mut listed: Vec<Listed> = Vec::new();
    for s in scripts {
      let Ok(tool) = compiled.tool(s, family) else {
        continue;
      };
      let d = tool.definition();
      if !valid_name(&d.name) {
        eprintln!(
          "otter: skill {}'s tool {:?} is not offered: a name is [A-Za-z0-9_-], at most 64, \
           without `__`, and no built-in tool's",
          s.skill, d.name
        );
        continue;
      }
      if let Some(other) = listed.iter().find(|l| l.def.name == d.name) {
        eprintln!(
          "otter: skill {}'s tool {} is not offered: skill {} has one of that name",
          s.skill, d.name, other.skill
        );
        continue;
      }
      // Otter wrote it from a table: an object.
      let mut schema: Value = serde_json::from_str(&d.parameters).unwrap_or_else(|_| json!({}));
      match &mut schema {
        Value::Object(m) => {
          m.entry("type").or_insert_with(|| json!("object"));
        }
        _ => schema = json!({"type": "object"}),
      }
      let description = if d.description.chars().count() > MAX_DESCRIPTION {
        d.description.chars().take(MAX_DESCRIPTION).collect()
      } else {
        d.description.clone()
      };
      listed.push(Listed {
        def: ToolDef {
          name: d.name.clone(),
          description,
          input_schema: schema,
        },
        skill: s.skill.clone(),
        tool,
        built_in: matches!(s.origin, Origin::BuiltIn { .. }),
        lazy: s.lazy,
        skill_file: match &s.origin {
          Origin::File(p) => p
            .parent()
            .and_then(Path::parent)
            .map(|d| d.join("SKILL.md")),
          Origin::BuiltIn { .. } => None,
        },
        origin: s.origin.clone(),
        family,
      });
    }
    // By name, so the list (and the provider's prompt cache) changes only
    // when the tools do.
    listed.sort_by(|a, b| a.def.name.cmp(&b.def.name));
    OtterTools {
      listed,
      settings,
      ..Default::default()
    }
  }

  /// Whether `l` is left out of the tool list: lazy, and `otter_run`
  /// there to call it (without the `otter` skill, lazy tools are listed).
  fn hidden(&self, l: &Listed) -> bool {
    l.lazy && self.language
  }

  pub fn definitions(&self) -> Vec<ToolDef> {
    let mut out: Vec<ToolDef> = self
      .listed
      .iter()
      .filter(|l| !self.hidden(l))
      .map(|l| l.def.clone())
      .collect();
    if self.language {
      out.extend(super::language::definitions());
      out.sort_by(|a, b| a.name.cmp(&b.name));
    }
    out
  }

  /// The lazy tools of `skill` (not in the tool list), by name.
  pub fn lazy_of(&self, skill: &str) -> Vec<&ToolDef> {
    self
      .listed
      .iter()
      .filter(|l| l.skill == skill && self.hidden(l))
      .map(|l| &l.def)
      .collect()
  }

  /// The skills with lazy tools here, and each one's SKILL.md when its
  /// files are here.
  pub fn lazy_skills(&self) -> Vec<(String, Option<PathBuf>)> {
    let mut out: Vec<(String, Option<PathBuf>)> = Vec::new();
    for l in self.listed.iter().filter(|l| self.hidden(l)) {
      if !out.iter().any(|(k, _)| *k == l.skill) {
        out.push((l.skill.clone(), l.skill_file.clone()));
      }
    }
    out
  }

  /// What `read_skill` adds to a skill with lazy tools: how to call them,
  /// and each one's description and parameters.
  pub fn lazy_section(&self, skill: &str) -> Option<String> {
    let defs = self.lazy_of(skill);
    let first = defs.first()?;
    let mut out = format!(
      "\n\n[Its tools are lazy: not in your tool list. Call them with `Tool.Call` in \
       `otter_run`, the arguments as a table, e.g. `{{...}} | Tool.Call(\"{}\")`: one call, or \
       several in one run. Each is approved as if called directly (otter_run itself does not \
       ask for them); the otter skill says more.]",
      first.name
    );
    for d in defs {
      out.push_str(&format!(
        "\n\n## {}\n{}\nParameters (JSON Schema): {}",
        d.name,
        d.description.trim(),
        d.input_schema
      ));
    }
    Some(out)
  }

  /// A listed tool's effects, and whether it is one of edge talk's.
  pub(crate) fn effects_of(&self, name: &str) -> Option<(Effects, bool)> {
    self
      .listed
      .iter()
      .find(|l| l.def.name == name)
      .map(|l| (l.tool.effects(), l.edge_talk()))
  }

  pub fn has(&self, name: &str) -> bool {
    (self.language && super::language::NAMES.contains(&name))
      || self.listed.iter().any(|l| l.def.name == name)
  }

  /// Whether a call with `effects` may run: `Ok(true)` when the user just
  /// approved it, `Ok(false)` when it needs no approval (no effects, yolo,
  /// `[otter] allow` or the place's grants, or `unasked`, a tool that runs
  /// without asking by default, unless `[otter] ask` names it), else why
  /// it may not.
  async fn approve(
    &self,
    thread: &RecordId,
    name: &str,
    session: String,
    command: String,
    effects: Effects,
    unasked: bool,
  ) -> Result<bool, String> {
    let granted = self.live.get();
    let yolo = (self.yolo || granted.yolo) && !self.read_only && !granted.read_only;
    if effects == Effects::NONE
      || yolo
      || allowed(&self.settings, &granted.otter, name)
      || (unasked && !asks(&self.settings, name))
    {
      return Ok(false);
    }
    if self.read_only || granted.read_only {
      return Err(format!(
        "this thread is read-only: {name} is not among the Otter tools `[otter] allow` lets run, \
         so it is refused, never asked"
      ));
    }
    let Some(approver) = &self.approver else {
      return Err("this call needs the user's approval, and nobody is here to give it".into());
    };
    let (live, settings, tool) = (self.live.clone(), self.settings.clone(), name.to_owned());
    let asking = approver(ApprovalRequest {
      grant: vec![name.to_owned()],
      grant_otter: true,
      browser: None,
      allowed_now: Some(Arc::new(move || {
        let g = live.get();
        g.yolo || allowed(&settings, &g.otter, &tool)
      })),
      ..ApprovalRequest::new(thread.clone(), session, command)
    });
    // A script's call: kept as waiting, so a run out of time says why.
    let decision = super::calls::awaiting_approval(name, asking).await;
    if !decision.allow {
      return Err(match decision.reason {
        Some(r) => format!("the user declined this call: {r}"),
        None => "the user declined this call".into(),
      });
    }
    Ok(true)
  }

  /// The language's own tools; `otter_run`'s `Tool.Call`s go to
  /// `dispatch` (none: there are no tools to call).
  async fn language_call(
    &self,
    thread: &RecordId,
    name: &str,
    input: &Value,
    dispatch: Option<&Dispatch<'_>>,
  ) -> ToolOutcome {
    use super::language;
    let outcome = |r: Result<String, String>| match r {
      Ok(t) => ToolOutcome::ok(crate::mcp::cut(t, &self.scratch)),
      Err(t) => ToolOutcome::err(crate::mcp::cut(t, &self.scratch)),
    };
    let req = match name {
      "otter_search" => return outcome(language::search(input)),
      "otter_describe" => return outcome(language::describe(input)),
      _ => match language::Request::parse(input, &self.cwd) {
        Ok(mut r) => {
          r.shards = self.ceiling.as_ref().and_then(|c| c.shards());
          Arc::new(r)
        }
        Err(e) => return ToolOutcome::err(e),
      },
    };
    let cwd = self.cwd.clone();
    if name == "otter_check" {
      let checked = tokio::task::spawn_blocking(move || language::check(&req, &cwd)).await;
      return outcome(checked.unwrap_or_else(|e| Err(format!("the check's thread failed: {e}"))));
    }
    // otter_run: checked first; it asks as its effects say.
    let (r, c) = (req.clone(), cwd.clone());
    let effects = match tokio::task::spawn_blocking(move || language::effects(&r, &c)).await {
      Ok(Ok(e)) => e,
      Ok(Err(why)) => return outcome(Err(why)),
      Err(e) => return ToolOutcome::err(format!("the check's thread failed: {e}")),
    };
    let approved = match self
      .approve(thread, name, "otter".into(), req.command(), effects, false)
      .await
    {
      Ok(a) => a,
      Err(why) => return ToolOutcome::err(why),
    };
    let cancel = CancelToken::new();
    let _guard = CancelOnDrop(cancel.clone());
    let (read_only, timeout) = (
      self.read_only || self.live.get().read_only,
      self.settings.timeout(),
    );
    // Its `Tool.Call`s come back here, served while it runs.
    let (calls, served) = match dispatch {
      Some(_) => {
        let (c, rx) = ToolCalls::new();
        (Some(c), Some(rx))
      }
      None => (None, None),
    };
    let running = tokio::task::spawn_blocking(move || {
      language::run(&req, &cwd, read_only, timeout, &cancel, calls)
    });
    let waiting = super::calls::Waiting::default();
    let mut ran = match (dispatch, served) {
      (Some(d), Some(rx)) => super::calls::serve(running, rx, d, &waiting).await,
      _ => running.await,
    }
    .unwrap_or_else(|e| Err(format!("the run's thread failed: {e}")));
    // Out of time while calls waited on the user: say so, not only the
    // time limit (a detached thread's question waits for its user).
    if let Err(why) = &mut ran
      && why.contains("otter_run timed out after")
      && let Some(more) = super::calls::still_waiting(&waiting)
    {
      why.push_str("\n\n");
      why.push_str(&more);
    }
    let mut out = outcome(ran.map(|t| {
      if t.is_empty() {
        "(done, nothing returned)".into()
      } else {
        t
      }
    }));
    if approved {
      out.output = format!("(approved by the user)\n{}", out.output);
    }
    out
  }

  pub fn call<'a>(
    &'a self,
    thread: &'a RecordId,
    name: &'a str,
    input: &'a Value,
  ) -> BoxFuture<'a, ToolOutcome> {
    self.call_with(thread, name, input, None::<&Dispatch<'a>>)
  }

  /// `call`, with the thread's tools for `otter_run`'s `Tool.Call`s.
  pub(crate) fn call_with<'a, 'd: 'a>(
    &'a self,
    thread: &'a RecordId,
    name: &'a str,
    input: &'a Value,
    dispatch: Option<&'a Dispatch<'d>>,
  ) -> BoxFuture<'a, ToolOutcome> {
    Box::pin(async move {
      if self.language && super::language::NAMES.contains(&name) {
        return self.language_call(thread, name, input, dispatch).await;
      }
      let Some(l) = self.listed.iter().find(|l| l.def.name == name) else {
        return ToolOutcome::err(format!("no tool {name}"));
      };
      // Edge talk's tools read its database, given by path in their
      // `env` and granted to them alone; they run without asking.
      let mut extra = Extra::default();
      if l.edge_talk() {
        let Some(db) = &self.edge_talk else {
          return ToolOutcome::err("edge talk's database is not on this node");
        };
        extra
          .env
          .push((super::edge_talk::ENV_KEY, db.display().to_string()));
        extra.read.push(db.clone());
      }
      // Held by a profile: its code checked against the shards it allows,
      // before anything of it runs.
      if let Some(rule) = self.ceiling.as_ref().and_then(|c| c.shards()) {
        let (origin, family) = (l.origin.clone(), l.family);
        let held = tokio::task::spawn_blocking(move || held_check(&origin, family, &rule))
          .await
          .unwrap_or_else(|e| Err(format!("the check's thread failed: {e}")));
        if let Err(why) = held {
          return ToolOutcome::err(format!("{name} is refused in this turn:\n{why}"));
        }
      }
      let approved = match self
        .approve(
          thread,
          name,
          format!("otter:{}", l.skill),
          format!("{name} {}", crate::mcp::one_line_json(input)),
          l.tool.effects(),
          l.edge_talk(),
        )
        .await
      {
        Ok(a) => a,
        Err(why) => return ToolOutcome::err(why),
      };
      let mut out = run(
        l.tool.clone(),
        input,
        &self.cwd,
        self.read_only || self.live.get().read_only,
        self.settings.timeout(),
        &self.scratch,
        extra,
      )
      .await;
      if approved {
        out.output = format!("(approved by the user)\n{}", out.output);
      }
      out
    })
  }
}

/// Cancels the token when dropped: the call's future went away
/// (interrupted, the thread cancelled), so the script stops at once.
struct CancelOnDrop(CancelToken);

impl Drop for CancelOnDrop {
  fn drop(&mut self) {
    self.0.cancel();
  }
}

/// Calls `tool` with the model's arguments, on a blocking thread, its
/// files confined to `cwd`.
pub(crate) async fn run(
  tool: Tool,
  input: &Value,
  cwd: &Path,
  read_only: bool,
  timeout: Duration,
  scratch: &Path,
  extra: Extra,
) -> ToolOutcome {
  let arguments = if input.is_null() {
    String::new()
  } else {
    input.to_string()
  };
  let call = match ToolInput::from_json(&arguments) {
    Ok(c) => c.with_working_dir(cwd.display().to_string()),
    Err(e) => return ToolOutcome::err(e.to_string()),
  };
  let call = if extra.env.is_empty() {
    call
  } else {
    call.with_env(otter_core::Var::table(
      extra
        .env
        .iter()
        .map(|(k, v)| (*k, otter_core::Var::string(v.as_str())))
        .collect::<Vec<_>>(),
    ))
  };
  let mut services = otter_core::service::Services::default();
  let mut access = FsAccess::within(cwd).unwrap_or_else(|_| FsAccess::denied());
  for path in &extra.read {
    access = match access.grant_read(path) {
      Ok(a) => a,
      Err(e) => return ToolOutcome::err(format!("{}: {e}", path.display())),
    };
  }
  services.insert(if read_only {
    access.read_only()
  } else {
    access
  });
  let cancel = CancelToken::new();
  let _guard = CancelOnDrop(cancel.clone());
  let options = CallOptions {
    services,
    cancel: Some(cancel),
    timeout: Some(timeout),
    ..Default::default()
  };
  let name = tool.definition().name.clone();
  let result = tokio::task::spawn_blocking(move || tool.call_with(&call, &options)).await;
  let logged = |log: &[String]| {
    if log.is_empty() {
      String::new()
    } else {
      format!("\n\nIt logged:\n{}", log.join("\n"))
    }
  };
  let (text, ok) = match result {
    Ok(Ok(out)) => {
      // The model gets the result; the node's log, what it logged.
      for line in &out.log {
        eprintln!("otter: {name}: {line}");
      }
      (out.text, true)
    }
    Ok(Err(e)) => {
      let log = match &e {
        ToolError::Failed { log, .. }
        | ToolError::Stopped { log }
        | ToolError::Cancelled { log }
        | ToolError::TimedOut { log, .. } => logged(log),
        _ => String::new(),
      };
      (format!("{e}{log}"), false)
    }
    Err(e) => (format!("the tool's thread failed: {e}"), false),
  };
  let mut text = crate::mcp::cut(text, scratch);
  if text.is_empty() {
    text = if ok {
      "(done, nothing returned)"
    } else {
      "(failed, saying nothing)"
    }
    .into();
  }
  if ok {
    ToolOutcome::ok(text)
  } else {
    ToolOutcome::err(text)
  }
}

#[cfg(test)]
mod tests {
  use std::sync::Mutex as StdMutex;

  use super::*;
  use crate::otter::scripts_in;
  use crate::rules::{Granted, Live};
  use crate::tools::Decision;

  /// Pure: it only computes, so it never asks.
  const HI: &str = r#"
@fn(Hi input: {input: {} env: {} working-dir: String} output: String params: {} {
  input.input.name | If({IsString} {ExpectString} {"nobody"}) = name
  f"Hi, {name}!"
})
{definition: {name: "hi" description: "Greets."} use: "Hi"}
"#;

  /// It logs, waits and writes: it asks.
  const WORK: &str = r#"
@fn(Work input: {input: {} env: {} working-dir: String} output: String params: {} {
  input.input = args
  input.working-dir = dir
  args.fail | When({Is(true)} {"about to fail" | Log  "asked to fail" | Fail})
  args.wait | When({Is(true)} {"waiting" | Log  Repeat({Pause} forever: true)})
  args.write | When({Is(true)} {[dir "out.txt"] | FS.Join | FS.Write(contents: "x")})
  args.later | When({Is(true)} {Pause(0.5)  [dir "late.txt"] | FS.Join | FS.Write(contents: "x")})
  f"worked in {dir}"
})
{
  definition: {
    name: "work"
    description: "Works."
    parameters: {type: "object" properties: {fail: {type: "boolean"} wait: {type: "boolean"}}}
  }
  use: "Work"
}
"#;

  #[tokio::test]
  async fn the_languages_tools_come_with_the_skill_and_ask_by_effects() {
    let mut s = setup(Default::default());
    // Off: neither listed nor callable.
    assert!(!s.tools.has("otter_run"));
    s.tools.language = true;
    let names: Vec<String> = s.tools.definitions().into_iter().map(|d| d.name).collect();
    assert_eq!(
      names,
      [
        "hi",
        "otter_check",
        "otter_describe",
        "otter_run",
        "otter_search",
        "work"
      ]
    );
    // Search, describe and check never ask.
    let out = s
      .tools
      .call(&thread(), "otter_search", &json!({"query": "Count"}))
      .await;
    assert!(
      out.ok && out.output.starts_with("Count ("),
      "{}",
      out.output
    );
    let out = s
      .tools
      .call(&thread(), "otter_describe", &json!({"name": "Nope"}))
      .await;
    assert!(
      !out.ok && out.output.starts_with("no shard named Nope"),
      "{}",
      out.output
    );
    let out = s
      .tools
      .call(&thread(), "otter_check", &json!({"code": "\"x\" | Log"}))
      .await;
    assert!(
      out.ok && out.output.contains("effects: io"),
      "{}",
      out.output
    );
    // Code that only computes runs at once; with nobody to ask, code that
    // logs (I/O) cannot.
    let out = s
      .tools
      .call(&thread(), "otter_run", &json!({"code": "[1 2] | Count"}))
      .await;
    assert!(out.ok, "{}", out.output);
    assert_eq!(out.output, "root: 2");
    let logs = json!({"code": "\"hi\" | Log\n1"});
    let out = s.tools.call(&thread(), "otter_run", &logs).await;
    assert!(
      !out.ok && out.output.contains("nobody is here"),
      "{}",
      out.output
    );
    // Code with problems is not run, nor asked about.
    let (a, asked) = approver(Decision::allow());
    s.tools.approver = Some(a);
    let out = s
      .tools
      .call(&thread(), "otter_run", &json!({"code": "Nope | Log"}))
      .await;
    assert!(
      !out.ok && out.output.starts_with("code:1:1: "),
      "{}",
      out.output
    );
    assert!(asked.lock().unwrap().is_empty());
    // Asked, with the code, granted as an Otter tool.
    let out = s.tools.call(&thread(), "otter_run", &logs).await;
    assert!(out.ok, "{}", out.output);
    assert_eq!(out.output, "(approved by the user)\nhi\nroot: 1");
    let req = asked.lock().unwrap().remove(0);
    assert_eq!(req.session, "otter");
    assert_eq!(req.command, "otter_run \"hi\" | Log\n1");
    assert_eq!(req.grant, ["otter_run"]);
    assert!(req.grant_otter);
    let (a, _) = approver(Decision::decline(None));
    s.tools.approver = Some(a);
    let out = s.tools.call(&thread(), "otter_run", &logs).await;
    assert_eq!(out.output, "the user declined this call");
    // `[otter] allow` lets it run unasked.
    let mut s = setup(OtterSettings {
      allow: vec!["otter_run".into()],
      ..Default::default()
    });
    s.tools.language = true;
    assert_eq!(
      s.tools.call(&thread(), "otter_run", &logs).await.output,
      "hi\nroot: 1"
    );
  }

  #[tokio::test]
  async fn otter_run_in_a_read_only_thread_and_interrupted() {
    let (a, asked) = approver(Decision::allow());
    let mut s = setup(Default::default());
    s.tools.language = true;
    s.tools.approver = Some(a);
    s.tools.read_only = true;
    s.tools.yolo = true;
    let out = s
      .tools
      .call(&thread(), "otter_run", &json!({"code": "\"hi\" | Log"}))
      .await;
    assert!(
      !out.ok && out.output.contains("read-only"),
      "{}",
      out.output
    );
    assert!(asked.lock().unwrap().is_empty());
    assert!(
      s.tools
        .call(&thread(), "otter_run", &json!({"code": "1"}))
        .await
        .ok
    );
    // Dropped while it waits: it stops, and never writes.
    let mut s = setup(OtterSettings {
      allow: vec!["*".into()],
      ..Default::default()
    });
    s.tools.language = true;
    let late = s.tools.cwd.join("late.txt");
    let code =
      json!({"code": format!("Pause(0.5)\n\"{}\" | FS.Write(contents: \"x\")", late.display())});
    let t = thread();
    let call = s.tools.call(&t, "otter_run", &code);
    assert!(
      tokio::time::timeout(Duration::from_millis(100), call)
        .await
        .is_err()
    );
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert!(!late.exists());
    // Left alone, it does.
    assert!(s.tools.call(&thread(), "otter_run", &code).await.ok);
    assert!(late.is_file());
  }

  /// The thread's tools for `Tool.Call`, as `NodeTools` gives them: here,
  /// these Otter tools.
  fn dispatching<'a>(
    tools: &'a OtterTools,
    t: &'a RecordId,
  ) -> impl Fn(String, Value) -> BoxFuture<'a, ToolOutcome> + Send + Sync + 'a {
    move |n, i| Box::pin(async move { tools.call(t, &n, &i).await })
  }

  fn code(c: &str) -> Value {
    json!({ "code": c })
  }

  /// `otter_run` of `c`, its `Tool.Call`s reaching `tools`.
  async fn script(tools: &OtterTools, t: &RecordId, c: &str) -> ToolOutcome {
    let d = dispatching(tools, t);
    tools.call_with(t, "otter_run", &code(c), Some(&d)).await
  }

  #[tokio::test]
  async fn decide_asks_the_threads_decide_tool_and_its_cost_counts() {
    let mut s = setup(Default::default());
    s.tools.language = true;
    let t = thread();
    let seen: Arc<StdMutex<Vec<(String, Value)>>> = Arc::default();
    let kept = seen.clone();
    let d = move |n: String, i: Value| -> BoxFuture<'static, ToolOutcome> {
      kept.lock().unwrap().push((n, i));
      Box::pin(async move {
        let mut o = ToolOutcome::ok(
          json!({"answers": {"spam": {"type": "yes_no", "p": 0.9}}, "calibrated": true})
            .to_string(),
        );
        o.spent = Some(crate::tools::Spent {
          model: "gemini/gemini-decision-1".into(),
          usage: Default::default(),
          usd: Some(0.0001),
        });
        o
      })
    };
    let c = code("\"buy now!!\" | Decide({spam: \"Is it spam?\"}) | .answers.spam.p");
    let (out, spent) =
      crate::tools::spending(s.tools.call_with(&t, "otter_run", &c, Some(&d))).await;
    assert!(out.ok, "{}", out.output);
    assert_eq!(out.output, "root: 0.9");
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen[0].0, "decide");
    assert_eq!(seen[0].1["input"], "buy now!!");
    assert_eq!(seen[0].1["questions"]["spam"], "Is it spam?");
    assert_eq!(spent.len(), 1, "priced to the thread with otter_run's call");
    // Without otter_run's tools (a skill's tool): no decisions.
    let out = s
      .tools
      .call(&t, "otter_run", &code("\"x\" | Decide({a: \"?\"})"))
      .await;
    assert!(
      !out.ok && out.output.contains("no decisions here"),
      "{}",
      out.output
    );
  }

  #[tokio::test]
  async fn a_script_out_of_time_on_a_pending_approval_says_so() {
    // Nobody answers (a detached thread whose user is away).
    let asked = Arc::new(StdMutex::new(Vec::new()));
    let kept = asked.clone();
    let a: Approver = Arc::new(move |req| {
      kept.lock().unwrap().push(req);
      Box::pin(std::future::pending())
    });
    // otter_run itself allowed: only the script's call asks.
    let mut s = setup(OtterSettings {
      timeout_secs: Some(1),
      allow: vec!["otter_run".into()],
      ..Default::default()
    });
    s.tools.language = true;
    s.tools.approver = Some(a);
    let t = thread();
    let out = script(&s.tools, &t, "{} | Tool.Call(\"work\")").await;
    assert!(!out.ok, "{}", out.output);
    assert_eq!(asked.lock().unwrap().len(), 1);
    assert!(
      out.output.contains("otter_run timed out after 1s")
        && out
          .output
          .contains("work was still waiting for the user's approval"),
      "{}",
      out.output
    );
    assert!(out.output.contains("Call work directly"), "{}", out.output);
    // A run out of time whose call was answered says nothing of one.
    let (a, asked) = approver(Decision::allow());
    s.tools.approver = Some(a);
    let out = script(
      &s.tools,
      &t,
      "{} | Tool.Call(\"work\") = w\nRepeat({Pause} forever: true)",
    )
    .await;
    assert_eq!(asked.lock().unwrap().len(), 1);
    assert!(!out.ok, "{}", out.output);
    assert!(out.output.contains("timed out"), "{}", out.output);
    assert!(!out.output.contains("approval"), "{}", out.output);
  }

  #[tokio::test]
  async fn tool_call_reaches_the_threads_tools_each_approved_as_its_own() {
    let (a, asked) = approver(Decision::allow());
    let mut s = setup(Default::default());
    s.tools.language = true;
    s.tools.approver = Some(a);
    let t = thread();
    // A tool that only computes: nothing is asked, otter_run included.
    let out = script(&s.tools, &t, "{name: \"Ann\"} | Tool.Call(\"hi\")").await;
    assert_eq!(out.output, "root: \"Hi, Ann!\"");
    assert!(out.ok);
    // No arguments; the name from a variable.
    assert_eq!(
      script(&s.tools, &t, "Tool.Call(\"hi\")").await.output,
      "root: \"Hi, nobody!\""
    );
    assert_eq!(
      script(
        &s.tools,
        &t,
        "\"hi\" = which\n{name: \"Bo\"} | Tool.Call(which)"
      )
      .await
      .output,
      "root: \"Hi, Bo!\""
    );
    // Several calls, their results combined.
    let out = script(
      &s.tools,
      &t,
      "{name: \"A\"} | Tool.Call(\"hi\") = a\n{name: \"B\"} | Tool.Call(\"hi\") = b\nf\"{a} {b}\"",
    )
    .await;
    assert_eq!(out.output, "root: \"Hi, A! Hi, B!\"");
    assert!(asked.lock().unwrap().is_empty());
    // A tool that asks asks, once, saying it comes from a script; the
    // script that calls it does not ask.
    let out = script(&s.tools, &t, "{write: true} | Tool.Call(\"work\")").await;
    assert!(out.ok, "{}", out.output);
    assert!(
      out.output.starts_with("root: \"worked in "),
      "{}",
      out.output
    );
    let req = asked.lock().unwrap().remove(0);
    assert!(asked.lock().unwrap().is_empty());
    assert_eq!(req.session, "otter:demo");
    assert_eq!(
      req.command,
      format!(
        "{}work {{\"write\":true}}",
        super::super::calls::FROM_SCRIPT
      )
    );
    assert_eq!(req.grant, ["work"]);
    // A script with effects of its own asks for them, as otter_run, and
    // its calls each as their tool.
    let out = script(&s.tools, &t, "\"go\" | Log\n{} | Tool.Call(\"work\")").await;
    assert!(out.ok, "{}", out.output);
    let reqs: Vec<String> = asked.lock().unwrap().drain(..).map(|r| r.command).collect();
    assert_eq!(reqs.len(), 2, "{reqs:?}");
    assert!(reqs[0].starts_with("otter_run \"go\""), "{reqs:?}");
    assert!(
      reqs[1].starts_with(super::super::calls::FROM_SCRIPT),
      "{reqs:?}"
    );
    // A failing tool fails the shard with its message, and Maybe catches it.
    let out = script(&s.tools, &t, "{fail: true} | Tool.Call(\"work\")").await;
    assert!(
      !out.ok && out.output.contains("root: failed: work: asked to fail"),
      "{}",
      out.output
    );
    asked.lock().unwrap().clear();
    let out = script(
      &s.tools,
      &t,
      "Maybe({{fail: true} | Tool.Call(\"work\")} {\"caught\"} silent: true)",
    )
    .await;
    // Maybe may log, which is I/O in Otter's contract: otter_run asks for
    // that, as for any script with effects of its own.
    assert_eq!(out.output, "(approved by the user)\nroot: \"caught\"");
    // Refused, and unknown.
    for refused in [
      "otter_run",
      "spawn",
      "send",
      "wait",
      "cancel",
      "wake_at",
      "schedule",
    ] {
      let out = script(&s.tools, &t, &format!("Tool.Call(\"{refused}\")")).await;
      assert!(
        !out.ok && out.output.contains("cannot be called from a script"),
        "{refused}: {}",
        out.output
      );
    }
    let out = script(&s.tools, &t, "Tool.Call(\"nope\")").await;
    assert!(
      !out.ok && out.output.contains("no tool nope"),
      "{}",
      out.output
    );
    // Declined: the call fails, and the script can go on.
    // otter_run's own question (Maybe's I/O) allowed, the call's declined.
    let asked = Arc::new(StdMutex::new(Vec::new()));
    let kept = asked.clone();
    s.tools.approver = Some(Arc::new(move |req: ApprovalRequest| {
      let d = if req.command.starts_with("otter_run") {
        Decision::allow()
      } else {
        Decision::decline(None)
      };
      kept.lock().unwrap().push(req.command);
      Box::pin(async move { d })
    }));
    let out = script(
      &s.tools,
      &t,
      "Maybe({{write: true} | Tool.Call(\"work\")} {\"declined\"} silent: true)",
    )
    .await;
    assert_eq!(out.output, "(approved by the user)\nroot: \"declined\"");
    assert_eq!(asked.lock().unwrap().len(), 2);
    // Without the thread's tools (a skill's tool, a check): none here.
    let out = s
      .tools
      .call(&t, "otter_run", &code("Tool.Call(\"hi\")"))
      .await;
    assert!(
      !out.ok && out.output.contains("no tools here"),
      "{}",
      out.output
    );
    // `[otter] allow` lets a tool run from a script unasked, as directly.
    let mut s = setup(OtterSettings {
      allow: vec!["work".into()],
      ..Default::default()
    });
    s.tools.language = true;
    let out = script(&s.tools, &t, "{write: true} | Tool.Call(\"work\")").await;
    assert!(out.ok, "{}", out.output);
  }

  #[tokio::test]
  async fn tool_call_in_a_read_only_thread_and_stopped() {
    let (a, asked) = approver(Decision::allow());
    let mut s = setup(Default::default());
    s.tools.language = true;
    s.tools.approver = Some(a);
    s.tools.read_only = true;
    let t = thread();
    {
      let tools = &s.tools;
      let d = dispatching(tools, &t);
      // otter_run runs (it only computes); a tool that computes runs; one
      // with effects is refused, never asked.
      let out = tools
        .call_with(&t, "otter_run", &code("Tool.Call(\"hi\")"), Some(&d))
        .await;
      assert_eq!(out.output, "root: \"Hi, nobody!\"");
      let out = tools
        .call_with(
          &t,
          "otter_run",
          &code("{write: true} | Tool.Call(\"work\")"),
          Some(&d),
        )
        .await;
      assert!(
        !out.ok && out.output.contains("read-only"),
        "{}",
        out.output
      );
      assert!(asked.lock().unwrap().is_empty());
    }
    // Dropped while a call runs: the call stops too, and never writes.
    let mut s = setup(OtterSettings {
      allow: vec!["*".into()],
      ..Default::default()
    });
    s.tools.language = true;
    let late = s.tools.cwd.join("late.txt");
    {
      let tools = &s.tools;
      let d = dispatching(tools, &t);
      let input = code("{later: true} | Tool.Call(\"work\")");
      let call = tools.call_with(&t, "otter_run", &input, Some(&d));
      assert!(
        tokio::time::timeout(Duration::from_millis(100), call)
          .await
          .is_err()
      );
      tokio::time::sleep(Duration::from_millis(1000)).await;
      assert!(!late.exists());
      // Left alone, it does.
      assert!(tools.call_with(&t, "otter_run", &input, Some(&d)).await.ok);
      assert!(late.is_file());
    }
    // otter_run's time limit stops a call in progress.
    let mut s = setup(OtterSettings {
      allow: vec!["*".into()],
      timeout_secs: Some(1),
      ..Default::default()
    });
    s.tools.language = true;
    let tools = &s.tools;
    let d = dispatching(tools, &t);
    let started = std::time::Instant::now();
    let out = tools
      .call_with(
        &t,
        "otter_run",
        &code("{wait: true} | Tool.Call(\"work\")"),
        Some(&d),
      )
      .await;
    assert!(
      !out.ok && out.output.contains("timed out"),
      "{}",
      out.output
    );
    assert!(started.elapsed() < Duration::from_secs(5));
  }

  #[tokio::test]
  async fn lazy_tools_are_left_out_of_the_list_while_otter_run_can_call_them() {
    let dir = tempfile::tempdir().unwrap();
    let scripts: Vec<Script> = skill(dir.path(), "demo", &[("hi.otr", HI), ("work.otter", WORK)])
      .into_iter()
      .map(|s| Script { lazy: true, ..s })
      .collect();
    let mut tools = OtterTools::new(&Scripts::default(), &scripts, Default::default());
    tools.cwd = dir.path().to_owned();
    let names =
      |t: &OtterTools| -> Vec<String> { t.definitions().into_iter().map(|d| d.name).collect() };
    // Without the otter skill, nothing could call them: listed.
    assert_eq!(names(&tools), ["hi", "work"]);
    assert!(tools.lazy_skills().is_empty() && tools.lazy_section("demo").is_none());
    tools.language = true;
    assert_eq!(
      names(&tools),
      ["otter_check", "otter_describe", "otter_run", "otter_search"]
    );
    assert!(tools.has("hi"));
    assert_eq!(
      tools.lazy_skills(),
      [(
        "demo".to_owned(),
        Some(dir.path().join("skills/demo/SKILL.md"))
      )]
    );
    let section = tools.lazy_section("demo").unwrap();
    assert!(section.contains("Tool.Call(\"hi\")"), "{section}");
    assert!(
      section.contains("## work\nWorks.\nParameters (JSON Schema): {"),
      "{section}"
    );
    // Called through otter_run, as any tool.
    let t = thread();
    assert_eq!(
      script(&tools, &t, "{name: \"Lu\"} | Tool.Call(\"hi\")")
        .await
        .output,
      "root: \"Hi, Lu!\""
    );
  }

  #[test]
  fn names_that_can_be_offered() {
    assert!(valid_name("google_flights_search"));
    assert!(valid_name("a-b_C9"));
    for bad in [
      "",
      "github__get",
      "has space",
      "dot.ted",
      "shell_run",
      "browse_web",
      "read_skill",
      "queue_call",
      "otter_run",
      "otter_search",
    ] {
      assert!(!valid_name(bad), "{bad}");
    }
    assert!(!valid_name(&"x".repeat(65)));
  }

  #[test]
  fn settings_read_and_allow() {
    let s: OtterSettings = toml::from_str(
      r#"
      allow = ["google_flights_*"]
      timeout_secs = 99999
      [places."global"]
      allow = ["echo"]
      "#,
    )
    .unwrap();
    assert_eq!(s.timeout(), std::time::Duration::from_secs(3600));
    assert_eq!(
      OtterSettings::default().timeout(),
      std::time::Duration::from_secs(120)
    );
    assert!(allowed(&s, &[], "google_flights_search"));
    assert!(!allowed(&s, &[], "echo"));
    assert!(allowed(&s, &s.places["global"].allow, "echo"));
    assert!(toml::from_str::<OtterSettings>("alow = []").is_err());
  }

  fn thread() -> RecordId {
    "2026-10-16T120000Z-mbp-k3f9".parse().unwrap()
  }

  /// A home workspace's skill `name` with these tool scripts.
  fn skill(home: &Path, name: &str, files: &[(&str, &str)]) -> Vec<Script> {
    let dir = home.join("skills").join(name);
    std::fs::create_dir_all(dir.join("tools")).unwrap();
    for (f, text) in files {
      std::fs::write(dir.join("tools").join(f), text).unwrap();
    }
    scripts_in(name, &dir)
  }

  /// An approver that answers `decision` and keeps what it was asked.
  fn approver(decision: Decision) -> (Approver, Arc<StdMutex<Vec<ApprovalRequest>>>) {
    let asked = Arc::new(StdMutex::new(Vec::new()));
    let kept = asked.clone();
    let approver: Approver = Arc::new(move |req| {
      kept.lock().unwrap().push(req);
      let d = decision.clone();
      Box::pin(async move { d })
    });
    (approver, asked)
  }

  struct Setup {
    dir: tempfile::TempDir,
    tools: OtterTools,
  }

  fn setup(settings: OtterSettings) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let scripts = skill(dir.path(), "demo", &[("hi.otr", HI), ("work.otter", WORK)]);
    let mut tools = OtterTools::new(&Scripts::default(), &scripts, settings);
    tools.cwd = dir.path().join("work");
    std::fs::create_dir_all(&tools.cwd).unwrap();
    tools.scratch = dir.path().join("scratch");
    Setup { dir, tools }
  }

  #[tokio::test]
  async fn a_tool_is_defined_and_called() {
    let s = setup(Default::default());
    let defs = s.tools.definitions();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["hi", "work"]);
    assert_eq!(defs[0].description, "Greets.");
    // No parameters given: an object with none.
    assert_eq!(
      defs[0].input_schema,
      json!({"type": "object", "properties": {}})
    );
    assert_eq!(
      defs[1].input_schema["properties"]["fail"]["type"],
      "boolean"
    );
    assert!(s.tools.has("work") && !s.tools.has("nope"));

    // Only computing: no approver needed.
    let out = s.tools.call(&thread(), "hi", &json!({"name": "Ann"})).await;
    assert!(out.ok, "{}", out.output);
    assert_eq!(out.output, "Hi, Ann!");
    let out = s.tools.call(&thread(), "hi", &Value::Null).await;
    assert_eq!(out.output, "Hi, nobody!");
    let out = s.tools.call(&thread(), "hi", &json!([1])).await;
    assert!(
      !out.ok && out.output.contains("JSON object"),
      "{}",
      out.output
    );
  }

  #[tokio::test]
  async fn a_call_with_effects_asks_and_runs_once_allowed() {
    let mut s = setup(Default::default());
    // Nobody to ask.
    let out = s.tools.call(&thread(), "work", &json!({})).await;
    assert!(
      !out.ok && out.output.contains("nobody is here"),
      "{}",
      out.output
    );

    let (a, asked) = approver(Decision::allow());
    s.tools.approver = Some(a);
    let out = s
      .tools
      .call(&thread(), "work", &json!({"write": true}))
      .await;
    assert!(out.ok, "{}", out.output);
    assert_eq!(
      out.output,
      format!(
        "(approved by the user)\nworked in {}",
        s.tools.cwd.display()
      )
    );
    assert!(s.tools.cwd.join("out.txt").is_file());
    let req = asked.lock().unwrap().remove(0);
    assert_eq!(req.session, "otter:demo");
    assert_eq!(req.command, r#"work {"write":true}"#);
    assert_eq!(req.grant, ["work"]);
    assert!(req.grant_otter && req.grant_server.is_none());
    assert!(!req.allowed_now());

    let (a, _) = approver(Decision::decline(Some("not now".into())));
    s.tools.approver = Some(a);
    let out = s.tools.call(&thread(), "work", &json!({})).await;
    assert!(!out.ok);
    assert_eq!(out.output, "the user declined this call: not now");
    drop(s.dir);
  }

  #[tokio::test]
  async fn allowed_calls_do_not_ask() {
    let (a, asked) = approver(Decision::decline(None));
    // By `[otter] allow`.
    let mut s = setup(OtterSettings {
      allow: vec!["wo*".into()],
      ..Default::default()
    });
    s.tools.approver = Some(a.clone());
    assert!(s.tools.call(&thread(), "work", &json!({})).await.ok);
    // By the place's grants, read at each call.
    let mut s = setup(Default::default());
    s.tools.approver = Some(a.clone());
    let granted = Arc::new(StdMutex::new(Vec::<String>::new()));
    let g = granted.clone();
    s.tools.live = Live::new(move || Granted {
      otter: g.lock().unwrap().clone(),
      ..Default::default()
    });
    assert!(!s.tools.call(&thread(), "work", &json!({})).await.ok);
    let req = asked.lock().unwrap().pop().unwrap();
    granted.lock().unwrap().push("work".into());
    // A question waiting on it goes once it is granted.
    assert!(req.allowed_now());
    assert!(s.tools.call(&thread(), "work", &json!({})).await.ok);
    // In yolo.
    let mut s = setup(Default::default());
    s.tools.approver = Some(a);
    s.tools.yolo = true;
    assert!(s.tools.call(&thread(), "work", &json!({})).await.ok);
    assert!(asked.lock().unwrap().is_empty());
  }

  #[tokio::test]
  async fn a_read_only_thread_is_refused_and_reads_only() {
    let (a, asked) = approver(Decision::allow());
    let mut s = setup(Default::default());
    s.tools.approver = Some(a);
    s.tools.read_only = true;
    s.tools.yolo = true;
    let out = s.tools.call(&thread(), "work", &json!({})).await;
    assert!(
      !out.ok && out.output.contains("read-only"),
      "{}",
      out.output
    );
    assert!(asked.lock().unwrap().is_empty());
    // Pure tools still run.
    assert!(s.tools.call(&thread(), "hi", &json!({})).await.ok);
    // Allowed, it runs, but writes nothing.
    let mut s = setup(OtterSettings {
      allow: vec!["work".into()],
      ..Default::default()
    });
    s.tools.read_only = true;
    assert!(s.tools.call(&thread(), "work", &json!({})).await.ok);
    let out = s
      .tools
      .call(&thread(), "work", &json!({"write": true}))
      .await;
    assert!(!out.ok, "{}", out.output);
    assert!(!s.tools.cwd.join("out.txt").exists());
  }

  #[tokio::test]
  async fn failures_say_what_was_logged() {
    let mut s = setup(OtterSettings {
      allow: vec!["*".into()],
      timeout_secs: Some(1),
      ..Default::default()
    });
    let out = s
      .tools
      .call(&thread(), "work", &json!({"fail": true}))
      .await;
    assert!(!out.ok);
    assert_eq!(out.output, "asked to fail\n\nIt logged:\nabout to fail");
    let started = std::time::Instant::now();
    let out = s
      .tools
      .call(&thread(), "work", &json!({"wait": true}))
      .await;
    assert!(!out.ok);
    assert_eq!(
      out.output,
      "the tool call timed out after 1s\n\nIt logged:\nwaiting"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    // Its files stay in its directory.
    s.tools.cwd = s.dir.path().join("missing");
    let out = s
      .tools
      .call(&thread(), "work", &json!({"write": true}))
      .await;
    assert!(!out.ok, "{}", out.output);
  }

  #[tokio::test]
  async fn an_interrupted_call_is_cancelled() {
    let s = setup(OtterSettings {
      allow: vec!["*".into()],
      ..Default::default()
    });
    let late = s.tools.cwd.join("late.txt");
    // Left to finish, it writes after half a second.
    assert!(
      s.tools
        .call(&thread(), "work", &json!({"later": true}))
        .await
        .ok
    );
    assert!(late.is_file());
    std::fs::remove_file(&late).unwrap();
    // Dropped before: it never does.
    let (t, input) = (thread(), json!({"later": true}));
    let call = s.tools.call(&t, "work", &input);
    assert!(
      tokio::time::timeout(Duration::from_millis(100), call)
        .await
        .is_err()
    );
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert!(!late.exists());
  }

  #[test]
  fn scripts_that_cannot_be_offered_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let mut scripts = skill(
      dir.path(),
      "a",
      &[
        ("broken.shs", "@fn(Broken {"),
        (
          "lib.shs",
          "@fn(Twice input: String output: String params: {} { = s  [s s] | String.Join })",
        ),
        (
          "shell.shs",
          &HI.replace("name: \"hi\"", "name: \"shell_run\""),
        ),
        ("work.shs", WORK),
      ],
    );
    // A second skill's tool of the same name: the first wins.
    scripts.extend(skill(
      dir.path(),
      "b",
      &[("work.shs", WORK), ("hi.shs", HI)],
    ));
    let compiled = Scripts::default();
    let tools = OtterTools::new(&compiled, &scripts, Default::default());
    let names: Vec<String> = tools.definitions().into_iter().map(|d| d.name).collect();
    assert_eq!(names, ["hi", "work"]);
    assert_eq!(tools.listed[1].skill, "a");
    // Why each is not offered: the broken one's diagnostics; a library
    // says nothing.
    let why = |f: &str| {
      let s = scripts
        .iter()
        .find(|s| s.origin == Origin::File(dir.path().join("skills/a/tools").join(f)))
        .unwrap();
      compiled.tool(s, false).err().unwrap()
    };
    assert!(why("broken.shs").is_some());
    assert_eq!(why("lib.shs"), None);
  }

  #[test]
  fn an_apps_scripts_reach_no_network_and_read_only_their_skill() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("secret.txt"), "hunter2").unwrap();
    // A tool that embeds a file of the user's when compiled, one that
    // embeds its own skill's, and one that reaches the network.
    let embeds = |path: &str| {
      HI.replace(
        "f\"Hi, {name}!\"",
        &format!("@read(\"{path}\") = got\n  got"),
      )
    };
    let net = HI.replace(
      "f\"Hi, {name}!\"",
      "Http.Get(url: \"https://example.com\") | ToString",
    );
    let mut scripts = skill(
      dir.path(),
      "a",
      &[(
        "leak.otr",
        &embeds(&dir.path().join("secret.txt").display().to_string()),
      )],
    );
    std::fs::write(dir.path().join("skills/b/notes.txt"), "own").ok();
    scripts.extend(skill(
      dir.path(),
      "b",
      &[(
        "own.otr",
        &embeds("../notes.txt").replace("\"hi\"", "\"own\""),
      )],
    ));
    std::fs::write(dir.path().join("skills/b/notes.txt"), "own").unwrap();
    scripts.extend(skill(
      dir.path(),
      "c",
      &[("net.otr", &net.replace("\"hi\"", "\"net\""))],
    ));
    let compiled = Scripts::default();
    // The user's own threads: all three compile.
    for s in &scripts {
      assert!(compiled.tool(s, false).is_ok(), "{:?}", s.origin);
    }
    // An app's thread's: only the one reading its own skill.
    let family = OtterTools::new_for(&compiled, &scripts, Default::default(), true);
    let names: Vec<String> = family.definitions().into_iter().map(|d| d.name).collect();
    assert_eq!(names, ["own"]);
  }

  #[test]
  fn a_changed_script_is_compiled_again() {
    let dir = tempfile::tempdir().unwrap();
    let scripts = skill(dir.path(), "demo", &[("hi.shs", HI)]);
    let compiled = Scripts::default();
    let first = OtterTools::new(&compiled, &scripts, Default::default());
    assert_eq!(first.definitions()[0].description, "Greets.");
    // Unchanged: the same compiled tool.
    let again = compiled.tool(&scripts[0], false).unwrap();
    assert!(std::ptr::eq(
      again.definition(),
      first.listed[0].tool.definition()
    ));
    std::fs::write(
      dir.path().join("skills/demo/tools/hi.shs"),
      HI.replace("Greets.", "Greets anyone."),
    )
    .unwrap();
    let after = OtterTools::new(&compiled, &scripts, Default::default());
    assert_eq!(after.definitions()[0].description, "Greets anyone.");
    // A script that does not compile yet is tried again when a file it
    // may include appears.
    std::fs::write(
      dir.path().join("skills/demo/tools/hi.shs"),
      format!("@include(\"lib.shs\")\n{HI}"),
    )
    .unwrap();
    assert!(
      OtterTools::new(&compiled, &scripts, Default::default())
        .listed
        .is_empty()
    );
    std::fs::write(dir.path().join("skills/demo/tools/lib.shs"), "@const(x 1)").unwrap();
    assert!(compiled.tool(&scripts[0], false).is_ok());
  }

  fn flights() -> OtterTools {
    let b = crate::apps::SKILLS
      .iter()
      .find(|b| b.name == "google-flights")
      .unwrap();
    OtterTools::new(
      &Scripts::default(),
      &crate::otter::built_in_scripts(b),
      Default::default(),
    )
  }

  #[test]
  fn the_flights_tools_load() {
    let tools = flights();
    let defs = tools.definitions();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
      names,
      ["google_flights_cheap_dates", "google_flights_search"]
    );
    assert_eq!(
      defs[0].input_schema["required"],
      json!(["from", "to", "from-date", "to-date"])
    );
    assert_eq!(
      defs[1].input_schema["required"],
      json!(["from", "to", "date"])
    );
    // They reach the network: they ask.
    assert!(tools.listed.iter().all(|l| l.tool.effects().io));
  }

  #[tokio::test]
  async fn the_flights_tools_check_their_arguments() {
    let mut tools = flights();
    tools.yolo = true;
    let out = tools
      .call(&thread(), "google_flights_search", &json!({"from": "LHR"}))
      .await;
    assert!(!out.ok);
    assert_eq!(out.output, "from, to and date are required");
  }

  fn youtube() -> OtterTools {
    let b = crate::apps::SKILLS
      .iter()
      .find(|b| b.name == "youtube")
      .unwrap();
    let mut tools = OtterTools::new(
      &Scripts::default(),
      &crate::otter::built_in_scripts(b),
      Default::default(),
    );
    tools.yolo = true;
    tools
  }

  #[tokio::test]
  async fn the_youtube_tool_loads_and_checks_its_arguments() {
    let tools = youtube();
    let defs = tools.definitions();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["youtube_video"]);
    assert_eq!(defs[0].input_schema["required"], json!(["video"]));
    // It reaches the network: it asks.
    assert!(tools.listed.iter().all(|l| l.tool.effects().io));
    let out = tools.call(&thread(), "youtube_video", &json!({})).await;
    assert!(!out.ok);
    assert_eq!(out.output, "video is required: a YouTube video's ID or URL");
  }

  /// YouTube itself: `cargo test -p stitch-node --lib -- --ignored
  /// otter::host::tests::live_youtube --nocapture`.
  #[tokio::test]
  #[ignore = "reaches YouTube"]
  async fn live_youtube() {
    let mut tools = youtube();
    tools.scratch = std::env::temp_dir();
    let out = tools
      .call(
        &thread(),
        "youtube_video",
        &json!({"video": "https://youtu.be/JswxPY1f0lw?t=10", "max_chars": 2000, "max_comments": 3}),
      )
      .await;
    println!("{}", out.output);
    assert!(out.ok, "{}", out.output);
    assert!(
      out
        .output
        .contains("Title: The Psychology of Johnny Silverhand")
    );
    assert!(out.output.contains("\n[0:"), "no transcript");
    assert!(out.output.contains("from_seconds:"), "not cut");
    assert!(out.output.contains("Top comments:"), "no comments");
    // A language it has no captions in: the video's own, said so.
    let out = tools
      .call(
        &thread(),
        "youtube_video",
        &json!({"video": "JswxPY1f0lw", "language": "ja", "omit_comments": true, "max_chars": 1000}),
      )
      .await;
    println!("{}", out.output);
    assert!(
      out.ok && out.output.contains("none in ja"),
      "{}",
      out.output
    );
  }

  /// A month ahead, and the days after it.
  fn ahead(days: i64) -> String {
    (jiff::Zoned::now().date() + jiff::Span::new().days(30 + days)).to_string()
  }

  /// Google Flights itself: `cargo test -p stitch-node --lib -- --ignored
  /// --test-threads 1 otter::host::tests::live`.
  #[tokio::test]
  #[ignore = "reaches Google Flights"]
  async fn live_search_and_cheap_dates() {
    let mut tools = flights();
    tools.yolo = true;
    tools.scratch = std::env::temp_dir();
    let out = tools
      .call(
        &thread(),
        "google_flights_search",
        &json!({"from": "LHR", "to": "JFK", "date": ahead(0), "max-stops": 0}),
      )
      .await;
    println!("{}", out.output);
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("One-way LHR -> JFK"));
    assert!(out.output.contains("\n1. "), "no itineraries");
    let out = tools
      .call(
        &thread(),
        "google_flights_cheap_dates",
        &json!({"from": "LHR", "to": "JFK", "from-date": ahead(0), "to-date": ahead(3)}),
      )
      .await;
    println!("{}", out.output);
    assert!(out.ok, "{}", out.output);
  }
}
