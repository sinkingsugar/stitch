//! What a thread is told besides its own instructions (design §2.3): the
//! `AGENTS.md` files that apply where it works, and the skills it can use.
//!
//! - **Instructions:** `AGENTS.md` at the top of the home workspace (the
//!   user's own, on this node), then the user's own as records, synced to
//!   every node (`crate::instructions`: the global ones, then the place's),
//!   then the place's files: from the top of
//!   the project's checkout, or the folder, down to where the thread
//!   started, one per directory. A directory without `AGENTS.md` has its
//!   `CLAUDE.md` read instead, with its `@file` imports; user-level files of
//!   other agents (`~/.claude/CLAUDE.md`) are not read.
//! - **Skills:** `skills/<name>/SKILL.md` in the home workspace, listed by
//!   name and description; the agent reads one with the shell when a task
//!   fits it. Other agents' skills are not used: they name tools Stitch
//!   does not have.
//!
//! A thread's system prompt holds them as they were when it started; when
//! they change, the next message carries them (a `context` part).

use std::path::{Path, PathBuf};

/// Each instruction file at most, in bytes; longer ones are cut.
const MAX_FILE: usize = 64 * 1024;

/// A skill: a `SKILL.md` with a `name` and a `description`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
  pub name: String,
  pub description: String,
  pub path: PathBuf,
  /// The node whose files it is in, when not this one's: it is read with
  /// `read_skill` (a place's skill, from its record, design §2.14).
  pub from: Option<String>,
  /// Its tools are lazy (design §2.17): not in the tool list, called with
  /// `Tool.Call` in `otter_run` (`tools: lazy` in its front matter; a
  /// built-in's `BuiltInSkill::lazy`).
  pub lazy: bool,
  /// Its lazy tools here (none when the `otter` skill is off: then they
  /// are listed), named in its line.
  pub lazy_tools: Vec<String>,
}

/// Whether a SKILL.md's front matter makes its tools lazy: `tools: lazy`
/// (`tools: listed`, or no key, keeps them in the tool list).
pub fn lazy_tools(text: &str) -> bool {
  front_matter_key(text, "tools").is_some_and(|v| v.eq_ignore_ascii_case("lazy"))
}

/// The skills in `skills/` of the home workspace, by name.
pub fn skills(home: &Path) -> Vec<Skill> {
  skills_in(&home.join("skills"))
}

/// The skills in `dir` (`<name>/SKILL.md`), by name.
pub fn skills_in(dir: &Path) -> Vec<Skill> {
  let Ok(entries) = std::fs::read_dir(dir) else {
    return Vec::new();
  };
  let mut out: Vec<Skill> = entries
    .flatten()
    .filter_map(|e| {
      let path = e.path().join("SKILL.md");
      let text = std::fs::read_to_string(&path).ok()?;
      let (name, description) = front_matter(&text);
      Some(Skill {
        name: name.unwrap_or_else(|| e.file_name().to_string_lossy().into_owned()),
        description: description.unwrap_or_default(),
        path,
        from: None,
        lazy: lazy_tools(&text),
        lazy_tools: Vec::new(),
      })
    })
    .collect();
  out.sort_by(|a, b| a.name.cmp(&b.name));
  out
}

/// `name` and `description` from a `---` front matter: plain or quoted
/// values, or a folded block (`>` or `|`) on the lines below.
fn front_matter(text: &str) -> (Option<String>, Option<String>) {
  (
    front_matter_key(text, "name"),
    front_matter_key(text, "description"),
  )
}

/// One key of a `---` front matter.
fn front_matter_key(text: &str, key: &str) -> Option<String> {
  let mut lines = text.lines();
  if lines.next().map(str::trim) != Some("---") {
    return None;
  }
  let body: Vec<&str> = lines.take_while(|l| l.trim() != "---").collect();
  let get = |key: &str| -> Option<String> {
    let i = body
      .iter()
      .position(|l| l.strip_prefix(key).is_some_and(|r| r.starts_with(':')))?;
    let value = body[i][key.len() + 1..].trim();
    let value = if value.is_empty() || value.starts_with('>') || value.starts_with('|') {
      body[i + 1..]
        .iter()
        .take_while(|l| l.starts_with(' ') || l.starts_with('\t') || l.is_empty())
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned()
    } else {
      value.trim_matches(|c| c == '"' || c == '\'').to_owned()
    };
    (!value.is_empty()).then_some(value)
  };
  get(key)
}

/// The instructions that apply, as (label, text): the home workspace's
/// file, the user's own (`yours`, `crate::instructions`), then the place's
/// files from `top` down to `start`.
pub fn instructions(
  home: Option<&Path>,
  yours: Vec<(String, String)>,
  top: Option<&Path>,
  start: Option<&Path>,
) -> Vec<(String, String)> {
  let mut out = Vec::new();
  if let Some(text) = home.and_then(|h| read_capped(&h.join("AGENTS.md"))) {
    out.push(("Yours (AGENTS.md in the home workspace)".to_owned(), text));
  }
  out.extend(yours);
  let Some(top) = top else {
    return out;
  };
  let mut dirs = vec![top.to_owned()];
  if let Some(start) = start.filter(|s| s.starts_with(top))
    && let Ok(rest) = start.strip_prefix(top)
  {
    let mut d = top.to_owned();
    for c in rest.components() {
      d.push(c);
      dirs.push(d.clone());
    }
  }
  for dir in dirs {
    let rel = dir.strip_prefix(top).unwrap_or(&dir);
    let label = |name: &str| {
      if rel.as_os_str().is_empty() {
        name.to_owned()
      } else {
        format!("{}/{name}", rel.display())
      }
    };
    if let Some(text) = read_capped(&dir.join("AGENTS.md")) {
      out.push((label("AGENTS.md"), text));
    } else if let Some(text) = read_capped(&dir.join("CLAUDE.md")) {
      out.push((label("CLAUDE.md"), imports(&text, &dir, top)));
    }
  }
  out
}

fn read_capped(path: &Path) -> Option<String> {
  let text = std::fs::read_to_string(path).ok()?;
  if text.trim().is_empty() {
    return None;
  }
  if text.len() <= MAX_FILE {
    return Some(text);
  }
  let mut end = MAX_FILE;
  while !text.is_char_boundary(end) {
    end -= 1;
  }
  Some(format!("{}\n\n[… cut at {MAX_FILE} bytes]", &text[..end]))
}

/// A `CLAUDE.md` line that is only `@path` brings that file in, when it is
/// inside the checkout or folder.
fn imports(text: &str, dir: &Path, top: &Path) -> String {
  text
    .lines()
    .map(|l| {
      let Some(rel) = l.trim().strip_prefix('@').filter(|r| !r.contains(' ')) else {
        return l.to_owned();
      };
      let path = dir.join(rel);
      let inside = std::fs::canonicalize(&path).is_ok_and(|p| p.starts_with(top));
      match read_capped(&path).filter(|_| inside) {
        Some(t) => t,
        None => l.to_owned(),
      }
    })
    .collect::<Vec<_>>()
    .join("\n")
}

/// The context section of a thread's prompt, with the skills it has and
/// what to say about its tools; empty when there is nothing.
pub fn render(
  home: Option<&Path>,
  yours: Vec<(String, String)>,
  top: Option<&Path>,
  start: Option<&Path>,
  skills: &[Skill],
  tools: &[String],
) -> String {
  let mut out = String::new();
  let files = instructions(home, yours, top, start);
  if !files.is_empty() {
    out.push_str(
      "# Instructions\n\nThe user's and the project's instruction files, from the most general \
       to the most specific: follow them, the later ones where they disagree.\n",
    );
    for (label, text) in files {
      out.push_str(&format!("\n## {label}\n\n{}\n", text.trim_end()));
    }
  }
  if !skills.is_empty() {
    if !out.is_empty() {
      out.push('\n');
    }
    out.push_str(
      "# Skills\n\nInstructions for particular tasks. When a task fits one, read its SKILL.md \
       with the shell (`cat`) before starting, and follow it; files it mentions are next to it.",
    );
    if skills.iter().any(|s| s.from.is_some()) {
      out.push_str(
        " One built in, or whose files are on another machine, is read with `read_skill`.",
      );
    }
    out.push_str("\n\n");
    for s in skills {
      let at = match &s.from {
        Some(node) if node == crate::apps::BUILT_IN => "built in: read_skill".into(),
        Some(node) => format!("on {node}: read_skill"),
        None => s.path.display().to_string(),
      };
      let lazy = if s.lazy_tools.is_empty() {
        String::new()
      } else {
        format!(
          "; its tools {}, called with Tool.Call in otter_run: read_skill gives their parameters",
          s.lazy_tools.join(", ")
        )
      };
      out.push_str(&format!("- {}: {} ({at}{lazy})\n", s.name, s.description));
    }
  }
  if !tools.is_empty() {
    if !out.is_empty() {
      out.push('\n');
    }
    out.push_str("# Tools\n\n");
    for t in tools {
      out.push_str(&format!("- {t}\n"));
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn instructions_and_skills_are_gathered() {
    let dir = tempfile::tempdir().unwrap();
    let d = std::fs::canonicalize(dir.path()).unwrap();
    let home = d.join("home");
    let top = d.join("app");
    let deep = top.join("crates/core");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::create_dir_all(home.join("skills/release")).unwrap();
    std::fs::create_dir_all(home.join("skills/notes")).unwrap();
    std::fs::write(home.join("AGENTS.md"), "Be brief.\n").unwrap();
    std::fs::write(top.join("CLAUDE.md"), "# App\n@docs.md\n@../outside.md\n").unwrap();
    std::fs::write(top.join("docs.md"), "Run the tests.").unwrap();
    std::fs::write(d.join("outside.md"), "secret").unwrap();
    std::fs::write(deep.join("AGENTS.md"), "No unsafe here.").unwrap();
    std::fs::write(deep.join("CLAUDE.md"), "Ignored: AGENTS.md wins.").unwrap();
    std::fs::write(
      home.join("skills/release/SKILL.md"),
      "---\nname: release\ndescription: \"Cut a release: tag, build, publish.\"\n---\nSteps…\n",
    )
    .unwrap();
    std::fs::write(
      home.join("skills/notes/SKILL.md"),
      "---\nname: notes\ndescription: >\n  Keep meeting notes\n  in ~/notes.\n---\n",
    )
    .unwrap();

    let yours = vec![
      (
        "Yours, for every thread".to_owned(),
        "Speak plainly.".to_owned(),
      ),
      (
        "Yours, for app".to_owned(),
        "Ask before pushing.".to_owned(),
      ),
    ];
    let files = instructions(Some(&home), yours.clone(), Some(&top), Some(&deep));
    let labels: Vec<&str> = files.iter().map(|(l, _)| l.as_str()).collect();
    // The most general first: the user's own before the checkout's.
    assert_eq!(
      labels,
      [
        "Yours (AGENTS.md in the home workspace)",
        "Yours, for every thread",
        "Yours, for app",
        "CLAUDE.md",
        "crates/core/AGENTS.md"
      ]
    );
    // Imports inside the checkout come in; others stay as written.
    assert!(files[3].1.contains("Run the tests."));
    assert!(files[3].1.contains("@../outside.md"));
    assert!(!files[3].1.contains("secret"));

    let found = skills(&home);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].name, "notes");
    assert_eq!(found[0].description, "Keep meeting notes in ~/notes.");
    assert_eq!(found[1].description, "Cut a release: tag, build, publish.");

    let text = render(Some(&home), yours, Some(&top), Some(&deep), &found, &[]);
    assert!(text.starts_with("# Instructions"));
    assert!(text.contains("## crates/core/AGENTS.md\n\nNo unsafe here."));
    assert!(text.contains("## Yours, for app\n\nAsk before pushing."));
    assert!(text.contains("- release: Cut a release: tag, build, publish. ("));
    // Lazy tools: from the front matter; named in the skill's line.
    assert!(!found[0].lazy && !found[1].lazy);
    assert!(lazy_tools("---\nname: x\ntools: lazy\n---\n"));
    assert!(lazy_tools("---\ntools: \"Lazy\"\n---\n"));
    assert!(!lazy_tools("---\ntools: listed\n---\n"));
    assert!(!lazy_tools("tools: lazy\n"));
    let mut lazy = found.clone();
    lazy[1].lazy_tools = vec!["tag".into(), "publish".into()];
    let text = render(None, Vec::new(), None, None, &lazy, &[]);
    assert!(
      text.contains("/release/SKILL.md; its tools tag, publish, called with Tool.Call in otter_run: read_skill gives their parameters)\n"),
      "{text}"
    );
    // Nothing anywhere: nothing said.
    assert_eq!(render(None, Vec::new(), None, None, &[], &[]), "");
  }
}
