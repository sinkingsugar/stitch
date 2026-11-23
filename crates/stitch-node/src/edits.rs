//! Editing files without a shell: `write_file` and `edit_file` (exact
//! string replacement), on every node, the phone included, where they are
//! the only way an agent changes a file.
//!
//! Inside the thread's place (its folder, a phone's copy of it, its
//! checkout, or a phone's clone of its project) they run without asking: a
//! folder keeps every version (design §2.16, `stitch folder log`), a
//! checkout and a clone have git. Outside it they ask, as a shell command
//! that writes does, unless yolo is on. A read-only thread refuses them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use stitch_providers::ToolDef;
use stitch_records::RecordId;

use crate::place::expand_home;
use crate::rules::Live;
use crate::tools::{ApprovalRequest, Approver, ToolOutcome};

/// Largest file written or edited.
const MAX_FILE: usize = 10 * 1024 * 1024;

pub(crate) fn definitions() -> Vec<ToolDef> {
  vec![
    ToolDef {
      name: "write_file".into(),
      description: "Write a file whole: created (with its directories) or replaced. For a \
        change to part of a file, edit_file. Inside this thread's place (its folder or \
        checkout) it runs at once; elsewhere the user is asked. A relative path is from where \
        the thread's shell sessions start."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "path": {"type": "string", "description": "The file: absolute, `~/…`, or relative."},
          "content": {"type": "string", "description": "The whole file."}
        },
        "required": ["path", "content"]
      }),
    },
    ToolDef {
      name: "edit_file".into(),
      description: "Change part of a text file: old_string, exactly as it is in the file \
        (indentation included), replaced by new_string. It must occur once, unless replace_all; \
        include enough around it to make it unique. The result shows the change as a diff. \
        Inside this thread's place it runs at once; elsewhere the user is asked."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "path": {"type": "string", "description": "The file: absolute, `~/…`, or relative."},
          "old_string": {"type": "string", "description": "The text to replace, exactly."},
          "new_string": {"type": "string", "description": "What replaces it."},
          "replace_all": {"type": "boolean", "description": "Every occurrence, not just one."}
        },
        "required": ["path", "old_string", "new_string"]
      }),
    },
  ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
  path: String,
  content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
  path: String,
  old_string: String,
  new_string: String,
  #[serde(default)]
  replace_all: bool,
}

/// Where a thread may edit, and how it is asked elsewhere.
#[derive(Clone)]
pub(crate) struct Editing {
  /// Relative paths start here.
  pub cwd: PathBuf,
  /// The thread's place here: edits inside it run at once.
  pub place: Option<PathBuf>,
  /// The project whose clone the place is, on a phone: its changes are
  /// committed with `commit_push`.
  pub clone_of: Option<stitch_records::RepoId>,
  pub read_only: bool,
  /// A read-only room participant's: what the read-only mode refuses
  /// asks the user instead (`live` keeps it read-only; design §2.27).
  pub room_ask: Option<crate::rules::RoomAsk>,
  /// Yolo, as the node's mode says it.
  pub yolo: bool,
  pub live: Live,
  pub approver: Option<Approver>,
  /// The thread's shells' sandbox (the `sandbox` mode): where they write
  /// without asking, these do too.
  pub sandbox: Option<stitch_shell::Sandbox>,
  /// On a phone, the folder's paths on other nodes, naming its copy.
  pub aliases: crate::place::Aliases,
  /// The hazard guard (design §2.28): judges replacing a file outside
  /// the place, and is told what was written (the files a later command
  /// may run).
  pub hazard: Option<Arc<crate::hazard::Guard>>,
}

fn resolve(cwd: &Path, path: &str, aliases: &crate::place::Aliases) -> PathBuf {
  if let Some(p) = aliases.map(path) {
    return p;
  }
  let p = expand_home(path);
  if p.is_absolute() { p } else { cwd.join(p) }
}

/// The file a tool's `path` names, as these tools write it: from `cwd`,
/// through `aliases`, with what exists of it resolved (links, `..`).
pub(crate) fn target(cwd: &Path, path: &str, aliases: &crate::place::Aliases) -> PathBuf {
  real(&resolve(cwd, path, aliases))
}

/// Whether `file` is in a directory beside `base` named `<base>-<name>`
/// (one of its worktrees, by convention).
fn in_sibling(file: &Path, base: &Path) -> bool {
  let base = std::fs::canonicalize(base).unwrap_or_else(|_| base.to_owned());
  let (Some(parent), Some(name)) = (base.parent(), base.file_name()) else {
    return false;
  };
  let prefix = format!("{}-", name.to_string_lossy());
  file
    .strip_prefix(parent)
    .ok()
    .and_then(|rest| rest.components().next())
    .is_some_and(|first| {
      let first = first.as_os_str().to_string_lossy();
      first.len() > prefix.len() && first.starts_with(&prefix)
    })
}

/// A path with what exists of it resolved (links, `..`), so a path cannot
/// seem inside the place while it is not.
fn real(path: &Path) -> PathBuf {
  use std::path::Component;
  // Component by component: what exists is resolved by the file system
  // (links included); a `..` after what does not exist goes up lexically,
  // as creating the missing directories and going back up would. A path
  // never keeps a `..`, so it cannot seem inside a directory it leaves.
  let mut out = PathBuf::new();
  let mut exists = true;
  for c in path.components() {
    match c {
      Component::Prefix(_) | Component::RootDir => out.push(c.as_os_str()),
      Component::CurDir => {}
      Component::ParentDir => {
        out.pop();
      }
      Component::Normal(n) => out.push(n),
    }
    if exists || out.exists() {
      match std::fs::canonicalize(&out) {
        Ok(r) => {
          out = r;
          exists = true;
        }
        Err(_) => exists = false,
      }
    }
  }
  out
}

impl Editing {
  /// Whether `file` may be written: at once, after asking, or not at all.
  async fn may(
    &self,
    thread: &RecordId,
    tool: &str,
    file: &Path,
    change: &str,
  ) -> Result<bool, String> {
    if file.components().any(|c| c.as_os_str() == ".git") {
      return Err("files inside .git are git's, not to be written".into());
    }
    if self.read_only || self.live.get().read_only {
      let (Some(ask), Some(approver)) = (&self.room_ask, &self.approver) else {
        return Err(format!("this thread is read-only: {tool} is refused"));
      };
      // In a room, the user is there: asked, the answer covers this
      // write wherever it is (design §2.27).
      let shown = self
        .place
        .as_ref()
        .and_then(|p| {
          let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
          file.strip_prefix(p).ok().map(Path::to_owned)
        })
        .unwrap_or_else(|| file.to_owned());
      let verb = if tool == "edit_file" { "edit" } else { "write" };
      let what = format!("{verb} {}", shown.display());
      let d = approver(ApprovalRequest::room(thread.clone(), "files", ask, &what)).await;
      return ask.decided(&d, tool).map(|()| true);
    }
    let under =
      |dir: &PathBuf| file.starts_with(std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone()));
    let inside = self.place.as_ref().is_some_and(under)
      || self.sandbox.as_ref().is_some_and(|s| {
        s.writable.iter().any(under) || s.siblings.iter().any(|base| in_sibling(file, base))
      });
    if inside {
      return Ok(false);
    }
    let yolo = (self.yolo || self.live.get().yolo) && !self.read_only && !self.live.get().read_only;
    // Outside the place, replacing a file is judged (design §2.28, auto
    // mode); one that does not exist yet loses nothing. Without a verdict,
    // such a replacement does not happen.
    let existing = std::fs::metadata(file)
      .ok()
      .filter(std::fs::Metadata::is_file)
      .map(|m| m.len());
    let verdict = match (&self.hazard, existing) {
      (Some(g), Some(existing)) => {
        g.check(crate::hazard::Action::Write {
          tool: tool.to_owned(),
          path: file.to_owned(),
          existing,
          change: change.to_owned(),
        })
        .await
      }
      _ => None,
    };
    if let Some(v) = &verdict {
      crate::hazard::record(v);
    }
    if yolo && let Some(v) = verdict.as_ref().filter(|v| v.fails()) {
      return Err(crate::hazard::unavailable(v));
    }
    let guarded = yolo && verdict.as_ref().is_some_and(crate::hazard::Verdict::asks);
    if yolo && !guarded {
      return Ok(false);
    }
    let stopped = verdict
      .as_ref()
      .filter(|_| guarded)
      .map(crate::hazard::declined)
      .unwrap_or_default();
    let Some(approver) = &self.approver else {
      return Err(format!(
        "{} is outside this thread's place, and nobody is here to approve writing it{stopped}",
        file.display()
      ));
    };
    let live = self.live.clone();
    let decision = approver(ApprovalRequest {
      allowed_now: (!guarded)
        .then(|| -> Arc<dyn Fn() -> bool + Send + Sync> { Arc::new(move || live.get().yolo) }),
      verdict: verdict.clone(),
      user_only: guarded,
      ..ApprovalRequest::new(
        thread.clone(),
        "files".into(),
        crate::hazard::question(&format!("{tool} {}", file.display()), verdict.as_ref()),
      )
    })
    .await;
    if decision.allow {
      Ok(true)
    } else {
      Err(match decision.reason {
        Some(r) => format!("the user declined writing {}: {r}{stopped}", file.display()),
        None => format!("the user declined writing {}{stopped}", file.display()),
      })
    }
  }

  /// Tells the guard what was written: a command later this turn may run
  /// it.
  fn wrote(&self, file: &Path) {
    if let Some(g) = &self.hazard {
      g.touched(file.to_owned());
    }
  }

  pub(crate) async fn call(&self, thread: &RecordId, name: &str, input: &Value) -> ToolOutcome {
    let r = match name {
      "write_file" => self.write(thread, input).await,
      "edit_file" => self.edit(thread, input).await,
      _ => Err(format!("no tool {name}")),
    };
    match r {
      Ok(out) => ToolOutcome::ok(out),
      Err(e) => ToolOutcome::err(e),
    }
  }

  async fn write(&self, thread: &RecordId, input: &Value) -> Result<String, String> {
    let w: Write = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    if w.content.len() > MAX_FILE {
      return Err(format!(
        "{} bytes: more than this tool writes",
        w.content.len()
      ));
    }
    let file = real(&resolve(&self.cwd, &w.path, &self.aliases));
    let asked = self.may(thread, "write_file", &file, &w.content).await?;
    let existed = file.exists();
    save(&file, w.content.as_bytes()).map_err(|e| format!("{}: {e}", file.display()))?;
    self.wrote(&file);
    Ok(format!(
      "{}{} {} ({} bytes)",
      if asked {
        "(approved by the user)\n"
      } else {
        ""
      },
      if existed { "replaced" } else { "created" },
      file.display(),
      w.content.len()
    ))
  }

  async fn edit(&self, thread: &RecordId, input: &Value) -> Result<String, String> {
    let e: Edit = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
    if e.old_string.is_empty() {
      return Err("old_string is empty: to write a file whole, write_file".into());
    }
    if e.old_string == e.new_string {
      return Err("old_string and new_string are the same: nothing would change".into());
    }
    let file = real(&resolve(&self.cwd, &e.path, &self.aliases));
    let text =
      std::fs::read_to_string(&file).map_err(|err| format!("{}: {err}", file.display()))?;
    let n = text.matches(&e.old_string).count();
    match n {
      0 => {
        return Err(format!(
          "old_string is not in {}: it must match exactly, whitespace and indentation included",
          file.display()
        ));
      }
      1 => {}
      _ if !e.replace_all => {
        return Err(format!(
          "old_string occurs {n} times in {}: include more around it to make it unique, or \
           set replace_all",
          file.display()
        ));
      }
      _ => {}
    }
    let new = if e.replace_all {
      text.replace(&e.old_string, &e.new_string)
    } else {
      text.replacen(&e.old_string, &e.new_string, 1)
    };
    if new.len() > MAX_FILE {
      return Err(format!("{} bytes: more than this tool writes", new.len()));
    }
    let change = format!(
      "replacing{}:\n{}\nwith:\n{}",
      if e.replace_all { " every" } else { "" },
      e.old_string,
      e.new_string
    );
    let asked = self.may(thread, "edit_file", &file, &change).await?;
    save(&file, new.as_bytes()).map_err(|err| format!("{}: {err}", file.display()))?;
    self.wrote(&file);
    let diff = similar::TextDiff::from_lines(&text, &new)
      .unified_diff()
      .context_radius(2)
      .to_string();
    let diff: String = if diff.len() > 4000 {
      format!(
        "{}\n[… diff cut]",
        diff.chars().take(4000).collect::<String>()
      )
    } else {
      diff
    };
    Ok(format!(
      "{}edited {} ({} replacement{})\n{diff}",
      if asked {
        "(approved by the user)\n"
      } else {
        ""
      },
      file.display(),
      n.min(if e.replace_all { n } else { 1 }),
      if e.replace_all && n > 1 { "s" } else { "" }
    ))
  }
}

/// Writes a file whole, through a temporary file beside it, keeping its
/// permissions when it was there.
fn save(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
  let parent = file
    .parent()
    .ok_or_else(|| std::io::Error::other("no directory"))?;
  std::fs::create_dir_all(parent)?;
  let perms = std::fs::metadata(file).ok().map(|m| m.permissions());
  let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
  std::io::Write::write_all(&mut tmp, bytes)?;
  if let Some(p) = perms {
    std::fs::set_permissions(tmp.path(), p)?;
  }
  tmp.persist(file).map_err(|e| e.error)?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::tools::Decision;

  fn editing(place: &Path, approve: Option<bool>) -> Editing {
    Editing {
      cwd: place.to_owned(),
      place: Some(place.to_owned()),
      clone_of: None,
      read_only: false,
      room_ask: None,
      yolo: false,
      aliases: crate::place::Aliases::default(),
      live: Live::default(),
      approver: approve.map(|allow| -> Approver {
        Arc::new(move |_| {
          Box::pin(async move {
            Decision {
              allow,
              ..Decision::default()
            }
          })
        })
      }),
      sandbox: None,
      hazard: None,
    }
  }

  fn thread() -> RecordId {
    "2026-10-14T100000Z-mac-abcd".parse().unwrap()
  }

  #[tokio::test]
  async fn inside_the_place_edits_run_at_once() {
    let d = tempfile::tempdir().unwrap();
    let e = editing(d.path(), None);
    let out = e
      .call(
        &thread(),
        "write_file",
        &json!({"path": "notes/a.md", "content": "one\ntwo\n"}),
      )
      .await;
    assert!(out.ok, "{}", out.output);
    let out = e
      .call(
        &thread(),
        "edit_file",
        &json!({"path": "notes/a.md", "old_string": "two", "new_string": "TWO"}),
      )
      .await;
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("+TWO"), "{}", out.output);
    assert_eq!(
      std::fs::read_to_string(d.path().join("notes/a.md")).unwrap(),
      "one\nTWO\n"
    );
    // Not there, or more than once: said, nothing written.
    let out = e
      .call(
        &thread(),
        "edit_file",
        &json!({"path": "notes/a.md", "old_string": "three", "new_string": "x"}),
      )
      .await;
    assert!(!out.ok && out.output.contains("not in"));
    e.call(
      &thread(),
      "write_file",
      &json!({"path": "b.md", "content": "x x\n"}),
    )
    .await;
    let out = e
      .call(
        &thread(),
        "edit_file",
        &json!({"path": "b.md", "old_string": "x", "new_string": "y"}),
      )
      .await;
    assert!(!out.ok && out.output.contains("2 times"));
    let out = e
      .call(
        &thread(),
        "edit_file",
        &json!({"path": "b.md", "old_string": "x", "new_string": "y", "replace_all": true}),
      )
      .await;
    assert!(out.ok);
    assert_eq!(
      std::fs::read_to_string(d.path().join("b.md")).unwrap(),
      "y y\n"
    );
  }

  #[tokio::test]
  async fn outside_the_place_the_user_is_asked() {
    let d = tempfile::tempdir().unwrap();
    let place = d.path().join("place");
    std::fs::create_dir_all(&place).unwrap();
    let outside = d.path().join("outside.md");
    let input = json!({"path": outside.display().to_string(), "content": "x"});
    let out = editing(&place, None)
      .call(&thread(), "write_file", &input)
      .await;
    assert!(!out.ok && out.output.contains("nobody is here"));
    let out = editing(&place, Some(false))
      .call(&thread(), "write_file", &input)
      .await;
    assert!(!out.ok && out.output.contains("declined"));
    assert!(!outside.exists());
    let out = editing(&place, Some(true))
      .call(&thread(), "write_file", &input)
      .await;
    assert!(out.ok && out.output.starts_with("(approved by the user)"));
    // `..` does not make a path inside, after a directory that is not
    // there either.
    for sneaky in [
      "../sneaky.md",
      "n/../../sneaky.md",
      "n/m/../../../sneaky.md",
    ] {
      let out = editing(&place, None)
        .call(
          &thread(),
          "write_file",
          &json!({"path": sneaky, "content": "x"}),
        )
        .await;
      assert!(!out.ok, "{sneaky}: {}", out.output);
      assert!(!d.path().join("sneaky.md").exists(), "{sneaky}");
    }
    assert_eq!(
      target(&place, "n/../../sneaky.md", &Default::default()),
      std::fs::canonicalize(d.path()).unwrap().join("sneaky.md")
    );
  }

  #[tokio::test]
  async fn in_the_sandbox_its_worktrees_and_directories_are_written_at_once() {
    let d = tempfile::tempdir().unwrap();
    let place = d.path().join("proj");
    let worktree = d.path().join("proj-feature");
    let cache = d.path().join("cache");
    for dir in [&place, &worktree, &cache] {
      std::fs::create_dir_all(dir).unwrap();
    }
    let mut e = editing(&place, None);
    e.sandbox = Some(stitch_shell::Sandbox {
      writable: vec![place.clone(), cache.clone()],
      siblings: vec![place.clone()],
      ..Default::default()
    });
    for file in [worktree.join("a.md"), cache.join("b.md")] {
      let input = json!({"path": file.display().to_string(), "content": "x"});
      let out = e.call(&thread(), "write_file", &input).await;
      assert!(out.ok, "{}: {}", file.display(), out.output);
    }
    // A sibling of another name is outside: asked, and nobody is here.
    let other = d.path().join("projx.md");
    let input = json!({"path": other.display().to_string(), "content": "x"});
    let out = e.call(&thread(), "write_file", &input).await;
    assert!(
      !out.ok && out.output.contains("nobody is here"),
      "{}",
      out.output
    );
  }

  #[tokio::test]
  async fn read_only_threads_and_git_are_refused() {
    let d = tempfile::tempdir().unwrap();
    let input = json!({"path": "a.md", "content": "x"});
    let mut e = editing(d.path(), Some(true));
    e.read_only = true;
    assert!(!e.call(&thread(), "write_file", &input).await.ok);
    let e = editing(d.path(), Some(true));
    let out = e
      .call(
        &thread(),
        "write_file",
        &json!({"path": ".git/config", "content": "x"}),
      )
      .await;
    assert!(!out.ok);
  }

  #[tokio::test]
  async fn the_guard_judges_replacing_a_file_outside_the_place_in_auto() {
    use crate::hazard::Mode;
    let place = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let keys = outside.path().join("authorized_keys");
    std::fs::write(&keys, "ssh-ed25519 AAAA… me").unwrap();
    // Declines: what asks is refused.
    let mut e = editing(place.path(), Some(false));
    e.yolo = true;
    let (g, fake) = crate::hazard::tests::guard(Mode::On, |input| {
      Ok(if input.contains("authorized_keys") {
        0.9
      } else {
        0.1
      })
    });
    e.hazard = Some(g.clone());
    let input = json!({"path": keys.display().to_string(), "content": ""});
    let out = e.call(&thread(), "write_file", &input).await;
    assert!(
      !out.ok && out.output.contains("hazard guard"),
      "{}",
      out.output
    );
    assert_eq!(
      std::fs::read_to_string(&keys).unwrap(),
      "ssh-ed25519 AAAA… me"
    );
    // A new file outside, and anything inside the place: not judged, and
    // what is written is remembered for the commands that may run it.
    let new = outside.path().join("new.txt");
    let input = json!({"path": new.display().to_string(), "content": "x"});
    assert!(e.call(&thread(), "write_file", &input).await.ok);
    let input = json!({"path": "run.sh", "content": "rm -rf ~/devel"});
    assert!(e.call(&thread(), "write_file", &input).await.ok);
    assert_eq!(fake.inputs.lock().unwrap().len(), 1);
    let v = g
      .check(crate::hazard::Action::Shell {
        line: "just go".into(),
        cwd: Some(place.path().to_owned()),
        host: None,
      })
      .await
      .unwrap();
    // What it wrote in the place is read; outside it, said and not read.
    assert_eq!(v.files, ["run.sh"]);
    assert!(
      v.notes
        .iter()
        .any(|n| n.contains("new.txt") && n.contains("outside")),
      "{:?}",
      v.notes
    );
  }
}
