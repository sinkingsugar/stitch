//! App containers (design §2.23): an app's Containerfile built into an
//! image in rootless podman, and the commands its manifest declares run in
//! containers whose whole invocation Stitch owns, under grants of their
//! own (build, run, shell), each the user's.

pub mod build;
pub mod engine;
#[cfg(all(test, target_os = "linux"))]
mod live_tests;
pub mod run;
pub mod spec;
pub mod storage;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Value, json};
use stitch_records::{Author, RecordId};

use super::Manifest;
use super::threads::Owner;
use crate::node::Shared;
use crate::{Error, Result};
use spec::ContainerNeeds;

/// The three grants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
  Build,
  Run,
  Shell,
}

impl Kind {
  pub fn word(self) -> &'static str {
    match self {
      Kind::Build => "build",
      Kind::Run => "run",
      Kind::Shell => "shell",
    }
  }

  fn field(self, name: &str) -> String {
    format!("{}:{name}", self.word())
  }

  /// The hash of what this grant covers in `c`.
  pub fn hash(self, c: &ContainerNeeds) -> Option<String> {
    match self {
      Kind::Build => Some(c.build_hash()),
      Kind::Run => (!c.commands.is_empty()).then(|| c.run_hash()),
      Kind::Shell => c.shell_hash(),
    }
  }
}

/// A container grant as the user gave it.
#[derive(Clone, Debug, PartialEq)]
pub struct Granted {
  pub hash: String,
  pub spec: ContainerNeeds,
}

/// The grant standing (`None`: never answered; `Some(None)`: denied).
pub(crate) fn grant(
  index: &stitch_store::Index,
  thread: &RecordId,
  name: &str,
  kind: Kind,
) -> Option<Option<Granted>> {
  let set = index.current_user_value(thread, &kind.field(name)).ok()??;
  let v: Value = match &set.value {
    Value::String(s) => serde_json::from_str(s).ok()?,
    v => v.clone(),
  };
  if v.get("allow").and_then(Value::as_bool) != Some(true) {
    return Some(None);
  }
  let spec: ContainerNeeds = serde_json::from_value(v.get("spec")?.clone()).ok()?;
  let pins = spec.pins.clone();
  // Checked again, as a manifest's: what can no longer be declared is a
  // denial.
  let Ok(mut spec) = spec.checked() else {
    return Some(None);
  };
  spec.pins = pins;
  let hash = v.get("hash")?.as_str()?.to_owned();
  if kind.hash(&spec).as_deref() != Some(hash.as_str()) {
    return Some(None);
  }
  Some(Some(Granted { hash, spec }))
}

/// The grant of `kind` standing for an app, when given.
pub(crate) fn granted(s: &Shared, owner: &Owner, kind: Kind) -> Option<Granted> {
  grant(
    &s.state().index.lock().expect("index lock"),
    &owner.thread,
    &owner.name,
    kind,
  )
  .flatten()
}

/// An app's key: its conversation and name, as its images and containers
/// are labelled.
pub(crate) fn app_key(owner: &Owner) -> String {
  let mut h = blake3::Hasher::new();
  h.update(owner.thread.to_string().as_bytes());
  h.update(&[0]);
  h.update(owner.name.as_bytes());
  h.finalize().to_hex()[..16].to_owned()
}

/// This node's key: its data directory's, as its containers are labelled,
/// so a node cleans up only its own (a test node's never the user's).
pub fn node_key(data: &std::path::Path) -> String {
  let d = std::fs::canonicalize(data).unwrap_or_else(|_| data.to_owned());
  blake3::hash(d.to_string_lossy().as_bytes()).to_hex()[..12].to_owned()
}

/// The node's directory for containers: the seccomp profile, runs'
/// staging, the ledger.
pub(crate) fn dir(s: &Shared) -> PathBuf {
  s.data.join("containers")
}

/// What a page hears of its container grants.
pub(crate) fn said(s: &Shared, m: &Manifest) -> Option<Value> {
  let c = m.needs.as_ref()?.container.as_ref()?;
  let thread = m.thread.as_ref()?;
  let index = s.state().index.lock().expect("index lock");
  let mut v = json!({});
  for k in [Kind::Build, Kind::Run, Kind::Shell] {
    let Some(want) = k.hash(c) else { continue };
    let state = match grant(&index, thread, &m.name, k) {
      None => "ask",
      Some(None) => "denied",
      Some(Some(g)) if g.hash == want => "allowed",
      Some(Some(_)) => "changed",
    };
    v[k.word()] = json!(state);
  }
  v["allow"] = json!(format!(
    "stitch app allow-container \"{}\" --thread {thread}",
    m.name
  ));
  Some(v)
}

/// The family's container minutes today, kept on the node.
#[derive(Default)]
pub(crate) struct Minutes {
  /// App key → (day, seconds used).
  used: HashMap<String, (String, u64)>,
}

fn minutes() -> &'static Mutex<Minutes> {
  static M: OnceLock<Mutex<Minutes>> = OnceLock::new();
  M.get_or_init(|| Mutex::new(Minutes::default()))
}

fn today() -> String {
  jiff::Zoned::now().date().to_string()
}

fn minutes_file(s: &Shared) -> PathBuf {
  dir(s).join("minutes.json")
}

/// Seconds the app used today.
pub(crate) fn used_today(s: &Shared, key: &str) -> u64 {
  let mut m = minutes().lock().expect("minutes lock");
  if m.used.is_empty()
    && let Ok(text) = std::fs::read_to_string(minutes_file(s))
    && let Ok(v) = serde_json::from_str::<HashMap<String, (String, u64)>>(&text)
  {
    m.used = v;
  }
  match m.used.get(key) {
    Some((d, secs)) if *d == today() => *secs,
    _ => 0,
  }
}

/// Counts `secs` to the app's day.
pub(crate) fn count(s: &Shared, key: &str, secs: u64) {
  let mut m = minutes().lock().expect("minutes lock");
  let day = today();
  let e = m.used.entry(key.to_owned()).or_insert((day.clone(), 0));
  if e.0 != day {
    *e = (day, 0);
  }
  e.1 += secs;
  let _ = std::fs::create_dir_all(dir(s));
  let _ = std::fs::write(
    minutes_file(s),
    serde_json::to_vec(&m.used).unwrap_or_default(),
  );
}

/// The user grants an app's containers what its latest version declares
/// (each of `build`, `run`, `shell` by the hash shown), or denies them
/// all. With no hash, nothing is written: what would be granted is shown,
/// with the Containerfile (and its change since the last build grant).
pub(crate) async fn allow(
  s: &Arc<Shared>,
  app: &str,
  thread: Option<&str>,
  deny: bool,
  hashes: &[(Kind, String)],
) -> Result<Vec<String>> {
  let (thread, manifest) = super::threads::app_version(s, app, thread)?;
  let m = Manifest::read(&s.state().blobs, &manifest)
    .ok_or_else(|| Error::Config(format!("app version {manifest} is not here")))?;
  let name = m.name.clone();
  let owner = Owner {
    thread: thread.clone(),
    name: name.clone(),
  };
  let author = Author::user(s.state().node.clone());
  if deny {
    for k in [Kind::Build, Kind::Run, Kind::Shell] {
      crate::meta::set(
        s,
        &thread,
        &k.field(&name),
        json!({"allow": false}).to_string().into(),
        author.clone(),
      )?;
    }
    s.apps.bump();
    let stopped = run::stop_app(s, &app_key(&owner)).await;
    return Ok(vec![format!(
      "App \"{name}\" (thread {thread}) may no longer build or run containers{}.",
      if stopped > 0 {
        format!("; {stopped} running stopped")
      } else {
        String::new()
      }
    )]);
  }
  let c = m
    .needs
    .as_ref()
    .and_then(|n| n.container.clone())
    .ok_or_else(|| {
      Error::Config(format!(
        "app \"{name}\" declares no container (no `container` in its {})",
        super::threads::NEEDS_FILE
      ))
    })?;
  let mut out = Vec::new();
  let (engine_said, gpu_said) = engine_here(&c);
  // What each grant covers, with its hash.
  let blobs = &s.state().blobs;
  let cf_text = |c: &ContainerNeeds| -> Option<String> {
    let h: stitch_records::BlobHash = c.pins.as_ref()?.files.get("Containerfile")?.parse().ok()?;
    std::fs::read_to_string(blobs.path(&h)).ok()
  };
  let before = {
    let index = s.state().index.lock().expect("index lock");
    grant(&index, &thread, &name, Kind::Build).flatten()
  };
  let mut written = Vec::new();
  for (k, given) in hashes {
    let want = k
      .hash(&c)
      .ok_or_else(|| Error::Config(format!("app \"{name}\" asks for no {} grant", k.word())))?;
    if given.trim() != want {
      return Err(Error::Config(format!(
        "app \"{name}\"'s latest version asks for another {} grant ({want}) than {given}: \
         nothing granted. See it with `stitch app allow-container \"{name}\" --thread {thread}`.",
        k.word()
      )));
    }
  }
  for (k, _) in hashes {
    let want = k.hash(&c).expect("checked");
    crate::meta::set(
      s,
      &thread,
      &k.field(&name),
      json!({"allow": true, "hash": want, "spec": c, "version": manifest.to_string()})
        .to_string()
        .into(),
      author.clone(),
    )?;
    written.push(*k);
  }
  if !written.is_empty() {
    s.apps.bump();
  }
  let head = if written.is_empty() {
    format!("App \"{name}\" (thread {thread}) asks to use containers:")
  } else {
    format!(
      "App \"{name}\" (thread {thread}) is granted {}:",
      written
        .iter()
        .map(|k| k.word())
        .collect::<Vec<_>>()
        .join(", ")
    )
  };
  out.push(head);
  if let Some(e) = engine_said {
    out.push(format!("  HERE: {e}"));
  }
  for k in [Kind::Build, Kind::Run, Kind::Shell] {
    let Some(h) = k.hash(&c) else { continue };
    let lines = match k {
      Kind::Build => spec::build_said(&c),
      Kind::Run => spec::run_said(&c),
      Kind::Shell => spec::shell_said(&c),
    };
    let standing = {
      let index = s.state().index.lock().expect("index lock");
      match grant(&index, &thread, &name, k) {
        None => "not granted".to_owned(),
        Some(None) => "denied".to_owned(),
        Some(Some(g)) if g.hash == h => "granted".to_owned(),
        Some(Some(_)) => "granted for an older version: asks again".to_owned(),
      }
    };
    out.push(format!("{} ({standing}; hash {h}):", k.word()));
    out.extend(lines.into_iter().map(|l| format!("  {l}")));
    if k == Kind::Run
      && let Some(g) = &gpu_said
    {
      out.push(format!("  {g}"));
    }
  }
  // The Containerfile, or its change since the last build grant.
  if let Some(now) = cf_text(&c) {
    match before.as_ref().and_then(|b| cf_text(&b.spec)) {
      Some(old) if old == now => {
        out.push("Containerfile: unchanged since the last build grant.".into())
      }
      Some(old) => {
        out.push("Containerfile, changed since the last build grant:".into());
        let diff = similar::TextDiff::from_lines(&old, &now);
        out.push(
          diff
            .unified_diff()
            .header("granted", "now")
            .to_string()
            .trim_end()
            .to_owned(),
        );
      }
      None => {
        out.push("Containerfile:".into());
        out.extend(now.lines().map(|l| format!("  | {l}")));
      }
    }
  }
  if written.is_empty() {
    let mut cmd = format!("stitch app allow-container \"{name}\" --thread {thread}");
    for k in [Kind::Build, Kind::Run, Kind::Shell] {
      if let Some(h) = k.hash(&c) {
        cmd.push_str(&format!(" --{} {h}", k.word()));
      }
    }
    out.push(format!(
      "Nothing was granted. To grant exactly this (drop what you do not grant): {cmd}"
    ));
  } else if written.contains(&Kind::Build) {
    // The user's act starts the build.
    let s2 = s.clone();
    let owner2 = owner.clone();
    tokio::spawn(async move {
      let r = build::ensure(&s2, &owner2, true).await;
      if let Err(e) = r {
        eprintln!("build of app \"{}\": {e}", owner2.name);
      }
    });
    out.push(
      "The build starts now, in the background (`stitch app storage` shows its image once built)."
        .into(),
    );
  }
  Ok(out)
}

/// What this node says of running `c` (podman missing, refused), and of
/// its GPU commands.
fn engine_here(c: &ContainerNeeds) -> (Option<String>, Option<String>) {
  let e = match engine::Podman::find() {
    None if cfg!(target_os = "linux") => Some("no podman on this node: nothing runs here".into()),
    None => Some("app containers run only on Linux for now: nothing runs here".into()),
    Some(_) => None,
  };
  let gpu = c.commands.values().any(|c| c.gpu).then(|| {
    format!(
      "GPU commands use NVIDIA's CDI with a spec Stitch generates with the system's toolkit \
       (its hooks only {}), toolkit {}.{}.{} or later, one GPU command at a time",
      engine::CDI_HOOKS.join(", "),
      spec::MIN_NVIDIA_CTK.0,
      spec::MIN_NVIDIA_CTK.1,
      spec::MIN_NVIDIA_CTK.2
    )
  });
  (e, gpu)
}

/// The tools an app's thread has for its containers, under the grants
/// standing: `app_command` (its run grant's commands), `app_shell` (its
/// shell grant).
pub(crate) fn definitions(s: &Shared, owner: &Owner) -> Vec<stitch_providers::ToolDef> {
  let mut out = Vec::new();
  if let Some(g) = granted(s, owner, Kind::Run) {
    let mut about = String::from(
      "Run one of the app's declared commands in its container (no network; your files are \
       copied in read-only; what it writes comes back into the app's data folder under out/, \
       images attached to your reply). Commands:",
    );
    for (n, c) in &g.spec.commands {
      about.push_str(&format!(
        "\n- {n}{}: args {}",
        c.description
          .as_ref()
          .map(|d| format!(" ({d})"))
          .unwrap_or_default(),
        if c.args.is_empty() {
          "none".to_owned()
        } else {
          c.args
            .iter()
            .map(|(k, a)| {
              format!(
                "{k}: {}",
                match a {
                  spec::Arg::File { dir, ext } => format!(
                    "a .{} file of the data folder{}",
                    ext.join("/."),
                    if dir.is_empty() {
                      String::new()
                    } else {
                      format!(" under {dir}/")
                    }
                  ),
                  spec::Arg::Out { .. } => "(the output folder: give nothing)".into(),
                  spec::Arg::Enum { values } => format!("one of {}", values.join(", ")),
                  spec::Arg::Int { min, max } => format!("{min} to {max}"),
                  spec::Arg::Slug => "a word".into(),
                }
              )
            })
            .collect::<Vec<_>>()
            .join("; ")
        }
      ));
    }
    out.push(stitch_providers::ToolDef {
      name: "app_command".into(),
      description: about,
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "command": {"type": "string", "enum": g.spec.commands.keys().collect::<Vec<_>>()},
          "args": {"type": "object", "description": "The command's arguments by name: paths relative to the app's data folder, numbers, words."}
        },
        "required": ["command"]
      }),
    });
  }
  if granted(s, owner, Kind::Shell).is_some() {
    out.push(stitch_providers::ToolDef {
      name: "app_shell".into(),
      description: "Run a command line (/bin/sh -c) in the app's container: no network, the image read-only, the data folder's work/ copied in at /in/work, what it writes in /out comes back into the data folder under out/. Each call is a new container: nothing persists but what it writes to /out.".into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {"command": {"type": "string"}},
        "required": ["command"]
      }),
    });
  }
  out
}

/// Runs `app_command` or `app_shell` for an app's thread, its grant's data
/// folder the place of its files: the tool's outcome, images attached.
pub(crate) async fn call(
  s: &Arc<Shared>,
  owner: &Owner,
  needs: &super::threads::ThreadNeeds,
  name: &str,
  input: &Value,
) -> crate::tools::ToolOutcome {
  use crate::tools::ToolOutcome;
  let data = match super::files::ensure(s, needs) {
    Ok(Some(d)) => d,
    Ok(None) => {
      return ToolOutcome::err(format!(
        "app \"{}\" has no data folder for its commands' files",
        owner.name
      ));
    }
    Err(e) => return ToolOutcome::err(e),
  };
  let quota = super::files::quota(needs);
  let r = match name {
    "app_command" => {
      let command = input.get("command").and_then(Value::as_str).unwrap_or("");
      let empty = serde_json::Map::new();
      let args = input
        .get("args")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
      run::run(s, owner, &data, quota, command, args).await
    }
    _ => {
      let line = input.get("command").and_then(Value::as_str).unwrap_or("");
      run::shell(s, owner, &data, quota, line).await
    }
  };
  let o = match r {
    Ok(o) => o,
    Err(e) => return ToolOutcome::err(e),
  };
  let label = if name == "app_command" {
    input
      .get("command")
      .and_then(Value::as_str)
      .unwrap_or("command")
  } else {
    "shell"
  };
  let mut out = if o.exit == Some(0) && o.stopped.is_none() {
    ToolOutcome::ok(run::said(label, &o))
  } else {
    ToolOutcome::err(run::said(label, &o))
  };
  // Images, from the data folder as written, into the blob store as the
  // reply's attachments, within the family's upload quota.
  let imgs = run::images(&o);
  let total: u64 = imgs.iter().map(|m| m.bytes).sum();
  if total > 0
    && let Err(e) = s
      .apps
      .family
      .ledgers
      .upload(owner, total, super::threads::UPLOAD_QUOTA)
  {
    out.output.push_str(&format!("\nImages not attached: {e}"));
    return out;
  }
  for m in imgs {
    let Ok(bytes) = super::files::read(&data, &m.rel, 16 << 20) else {
      continue;
    };
    if let Ok(hash) = s.state().blobs.put_bytes(&bytes) {
      out.generated.push(stitch_records::Attachment {
        hash,
        media_type: m.image.unwrap_or("image/png").into(),
        name: Some(m.rel.rsplit('/').next().unwrap_or("output").to_owned()),
        bytes: bytes.len() as u64,
      });
    }
  }
  out
}
