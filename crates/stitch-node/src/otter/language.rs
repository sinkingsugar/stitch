//! The language's own tools (design §2.17): `otter_search`,
//! `otter_describe`, `otter_check` and `otter_run`, offered with the
//! built-in `otter` skill. They answer as the `otter` command's `search`,
//! `describe`, `check` and `run` do, so what an agent learns in one works
//! in the other; the catalog is the one skills' tools compile against.
//!
//! - `otter_search` ranks the catalog's matches (an exact name first, then
//!   names that start with the query, names that contain it, then
//!   summaries; several words: the shards matching most of them) and gives
//!   each its signature.
//! - `otter_check` loads, lowers, composes and type-checks without running:
//!   `otter check`'s diagnostics, then the effects a run would have.
//! - `otter_run` checks, then runs on a mesh of its own: services with the
//!   thread's files (`FsAccess`), metered so a cancellation or the time
//!   limit stops even a loop that never waits (`Mesh::tick_budgeted`),
//!   sleeping while it waits (`Mesh::wait`). Its output is `otter run`'s:
//!   what it logged, then each entry's outcome.
//! - `@include` and `@read` read files in the thread's directory only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use otter_core::describe::OutputDesc;
use otter_core::diagnostic::{Diagnostic, PathStep, Phase, json_str};
use otter_core::fs_access::FsAccess;
use otter_core::shard::ShardType;
use otter_core::signature::{Effects, SignatureInput};
use otter_core::{
  Budget, CancelToken, Catalog, CompiledWire, Error, InstanceId, Mesh, Outcome, Var,
};
use otter_lang::files::File;
use otter_lang::lower::ROOT_WIRE;
use otter_lang::{Files, Program, Source, render};
use serde_json::{Value, json};
use stitch_providers::ToolDef;

use super::calls::ToolCalls;

/// The tools, by name.
pub(crate) const NAMES: [&str; 4] = ["otter_check", "otter_describe", "otter_run", "otter_search"];

/// What the code is called in diagnostics (`code:2:5: …`).
const CODE: &str = "code";

/// How many matches `otter_search` lists unless asked for more.
const DEFAULT_LIMIT: usize = 25;

pub(crate) fn definitions() -> Vec<ToolDef> {
  let code = json!({"type": "string", "description": "Otter source: top-level code, or declarations (@fn, @wire, @mesh, @run). Diagnostics call it `code`."});
  let file = json!({"type": "string", "description": "Instead of code: a script in the thread's directory (relative to it), as `otter check <file>` takes it; its includes resolve next to it."});
  let input =
    json!({"description": "A JSON value given to the top-level code as its input (absent: none)."});
  vec![
    ToolDef {
      name: "otter_check".into(),
      description: "Check Otter code without running it, as `otter check` does: parse, lower, \
        compose and type-check, every problem with its line and column (the code is `code`), \
        then the effects running it would have: io, time, suspends, random, unknown, or none \
        (it only computes, so otter_run runs it without asking). `@include` and `@read` read \
        files in the thread's directory; `file` checks a script there (a skill's tool: \
        `.stitch/skills/<skill>/tools/<tool>.otr`). `json: true` gives `otter check --json`'s \
        report."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "code": code.clone(),
          "file": file.clone(),
          "input": input.clone(),
          "json": {"type": "boolean", "description": "The report as `otter check --json` prints it."}
        }
      }),
    },
    ToolDef {
      name: "otter_describe".into(),
      description: "Describe one shard in full, as `otter describe <shard>` does (JSON): what \
        it does, its input and output types, every parameter with its types, forms, help, \
        whether it is required and its default, and its effects. A name it does not know \
        answers with the closest ones. Use it on every shard before writing a call to it."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "name": {"type": "string", "description": "The shard's exact name or alias, e.g. `Http.Get`, `Take`, `String.Join`."}
        },
        "required": ["name"]
      }),
    },
    ToolDef {
      name: "otter_run".into(),
      description: "Run Otter code and get what `otter run` prints: the lines it logged, then \
        each entry's outcome (`root: <value>` for top-level code, `root: failed: <why>`). It is \
        checked first: code with problems does not run, and its diagnostics come back. Code \
        with effects (the network, files, the clock, waiting, randomness; `Log` counts as I/O) \
        asks the user first unless allowed, and a read-only thread is refused it; code that \
        only computes runs at once. Files are confined to the thread's directory. It stops at \
        the node's time limit for tools, or when the turn is interrupted. `json: true` gives \
        `otter run --json`'s object."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "code": code,
          "file": file,
          "input": input,
          "json": {"type": "boolean", "description": "The result as `otter run --json` prints it."}
        }
      }),
    },
    ToolDef {
      name: "otter_search".into(),
      description: "Search the Otter catalog (the shards code and tools here can use: the core, \
        `Http.*`, `FS.*`, `Regex.*`, `Hash.*` and more) by name, alias or summary, as `otter \
        search <text>` does, best first. Each match is `Name (input -> output): summary`. Never \
        guess a shard: search, then otter_describe it."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "query": {"type": "string", "description": "Text in a shard's name or summary: `json`, `Http`, `split`; several words find the shards matching most of them."},
          "limit": {"type": "integer", "minimum": 1, "description": "The most matches to list (default 25)."}
        },
        "required": ["query"]
      }),
    },
  ]
}

fn catalog() -> Result<&'static Catalog, String> {
  super::host::catalog()
}

/// `otter_search`.
pub(crate) fn search(input: &Value) -> Result<String, String> {
  let query = input
    .get("query")
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|q| !q.is_empty())
    .ok_or("query is required: text in a shard's name or summary")?;
  let limit = input
    .get("limit")
    .and_then(Value::as_u64)
    .map_or(DEFAULT_LIMIT, |l| l.max(1) as usize);
  let hits = ranked(catalog()?, query);
  if hits.is_empty() {
    return Ok(format!(
      "no shard matches {query:?}: try a shorter word, or another one"
    ));
  }
  let mut lines: Vec<String> = hits.iter().take(limit).map(|s| line(s)).collect();
  if hits.len() > limit {
    lines.push(format!(
      "… and {} more: narrow the query, or raise limit",
      hits.len() - limit
    ));
  }
  Ok(lines.join("\n"))
}

/// The catalog's matches for `query`, best first; for several words, the
/// shards matching any, those matching most first.
fn ranked(catalog: &Catalog, query: &str) -> Vec<&'static ShardType> {
  let whole = catalog.search(query);
  let words: Vec<&str> = query.split_whitespace().collect();
  if !whole.is_empty() || words.len() < 2 {
    let mut hits = whole;
    hits.sort_by_key(|s| (rank(s, query), s.name()));
    return hits;
  }
  let mut matched: Vec<(&'static ShardType, usize, u8)> = Vec::new();
  for word in &words {
    for s in catalog.search(word) {
      match matched.iter_mut().find(|(m, ..)| std::ptr::eq(*m, s)) {
        Some(m) => {
          m.1 += 1;
          m.2 = m.2.min(rank(s, word));
        }
        None => matched.push((s, 1, rank(s, word))),
      }
    }
  }
  matched.sort_by_key(|(s, n, r)| (std::cmp::Reverse(*n), *r, s.name()));
  matched.into_iter().map(|(s, ..)| s).collect()
}

/// 0: the name or an alias is the query; 1: starts with it (or a dotted
/// part does: `Join` in `String.Join`); 2: contains it; 3: the summary does.
fn rank(s: &ShardType, query: &str) -> u8 {
  let q = query.to_lowercase();
  let names = std::iter::once(s.name()).chain(s.desc.aliases.iter().copied());
  let mut best = 3;
  for n in names {
    let n = n.to_lowercase();
    let r = if n == q || n.rsplit('.').next() == Some(q.as_str()) {
      0
    } else if n.starts_with(&q) || n.split('.').any(|p| p.starts_with(&q)) {
      1
    } else if n.contains(&q) {
      2
    } else {
      3
    };
    best = best.min(r);
  }
  best
}

/// `Name (input -> output): summary`.
fn line(s: &ShardType) -> String {
  let d = &s.desc;
  let input = match d.signature().input {
    SignatureInput::Ignored => "None".to_owned(),
    SignatureInput::Type(t) => t.to_string(),
  };
  let output = match d.output {
    OutputDesc::Fixed(t) => t.name().to_owned(),
    OutputDesc::Passthrough => "its input".to_owned(),
    OutputDesc::SameAsInput => "the input's type".to_owned(),
    OutputDesc::Dynamic(how) => how.to_owned(),
  };
  let aliases = if d.aliases.is_empty() {
    String::new()
  } else {
    format!(" [also {}]", d.aliases.join(", "))
  };
  format!("{}{aliases} ({input} -> {output}): {}", s.name(), d.summary)
}

/// `otter_describe`.
pub(crate) fn describe(input: &Value) -> Result<String, String> {
  let name = input
    .get("name")
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|n| !n.is_empty())
    .ok_or("name is required: a shard's exact name")?;
  let catalog = catalog()?;
  if let Some(json) = catalog.describe_json(name) {
    return Ok(json);
  }
  let names: Vec<String> = catalog.names().iter().map(|n| n.to_string()).collect();
  let near = otter_lang::problem::closest(name, names, 3);
  let hint = if near.is_empty() {
    " (otter_search finds shards by what they do)".to_owned()
  } else {
    format!(" (did you mean: {})", near.join(", "))
  };
  Err(format!("no shard named {name}{hint}"))
}

/// The code and input of `otter_check` and `otter_run`.
#[derive(Clone)]
pub(crate) struct Request {
  /// What diagnostics call it: `code`, or the file's path in the thread's
  /// directory.
  name: String,
  code: String,
  file: bool,
  input: Option<Var>,
  /// The input's shape is not known before it runs (an event's payload,
  /// docs/events.md): compiled as any value, its paths checked at run time.
  input_any: bool,
  json: bool,
  /// A capability profile holds the turn (docs/events.md): the code
  /// compiles against the catalog without the shards it denies, and a
  /// denied one is said as such.
  pub(crate) shards: Option<crate::profile::ShardRule>,
}

impl Request {
  /// The arguments, a `file` read from `cwd`.
  pub(crate) fn parse(input: &Value, cwd: &Path) -> Result<Request, String> {
    let text = |k: &str| input.get(k).and_then(Value::as_str);
    let (name, code, file) = match (text("code"), text("file")) {
      (Some(code), None) => (CODE.to_owned(), code.to_owned(), false),
      (None, Some(path)) => {
        let f = Confined::new(cwd).read("", path)?;
        let code = String::from_utf8(f.bytes).map_err(|_| format!("{path} is not UTF-8 text"))?;
        (f.name, code, true)
      }
      _ => return Err("give the code, or a file in the thread's directory (one of them)".into()),
    };
    let input_any = input.get("input_any").and_then(Value::as_bool) == Some(true);
    let value = match input.get("input") {
      None | Some(Value::Null) if input_any => Some(Var::None),
      None | Some(Value::Null) => None,
      Some(v) => Some(otter_core::json::parse(&v.to_string()).map_err(|e| format!("input: {e}"))?),
    };
    Ok(Request {
      name,
      code,
      file,
      input: value,
      input_any,
      json: input.get("json").and_then(Value::as_bool) == Some(true),
      shards: None,
    })
  }

  /// How a question shows it: the code, and its input.
  pub(crate) fn command(&self) -> String {
    let mut out = if self.file {
      format!("otter_run {}\n{}", self.name, self.code.trim())
    } else {
      format!("otter_run {}", self.code.trim())
    };
    if let Some(v) = &self.input {
      out.push_str(&format!(
        "\ninput: {}",
        otter_core::json::write(v).unwrap_or_else(|_| v.to_string())
      ));
    }
    out
  }
}

/// `@include` and `@read`, confined to one directory (the thread's): a
/// path resolves against the including file's directory there, and names
/// are relative to it.
pub(super) struct Confined {
  root: Option<PathBuf>,
}

impl Confined {
  pub(super) fn new(dir: &Path) -> Confined {
    Confined {
      root: std::fs::canonicalize(dir).ok(),
    }
  }
}

impl Files for Confined {
  fn resolve(&self, from: &str, path: &str, looked: &mut Vec<String>) -> Result<File, String> {
    let Some(root) = &self.root else {
      return Err("this thread has no directory to read files from".into());
    };
    let wanted = root
      .join(Path::new(from).parent().unwrap_or(Path::new("")))
      .join(path);
    looked.push(wanted.display().to_string());
    let real = std::fs::canonicalize(&wanted).map_err(|e| format!("{path}: {e}"))?;
    if !real.starts_with(root) {
      return Err(format!(
        "{path} is outside the thread's directory ({})",
        root.display()
      ));
    }
    let bytes = std::fs::read(&real).map_err(|e| format!("{path}: {e}"))?;
    let name = real
      .strip_prefix(root)
      .unwrap_or(&real)
      .display()
      .to_string();
    Ok(File {
      key: real.display().to_string(),
      name,
      bytes,
    })
  }

  fn contents(&self, name: &str) -> Option<Vec<u8>> {
    std::fs::read(self.root.as_ref()?.join(name)).ok()
  }
}

/// A program loaded, its entries compiled on a mesh of its own.
struct Prepared {
  program: Program,
  mesh: Mesh,
  entries: Vec<(String, Arc<CompiledWire>, Var)>,
  effects: Effects,
}

fn rendered(diagnostics: &[Diagnostic], source: &Source) -> String {
  diagnostics
    .iter()
    .map(|d| render(d, source))
    .collect::<String>()
    .trim_end()
    .to_owned()
}

/// A compose error as a located diagnostic, as `otter run` reports it.
fn located(program: &Program, wire: &str, err: Error) -> Diagnostic {
  let mut d = match err {
    Error::Diagnostic(d) => *d,
    other => Diagnostic::new(
      Phase::Compose,
      "compose-error",
      "compose-error",
      other.to_string(),
    ),
  };
  let span = program.lowered.map.locate(&d).or_else(|| {
    let mut probe = Diagnostic::new(Phase::Compose, "", "", "");
    probe.path = vec![PathStep::Wire(wire.to_owned())];
    program.lowered.map.locate(&probe)
  });
  match span {
    Some(span) => otter_lang::problem::locate(&mut d, &program.source, span),
    None => d.file = Some(program.source.name.clone()),
  }
  d
}

/// Loads the code against the catalog scripts run on, or, `approving`,
/// the one `otter_run`'s approval reads effects from (`Tool.Call` has none
/// there: each call is approved as its tool is).
fn load(req: &Request, cwd: &Path, approving: bool) -> Result<Program, (Source, Vec<Diagnostic>)> {
  let source = Source::new(req.name.as_str(), req.code.as_str());
  let catalog = if approving {
    super::host::approval_catalog()
  } else {
    catalog()
  };
  let catalog = match catalog {
    Ok(c) => c,
    Err(e) => {
      return Err((
        source,
        vec![Diagnostic::new(Phase::Construct, "generic", "catalog", e)],
      ));
    }
  };
  load_held(source, catalog, req.shards.as_ref(), &Confined::new(cwd))
}

/// Loads against `catalog`, or, held by a capability profile, against it
/// without the shards `rule` denies: a denied shard is refused here, before
/// anything runs, its diagnostic saying why.
pub(crate) fn load_held(
  source: Source,
  catalog: &Catalog,
  rule: Option<&crate::profile::ShardRule>,
  files: &dyn Files,
) -> Result<Program, (Source, Vec<Diagnostic>)> {
  let Some(rule) = rule else {
    return Program::load_with(source, catalog, &HashMap::new(), files);
  };
  let held = match rule.catalog(catalog) {
    Ok(c) => c,
    Err(e) => {
      return Err((
        source,
        vec![Diagnostic::new(Phase::Construct, "generic", "catalog", e)],
      ));
    }
  };
  Program::load_with(source, &held, &HashMap::new(), files).map_err(|(source, mut ds)| {
    for d in &mut ds {
      if d.code != "unknown-shard" {
        continue;
      }
      if let Some(why) = d
        .shard
        .as_deref()
        .and_then(|n| catalog.get(n))
        .and_then(|ty| rule.denies(ty))
      {
        d.message = why;
      }
    }
    (source, ds)
  })
}

/// Loads and compiles the entries, the top-level code with `input`; the
/// diagnostics rendered when it does not compile.
fn prepare(
  req: &Request,
  cwd: &Path,
  access: FsAccess,
  approving: bool,
  calls: Option<Arc<ToolCalls>>,
) -> Result<Prepared, (String, Vec<Diagnostic>)> {
  let input = req.input.as_ref();
  let program = load(req, cwd, approving).map_err(|(s, ds)| (rendered(&ds, &s), ds))?;
  let names = program.entries();
  if input.is_some() && !names.iter().any(|n| n == ROOT_WIRE) {
    let d = Diagnostic::new(
      Phase::Compose,
      "generic",
      "input",
      "input goes to the top-level code, and this code has none (it runs @run's mesh)",
    );
    return Err((d.message.clone(), vec![d]));
  }
  let mut mesh = Mesh::new();
  for w in &program.lowered.wires {
    mesh.add_wire(w.clone());
  }
  for f in &program.lowered.functions {
    mesh.add_function(f.clone());
  }
  let mut services = otter_core::service::Services::default();
  services.insert(access);
  if let Some(calls) = calls {
    services.insert_arc(calls);
  }
  *mesh.services_mut() = services;
  let mut entries = Vec::new();
  let mut effects = Effects::NONE;
  let mut problems = Vec::new();
  for name in names {
    let value = match input {
      Some(v) if name == ROOT_WIRE => v.clone(),
      _ => Var::None,
    };
    let ty = if req.input_any && name == ROOT_WIRE {
      otter_core::Type::any()
    } else {
      value.type_of()
    };
    match mesh.compile(&name, ty) {
      Ok(compiled) => {
        effects = effects.union(compiled.flow.analysis.effects);
        entries.push((name, compiled, value));
      }
      Err(e) => {
        let d = located(&program, &name, e);
        if !problems.contains(&d) {
          problems.push(d);
        }
      }
    }
  }
  if !problems.is_empty() {
    return Err((rendered(&problems, &program.source), problems));
  }
  Ok(Prepared {
    program,
    mesh,
    entries,
    effects,
  })
}

/// `none`, or the effects by Otter's names.
pub(crate) fn effects_text(e: Effects) -> String {
  let names: Vec<&str> = [
    (e.io, "io"),
    (e.time, "time"),
    (e.suspends, "suspends"),
    (e.random, "random"),
    (e.unknown, "unknown"),
  ]
  .into_iter()
  .filter_map(|(on, n)| on.then_some(n))
  .collect();
  if names.is_empty() {
    "none".into()
  } else {
    names.join(", ")
  }
}

/// The effects of the entries `names` of the code loaded for `otter_run`'s
/// approval (`Tool.Call` has none there).
fn approval_effects(req: &Request, cwd: &Path, names: &[String]) -> Effects {
  match load(req, cwd, true) {
    Ok(program) => program
      .analyze()
      .wires
      .iter()
      .filter(|w| names.contains(&w.name))
      .fold(Effects::NONE, |e, w| e.union(w.analysis.effects)),
    Err(_) => Effects::NONE,
  }
}

/// `otter_check`: `Ok` when it compiles. Runs nothing; reads included
/// files in `cwd`.
pub(crate) fn check(req: &Request, cwd: &Path) -> Result<String, String> {
  // Without input, `otter check` itself: its report and JSON.
  if req.input.is_none() {
    let (report, source) = match load(req, cwd, false) {
      Ok(program) => {
        let names = program.entries();
        (program.analyze(), (program.source, names))
      }
      // Said here rather than as a `CheckReport`, which Otter builds and
      // grows (`#[non_exhaustive]` since @if): a load that failed has its
      // diagnostics alone.
      Err((_, diagnostics)) if req.json => {
        return Err(format!(
          "{{\"ok\":false,\"file\":{},\"diagnostics\":[{}],\"wires\":[],\"functions\":[]}}",
          json_str(&req.name),
          diagnostics
            .iter()
            .map(Diagnostic::to_json)
            .collect::<Vec<_>>()
            .join(",")
        ));
      }
      Err((source, diagnostics)) => return Err(rendered(&diagnostics, &source)),
    };
    if req.json {
      return if report.ok() {
        Ok(report.to_json())
      } else {
        Err(report.to_json())
      };
    }
    let (source, names) = source;
    if !report.ok() {
      return Err(rendered(&report.diagnostics, &source));
    }
    let effects = report
      .wires
      .iter()
      .filter(|w| names.contains(&w.name))
      .fold(Effects::NONE, |e, w| e.union(w.analysis.effects));
    let asked = approval_effects(req, cwd, &names);
    return Ok(ok_text(&req.name, effects, asked));
  }
  // With input: its entries composed as a run would.
  match prepare(req, cwd, FsAccess::denied(), false, None) {
    Ok(p) if req.json => Ok(format!(
      "{{\"ok\":true,\"file\":{},\"diagnostics\":[],\"effects\":{}}}",
      json_str(&req.name),
      p.effects.to_json()
    )),
    Ok(p) => {
      let asked = prepare(req, cwd, FsAccess::denied(), true, None)
        .map(|a| a.effects)
        .unwrap_or(p.effects);
      Ok(ok_text(&req.name, p.effects, asked))
    }
    Err((_, ds)) if req.json => Err(format!(
      "{{\"ok\":false,\"file\":{},\"diagnostics\":[{}]}}",
      json_str(&req.name),
      ds.iter()
        .map(Diagnostic::to_json)
        .collect::<Vec<_>>()
        .join(",")
    )),
    Err((text, _)) => Err(text),
  }
}

/// `effects`: the code's; `asked`: those `otter_run` asks for, which leave
/// out `Tool.Call`'s (each call is approved as its tool is).
fn ok_text(name: &str, effects: Effects, asked: Effects) -> String {
  let what = if asked == Effects::NONE {
    "it only computes: otter_run runs it without asking"
  } else {
    "otter_run asks the user before running it, unless allowed"
  };
  let tool_calls = if asked == effects {
    String::new()
  } else if asked == Effects::NONE {
    "; all of them Tool.Call's, each call approved as its tool is (otter_run runs it \
     without asking)"
      .to_owned()
  } else {
    format!(
      "; without Tool.Call's, which are approved per call as each tool is: {} ({what})",
      effects_text(asked)
    )
  };
  if tool_calls.is_empty() {
    format!("{name}: ok\neffects: {} ({what})", effects_text(effects))
  } else {
    format!("{name}: ok\neffects: {}{tool_calls}", effects_text(effects))
  }
}

/// What `otter_run` needs to know before it runs: the effects it asks for
/// (none for `Tool.Call`: each call is approved as its tool is), or why it
/// cannot run.
pub(crate) fn effects(req: &Request, cwd: &Path) -> Result<Effects, String> {
  match prepare(req, cwd, FsAccess::denied(), true, None) {
    Ok(p) => Ok(p.effects),
    Err((_, ds)) if req.json => Err(run_json(&req.name, &ds, &[], None)),
    Err((text, _)) => Err(text),
  }
}

/// How a run ended.
struct Ran {
  ticks: i64,
  outcomes: Vec<(String, Option<Outcome>)>,
  spawned_failures: Vec<(String, Outcome)>,
  /// Cancelled, or out of time, before it finished.
  interrupted: Option<String>,
}

impl Ran {
  fn succeeded(&self) -> bool {
    self.interrupted.is_none()
      && self.spawned_failures.is_empty()
      && !self
        .outcomes
        .iter()
        .any(|(_, o)| matches!(o, Some(Outcome::Failed(_))))
  }
}

/// `otter_run`'s run, on this (blocking) thread: `Ok` when nothing failed.
/// Its files are confined to `cwd` (read only when `read_only`); it stops
/// at `timeout` or when `cancel` is cancelled.
pub(crate) fn run(
  req: &Request,
  cwd: &Path,
  read_only: bool,
  timeout: Duration,
  cancel: &CancelToken,
  calls: Option<Arc<ToolCalls>>,
) -> Result<String, String> {
  let access = FsAccess::within(cwd).unwrap_or_else(|_| FsAccess::denied());
  let access = if read_only {
    access.read_only()
  } else {
    access
  };
  let (ran, log) = otter_core::log::capture(|| {
    let p = prepare(req, cwd, access, false, calls)?;
    Ok::<_, (String, Vec<Diagnostic>)>(execute(p, timeout, cancel))
  });
  let ran = match ran {
    Ok(ran) => ran,
    Err((_, ds)) if req.json => return Err(run_json(&req.name, &ds, &log, None)),
    Err((text, _)) => return Err(text),
  };
  let ok = ran.succeeded();
  let text = if req.json {
    run_json(&req.name, &[], &log, Some(&ran))
  } else {
    let mut lines = log;
    lines.extend(outcome_lines(&ran));
    lines.join("\n")
  };
  if ok { Ok(text) } else { Err(text) }
}

/// A schedule's check (design §2.3): runs as `run` does and gives what the
/// top-level code returned, as text, or `None` when it returned nothing
/// (`none`, `false`, an empty string, sequence or table). What failed, or
/// code without top-level code, is an error.
pub(crate) fn check_value(
  req: &Request,
  cwd: &Path,
  read_only: bool,
  timeout: Duration,
  cancel: &CancelToken,
) -> Result<Option<String>, String> {
  let access = FsAccess::within(cwd).unwrap_or_else(|_| FsAccess::denied());
  let access = if read_only {
    access.read_only()
  } else {
    access
  };
  let (ran, log) = otter_core::log::capture(|| {
    let p = prepare(req, cwd, access, false, None)?;
    Ok::<_, (String, Vec<Diagnostic>)>(execute(p, timeout, cancel))
  });
  let ran = ran.map_err(|(text, _)| text)?;
  let failed = |why: String| {
    let mut lines = log.clone();
    lines.push(why);
    lines.join("\n")
  };
  if let Some(why) = &ran.interrupted {
    return Err(failed(why.clone()));
  }
  for (wire, o) in &ran.spawned_failures {
    if let Outcome::Failed(e) = o {
      return Err(failed(format!("{wire}: failed: {}", why(e))));
    }
  }
  let root = ran.outcomes.iter().find(|(w, _)| w == ROOT_WIRE);
  match root.map(|(_, o)| o) {
    Some(Some(Outcome::Completed(v))) => Ok(check_text(v)),
    Some(Some(Outcome::Failed(e))) => Err(failed(format!("failed: {}", why(e)))),
    Some(Some(_)) => Err(failed("it stopped before returning a value".into())),
    Some(None) => Err(failed("it did not complete".into())),
    None => Err("a check is top-level code that returns a value".into()),
  }
}

/// A check's value as the firing's message gives it; `None`: nothing.
fn check_text(v: &Var) -> Option<String> {
  match v {
    Var::None | Var::Bool(false) => None,
    Var::String(_) => v
      .as_str()
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .map(str::to_owned),
    Var::Seq(_) if v.as_seq().is_some_and(<[Var]>::is_empty) => None,
    Var::Table(_) if v.as_table().is_some_and(|t| t.is_empty()) => None,
    v => Some(otter_core::json::write(v).unwrap_or_else(|_| v.to_string())),
  }
}

/// Runs the entries until each finishes, `@run`'s iterations are done, the
/// time is up or `cancel` is: `Program::run` with a budget and services.
fn execute(mut p: Prepared, timeout: Duration, cancel: &CancelToken) -> Ran {
  let started = Instant::now();
  let deadline = started + timeout;
  let budget = Budget {
    fuel: None,
    cancel: Some(cancel.clone()),
    deadline: Some(deadline),
  }
  .start();
  let mut ids: Vec<(String, Option<InstanceId>)> = Vec::new();
  let mut outcomes: HashMap<InstanceId, Outcome> = HashMap::new();
  let mut spawned_failures = Vec::new();
  for (name, compiled, value) in std::mem::take(&mut p.entries) {
    match p.mesh.spawn(&compiled, value) {
      Ok(id) => ids.push((name, Some(id))),
      Err(e) => {
        spawned_failures.push((name.clone(), Outcome::Failed(e)));
        ids.push((name, None));
      }
    }
  }
  let run = p.program.lowered.run.as_ref();
  let frame = run
    .and_then(|r| r.fps)
    .and_then(|fps| Duration::try_from_secs_f64(1.0 / fps).ok());
  let iterations = run.and_then(|r| r.iterations);
  let mut ticks = 0i64;
  let mut next = Instant::now();
  let interrupted = loop {
    p.mesh.tick_budgeted(&budget);
    ticks += 1;
    for (id, wire, outcome) in p.mesh.take_finished() {
      if ids.iter().any(|(_, i)| *i == Some(id)) {
        outcomes.insert(id, outcome);
      } else if matches!(outcome, Outcome::Failed(_)) {
        spawned_failures.push((wire, outcome));
      }
    }
    if cancel.is_cancelled() {
      break Some("otter_run was cancelled".to_owned());
    }
    if Instant::now() >= deadline {
      break Some(format!("otter_run timed out after {timeout:?}"));
    }
    if p.mesh.running() == 0 || iterations.is_some_and(|n| ticks >= n) {
      break None;
    }
    match frame {
      // At `@run`'s rate: a cancellable sleep to the next frame.
      Some(frame) => {
        next = (next + frame).min(deadline);
        while Instant::now() < next && !cancel.is_cancelled() {
          std::thread::sleep((next - Instant::now()).min(Duration::from_millis(20)));
        }
        next = next.max(Instant::now());
      }
      None => p.mesh.wait(Some(deadline), Some(cancel)),
    }
  };
  if interrupted.is_some() {
    p.mesh.cancel_all();
  }
  // An instance the budget stopped failed as cancelled: it was interrupted.
  let outcomes = ids
    .into_iter()
    .map(|(wire, id)| {
      let o = id.and_then(|id| outcomes.remove(&id));
      let o = match o {
        Some(Outcome::Failed(Error::Cancelled)) => Some(Outcome::Cancelled),
        o => o,
      };
      (wire, o)
    })
    .collect();
  Ran {
    ticks,
    outcomes,
    spawned_failures,
    interrupted,
  }
}

/// Why a run failed, as Otter's tools say it: a runtime failure's own
/// message (it names the shard), without the runtime's "activation error"
/// in front, which tells a model nothing.
fn why(e: &Error) -> String {
  match e {
    Error::Activation(m) => m.clone(),
    other => other.to_string(),
  }
}

/// As `otter run` prints them.
fn outcome_lines(ran: &Ran) -> Vec<String> {
  let mut lines = Vec::new();
  for (wire, outcome) in &ran.outcomes {
    lines.push(match outcome {
      Some(Outcome::Completed(v)) => format!("{wire}: {v}"),
      Some(Outcome::Stopped) => format!("{wire}: stopped"),
      Some(Outcome::Cancelled) => format!("{wire}: cancelled"),
      Some(Outcome::Failed(e)) => format!("{wire}: failed: {}", why(e)),
      None if ran.interrupted.is_some() => format!("{wire}: cancelled"),
      None => format!("{wire}: still running after {} ticks", ran.ticks),
    });
  }
  for (wire, outcome) in &ran.spawned_failures {
    if let Outcome::Failed(e) = outcome {
      lines.push(format!("{wire} (spawned): failed: {}", why(e)));
    }
  }
  if let Some(why) = &ran.interrupted {
    lines.push(why.clone());
  }
  lines
}

/// `{wire, outcome, value?, error?}`, as `otter run --json` writes it.
fn outcome_json(wire: &str, outcome: Option<&Outcome>) -> String {
  let (kind, extra) = match outcome {
    Some(Outcome::Completed(v)) => (
      "completed",
      format!(",\"value\":{}", json_str(&v.to_string())),
    ),
    Some(Outcome::Stopped) => ("stopped", String::new()),
    Some(Outcome::Cancelled) => ("cancelled", String::new()),
    Some(Outcome::Failed(e)) => ("failed", format!(",\"error\":{}", json_str(&e.to_string()))),
    None => ("running", String::new()),
  };
  format!(
    "{{\"wire\":{},\"outcome\":\"{kind}\"{extra}}}",
    json_str(wire)
  )
}

/// `otter run --json`'s object (`interrupted` added when it was).
fn run_json(file: &str, diagnostics: &[Diagnostic], log: &[String], ran: Option<&Ran>) -> String {
  let join = |items: Vec<String>| items.join(",");
  let ok = diagnostics.is_empty() && ran.is_some_and(Ran::succeeded);
  let outcomes = ran.map_or(Vec::new(), |r| {
    r.outcomes
      .iter()
      .map(|(w, o)| outcome_json(w, o.as_ref()))
      .collect()
  });
  let spawned = ran.map_or(Vec::new(), |r| {
    r.spawned_failures
      .iter()
      .map(|(w, o)| outcome_json(w, Some(o)))
      .collect()
  });
  let interrupted = ran
    .and_then(|r| r.interrupted.as_deref())
    .map(|why| format!(",\"interrupted\":{}", json_str(why)))
    .unwrap_or_default();
  format!(
    "{{\"ok\":{ok},\"file\":{},\"diagnostics\":[{}],\"log\":[{}],\"outcomes\":[{}],\"spawned_failures\":[{}]{interrupted}}}",
    json_str(file),
    join(diagnostics.iter().map(Diagnostic::to_json).collect()),
    join(log.iter().map(|l| json_str(l)).collect()),
    join(outcomes),
    join(spawned),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn req(v: Value, cwd: &Path) -> Request {
    Request::parse(&v, cwd).unwrap()
  }

  fn run_in(v: Value, cwd: &Path, timeout: Duration) -> Result<String, String> {
    run(&req(v, cwd), cwd, false, timeout, &CancelToken::new(), None)
  }

  #[test]
  fn search_ranks_the_catalogs_matches_with_their_signatures() {
    let out = search(&json!({"query": "var"})).unwrap();
    let first = out.lines().next().unwrap();
    // The exact name first, though others sort before it.
    assert!(first.starts_with("Var ("), "{out}");
    // A dotted name's last part counts as the name; summaries come last.
    let out = search(&json!({"query": "join"})).unwrap();
    let names: Vec<&str> = out.lines().map(|l| l.split(' ').next().unwrap()).collect();
    assert_eq!(
      names,
      ["Bytes.Join", "FS.Join", "String.Join", "String.Format"],
      "{out}"
    );
    assert!(
      out.contains("String.Join ([Any] -> String): Joins a sequence of strings"),
      "{out}"
    );
    // Several words: the shards matching most of them.
    let out = search(&json!({"query": "http get"})).unwrap();
    assert!(out.starts_with("Http.Get (None -> "), "{out}");
    // A limit, and what it left out.
    let out = search(&json!({"query": "expect", "limit": 2})).unwrap();
    assert_eq!(out.lines().count(), 3, "{out}");
    assert!(out.ends_with("narrow the query, or raise limit"), "{out}");
    assert!(
      search(&json!({"query": "zzzqqq"}))
        .unwrap()
        .starts_with("no shard matches")
    );
    assert!(search(&json!({})).is_err());
  }

  #[test]
  fn describe_gives_the_catalogs_entry_or_the_closest_names() {
    let out = describe(&json!({"name": "Http.Get"})).unwrap();
    let d: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(d["name"], "Http.Get");
    assert_eq!(d["params"][0]["name"], "url");
    assert_eq!(d["params"][0]["required"], true);
    assert_eq!(d["effects"]["io"], true);
    let miss = describe(&json!({"name": "Tak"})).unwrap_err();
    assert!(
      miss.starts_with("no shard named Tak (did you mean: Take"),
      "{miss}"
    );
    assert!(
      describe(&json!({"name": "Zzzzzzzz"}))
        .unwrap_err()
        .ends_with("(otter_search finds shards by what they do)")
    );
  }

  #[test]
  fn check_reports_problems_where_they_are_and_effects() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    // A type error, with its line and column.
    let err = check(
      &req(json!({"code": "1 | Math.Add(2)\n\"a\" | Math.Add(1)"}), cwd),
      cwd,
    )
    .unwrap_err();
    assert!(err.starts_with("code:2:7: "), "{err}");
    // A parse error.
    let err = check(&req(json!({"code": "x = 1"}), cwd), cwd).unwrap_err();
    assert!(err.starts_with("code:1:5: parse error"), "{err}");
    // Fine: what running it would do.
    let ok = check(&req(json!({"code": "[1 2] | Count"}), cwd), cwd).unwrap();
    assert_eq!(
      ok,
      "code: ok\neffects: none (it only computes: otter_run runs it without asking)"
    );
    let ok = check(&req(json!({"code": "\"x\" | Log  Pause(0.1)"}), cwd), cwd).unwrap();
    assert!(
      ok.contains("effects: io, time, suspends (otter_run asks"),
      "{ok}"
    );
    // As `otter check --json`.
    let out = check(&req(json!({"code": "1", "json": true}), cwd), cwd).unwrap();
    let report: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["file"], "code");
    assert_eq!(report["wires"][0]["wire"], "root");
    let out = check(&req(json!({"code": "Nope", "json": true}), cwd), cwd).unwrap_err();
    let report: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["diagnostics"][0]["code"], "unknown-shard");
    // With an input, composed with it.
    let input_code = json!({"code": "Take(\"n\") | ExpectInt | Math.Add(1)", "input": {"n": 41}});
    assert!(check(&req(input_code, cwd), cwd).is_ok());
  }

  #[test]
  fn files_and_includes_stay_in_the_threads_directory() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("work");
    let tools = cwd.join(".stitch/skills/s/tools");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::write(
      tools.join("lib.otr"),
      "@fn(Twice input: Int output: Int params: {} { Math.Multiply(2) })",
    )
    .unwrap();
    std::fs::write(tools.join("main.otr"), "@include(\"lib.otr\")\n21 | Twice").unwrap();
    std::fs::write(dir.path().join("secret.otr"), "@const(s 1)").unwrap();
    // A file, its includes next to it.
    let ok = check(
      &req(json!({"file": ".stitch/skills/s/tools/main.otr"}), &cwd),
      &cwd,
    )
    .unwrap();
    assert!(
      ok.starts_with(".stitch/skills/s/tools/main.otr: ok"),
      "{ok}"
    );
    assert_eq!(
      run_in(
        json!({"file": ".stitch/skills/s/tools/main.otr"}),
        &cwd,
        Duration::from_secs(5)
      )
      .unwrap(),
      "root: 42"
    );
    // Nothing outside it, by a path or by an include.
    assert!(Request::parse(&json!({"file": "../secret.otr"}), &cwd).is_err());
    let err = check(
      &req(json!({"code": "@include(\"../secret.otr\")\n1"}), &cwd),
      &cwd,
    )
    .unwrap_err();
    assert!(err.contains("outside the thread's directory"), "{err}");
    assert!(Request::parse(&json!({"code": "1", "file": "x"}), &cwd).is_err());
    assert!(Request::parse(&json!({}), &cwd).is_err());
  }

  #[test]
  fn runs_print_the_log_and_outcomes() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let t = Duration::from_secs(5);
    assert_eq!(
      run_in(json!({"code": "\"hi\" | Log\n[1 2 3] | Count"}), cwd, t).unwrap(),
      "hi\nroot: 3"
    );
    // A failure: what it logged, and why.
    let err = run_in(json!({"code": "\"about to\" | Log\n\"no\" | Fail"}), cwd, t).unwrap_err();
    assert_eq!(err, "about to\nroot: failed: no");
    // Problems: not run.
    let err = run_in(json!({"code": "\"ran\" | Log\nNope"}), cwd, t).unwrap_err();
    assert!(
      err.starts_with("code:2:1: ") && !err.contains("ran\n"),
      "{err}"
    );
    // The input goes to the top-level code.
    assert_eq!(
      run_in(
        json!({"code": "Take(\"n\") | ExpectInt | Math.Add(1)", "input": {"n": 41}}),
        cwd,
        t
      )
      .unwrap(),
      "root: 42"
    );
    // A scheduled mesh, for its iterations.
    let mesh = "@wire(tick { Keep(n 0) n | Math.Add(1) | Update(n) | Log } looped: true)\n@mesh(main)\n@schedule(main tick)\n@run(main iterations: 3)";
    assert_eq!(
      run_in(json!({"code": mesh}), cwd, t).unwrap(),
      "1\n2\n3\ntick: still running after 3 ticks"
    );
    let err = run_in(json!({"code": mesh, "input": 1}), cwd, t).unwrap_err();
    assert!(err.starts_with("input goes to the top-level code"), "{err}");
    // As `otter run --json`.
    let out = run_in(json!({"code": "\"hi\" | Log\n2", "json": true}), cwd, t).unwrap();
    assert_eq!(
      serde_json::from_str::<Value>(&out).unwrap(),
      json!({"ok": true, "file": "code", "diagnostics": [], "log": ["hi"],
        "outcomes": [{"wire": "root", "outcome": "completed", "value": "2"}], "spawned_failures": []})
    );
  }

  #[test]
  fn runs_write_only_in_the_threads_directory() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(dir.path()).unwrap().join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let write = |path: &str| json!({"code": format!("\"{path}\" | FS.Write(contents: \"x\")")});
    let inside = cwd.join("in.txt");
    assert!(
      run_in(
        write(&inside.display().to_string()),
        &cwd,
        Duration::from_secs(5)
      )
      .is_ok()
    );
    assert!(inside.is_file());
    let outside = dir.path().join("out.txt");
    let err = run_in(
      write(&outside.display().to_string()),
      &cwd,
      Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(err.starts_with("root: failed: "), "{err}");
    assert!(!outside.exists());
    // Read-only: reads, never writes.
    let again = cwd.join("again.txt");
    let r = req(write(&again.display().to_string()), &cwd);
    assert!(
      run(
        &r,
        &cwd,
        true,
        Duration::from_secs(5),
        &CancelToken::new(),
        None
      )
      .is_err()
    );
    assert!(!again.exists());
  }

  #[test]
  fn runs_stop_at_the_time_limit_and_when_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    // Waiting, and a loop that never waits: both stop.
    for code in [
      "\"waiting\" | Log\nRepeat({Pause} forever: true)",
      "0 | Var(n)\nRepeat({n | Math.Add(1) | Update(n)} forever: true)",
    ] {
      let started = Instant::now();
      let err = run_in(json!({"code": code}), cwd, Duration::from_millis(300)).unwrap_err();
      assert!(err.ends_with("otter_run timed out after 300ms"), "{err}");
      assert!(started.elapsed() < Duration::from_secs(3));
    }
    let cancel = CancelToken::new();
    let c = cancel.clone();
    std::thread::spawn(move || {
      std::thread::sleep(Duration::from_millis(200));
      c.cancel();
    });
    let started = Instant::now();
    let r = req(json!({"code": "\"waiting\" | Log\nPause(30.0)"}), cwd);
    let err = run(&r, cwd, false, Duration::from_secs(60), &cancel, None).unwrap_err();
    assert_eq!(err, "waiting\nroot: cancelled\notter_run was cancelled");
    assert!(started.elapsed() < Duration::from_secs(5));
  }

  /// `run`, its `Tool.Call`s answered by stand-ins: `<tool> <arguments>`.
  fn with_tools(r: &Request, cwd: &Path) -> Result<String, String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
      .enable_all()
      .build()
      .unwrap();
    rt.block_on(async {
      let (calls, rx) = ToolCalls::new();
      let dispatch = |n: String, i: Value| -> crate::tools::BoxFuture<'static, _> {
        Box::pin(async move { crate::tools::ToolOutcome::ok(format!("{n} {i}")) })
      };
      let (r, cwd) = (Arc::new(r.clone()), cwd.to_owned());
      let running = tokio::task::spawn_blocking(move || {
        run(
          &r,
          &cwd,
          false,
          Duration::from_secs(5),
          &CancelToken::new(),
          Some(calls),
        )
      });
      super::super::calls::serve(running, rx, &dispatch, &Default::default())
        .await
        .unwrap()
    })
  }

  #[test]
  fn tool_call_is_found_described_and_checked() {
    let out = search(&json!({"query": "tool"})).unwrap();
    assert!(
      out.starts_with("Tool.Call ({...} | None -> String): Calls one of the thread's tools"),
      "{out}"
    );
    let d = describe(&json!({"name": "Tool.Call"})).unwrap();
    assert!(d.contains("youtube_video") && d.contains("Maybe"), "{d}");
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    // Only Tool.Call's effects: otter_run runs it unasked.
    let code = json!({"code": "{video: \"x\"} | Tool.Call(\"youtube_video\")"});
    let ok = check(&req(code.clone(), cwd), cwd).unwrap();
    assert!(
      ok.contains("all of them Tool.Call's") && ok.contains("runs it without asking"),
      "{ok}"
    );
    assert_eq!(effects(&req(code, cwd), cwd).unwrap(), Effects::NONE);
    // With effects of its own: those.
    let code = json!({"code": "Pause(0.1)\n{} | Tool.Call(\"x\")"});
    let ok = check(&req(code.clone(), cwd), cwd).unwrap();
    assert!(
      ok.contains("without Tool.Call's") && ok.contains("asks the user"),
      "{ok}"
    );
    assert_ne!(effects(&req(code, cwd), cwd).unwrap(), Effects::NONE);
    // The input is the arguments: a table or none.
    let err = check(&req(json!({"code": "1 | Tool.Call(\"x\")"}), cwd), cwd).unwrap_err();
    assert!(err.contains("Tool.Call needs Table or None input"), "{err}");
    // Stand-ins see the arguments as JSON.
    assert_eq!(
      with_tools(&req(json!({"code": "{n: 1} | Tool.Call(\"x\")"}), cwd), cwd).unwrap(),
      "root: \"x {\\\"n\\\":1}\""
    );
  }

  /// The snippets of the skills whose tools are lazy check, and run
  /// against stand-ins of their tools.
  #[test]
  fn the_lazy_skills_snippets_call_their_tools() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    for skill in crate::apps::SKILLS.iter().filter(|b| b.lazy) {
      let blocks: Vec<&str> = skill
        .text
        .split("```otter\n")
        .skip(1)
        .map(|b| b.split("```").next().unwrap())
        .collect();
      assert!(!blocks.is_empty(), "{}", skill.name);
      for code in blocks {
        assert!(code.contains("Tool.Call"), "{code}");
        let r = req(json!({"code": code}), cwd);
        let ok = check(&r, cwd).unwrap_or_else(|e| panic!("{code}\n{e}"));
        assert!(ok.contains("runs it without asking"), "{code}\n{ok}");
        let got = with_tools(&r, cwd);
        assert!(got.is_ok(), "{code}\n{}", got.unwrap_err());
      }
    }
  }

  /// Every snippet in the `otter` skill checks; those with a `// → ` line
  /// run to it, those with `// effects: ` have them, and tools load.
  #[test]
  fn the_skills_snippets_are_right() {
    let skill = crate::apps::SKILLS
      .iter()
      .find(|b| b.name == "otter")
      .unwrap();
    let blocks: Vec<&str> = skill
      .text
      .split("```otter\n")
      .skip(1)
      .map(|b| b.split("```").next().unwrap())
      .collect();
    assert!(blocks.len() >= 8, "{}", blocks.len());
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    for code in blocks {
      let tool = code.contains("definition:");
      let checked = check(&req(json!({"code": code}), cwd), cwd);
      assert!(checked.is_ok(), "{code}\n{}", checked.unwrap_err());
      if tool {
        let t = otter_lang::Tool::load(Source::new("tool.otr", code), catalog().unwrap());
        assert!(t.is_ok(), "{code}\n{}", t.err().unwrap());
      }
      for line in code.lines() {
        if let Some(want) = line.trim().strip_prefix("// → ") {
          let got = run_in(json!({"code": code}), cwd, Duration::from_secs(5));
          assert_eq!(got.as_deref(), Ok(want), "{code}");
        }
        if let Some(want) = line.trim().strip_prefix("// effects: ") {
          let ok = checked.as_ref().unwrap();
          assert!(ok.contains(&format!("effects: {want}")), "{code}\n{ok}");
        }
      }
      // Calls to the thread's tools run against stand-ins, which answer
      // with what they were called with.
      if code.contains("Tool.Call") {
        let got = with_tools(&req(json!({"code": code}), cwd), cwd);
        assert!(got.is_ok(), "{code}\n{}", got.unwrap_err());
      }
    }
  }
}
