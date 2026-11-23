//! What the hazard guard decides without a model (design §2.28): the
//! hard stops, a short conservative list of lines that ask whatever the
//! model says (and when it cannot be asked), and the cheap skip, lines
//! that only read and never reach the model.

use std::path::{Path, PathBuf};

use crate::rules::{Rules, guard_commands, guard_has_option};

/// Whether a line only reads and runs nothing else, so the model is not
/// asked (design §2.28): the built-in read-only commands, in their reading
/// forms, redirected nowhere but `/dev/null` (`Rules::allows` with the
/// defaults alone: none of the user's prefixes or grants, no yolo); and,
/// execution-aware, none of what can make a reading command run something:
/// an environment assignment (`LD_PRELOAD=…`, `GIT_EXTERNAL_DIFF=…`,
/// `PAGER=…`), a wrapper that runs another program (`env`, `xargs`,
/// `exec`, `command`, `timeout`, `nice`, `find -exec`), git (its
/// configuration runs helpers: fsmonitor, pagers, external diffs,
/// textconv), a shell function or alias defined on the line. When unsure,
/// it is scored. A function defined in the session earlier is beyond it:
/// the guard is mistake detection, the sandbox the boundary.
pub(crate) fn reads_only(line: &str) -> bool {
  if !Rules::default().allows(line) {
    return false;
  }
  if line.contains("()") || line.contains("alias ") || line.contains("function ") {
    return false;
  }
  let Some(raw) = crate::rules::raw_commands(line) else {
    return false;
  };
  raw.iter().all(|words| {
    let first = words.first().map(String::as_str).unwrap_or("");
    let program = first.rsplit('/').next().unwrap_or(first);
    !first.contains('=')
      && !matches!(
        program,
        "env"
          | "xargs"
          | "exec"
          | "command"
          | "builtin"
          | "timeout"
          | "nice"
          | "nohup"
          | "stdbuf"
          | "time"
          | "watch"
          | "sudo"
          | "doas"
          | "git"
          | "eval"
          | "source"
          | "."
      )
      && !words
        .iter()
        .any(|w| matches!(w.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir"))
  })
}

/// Directories whose whole removal is never routine.
const SYSTEM: &[&str] = &[
  "/",
  "/bin",
  "/boot",
  "/dev",
  "/etc",
  "/home",
  "/lib",
  "/lib64",
  "/opt",
  "/proc",
  "/root",
  "/sbin",
  "/srv",
  "/sys",
  "/usr",
  "/var",
  "/Users",
  "/System",
  "/Applications",
  "/Library",
  "/private",
  "/Volumes",
];

/// Why a line must ask whatever the model says, or `None`. `cwd` is
/// where it runs (a `cd` in the line moves it), `home` the user's home.
pub(crate) fn hard_stop(line: &str, cwd: Option<&Path>, home: Option<&Path>) -> Option<String> {
  if let Some(why) = raw_device_write(line) {
    return Some(why);
  }
  let mut cwd = cwd.map(Path::to_owned);
  for words in guard_commands(line) {
    if let Some(why) = command_stop(&words, &mut cwd, home, 0) {
      return Some(why);
    }
  }
  None
}

/// A redirection onto a disk device (`> /dev/sda`).
fn raw_device_write(line: &str) -> Option<String> {
  let mut rest = line;
  while let Some(i) = rest.find('>') {
    let after = rest[i + 1..].trim_start_matches(['>', '|', ' ', '\t']);
    if let Some(dev) = after.strip_prefix("/dev/")
      && is_disk(dev.split(|c: char| c.is_whitespace() || c == ';').next()?)
    {
      return Some("writes straight onto a disk device".into());
    }
    rest = &rest[i + 1..];
  }
  None
}

/// A disk (not `null`, `zero`, a terminal or a descriptor).
fn is_disk(dev: &str) -> bool {
  [
    "sd", "nvme", "disk", "rdisk", "hd", "vd", "xvd", "mmcblk", "md", "dm-", "mapper/",
  ]
  .iter()
  .any(|p| dev.starts_with(p))
}

fn command_stop(
  words: &[String],
  cwd: &mut Option<PathBuf>,
  home: Option<&Path>,
  depth: usize,
) -> Option<String> {
  // What `sudo` or `doas` runs is the command.
  let mut words = words;
  while let Some(first) = words.first()
    && matches!(first.as_str(), "sudo" | "doas")
  {
    let mut i = 1;
    while let Some(w) = words.get(i)
      && w.starts_with('-')
    {
      // Options taking a value.
      i += if matches!(w.as_str(), "-u" | "-g" | "-C" | "-h" | "-p" | "-U") {
        2
      } else {
        1
      };
    }
    words = &words[i.min(words.len())..];
  }
  let program = words.first()?.rsplit('/').next().unwrap_or_default();
  let sub = words
    .iter()
    .skip(1)
    .find(|w| !w.starts_with('-'))
    .map(String::as_str)
    .unwrap_or("");
  let has = |opts: &[&str]| guard_has_option(words, opts);
  let operands = || words.iter().skip(1).filter(|w| !w.starts_with('-'));
  match program {
    "cd" => {
      let to = words.get(1).map_or("~", String::as_str);
      *cwd = moved(cwd.as_deref(), to, home);
      None
    }
    "bash" | "sh" | "zsh" | "dash" if depth < 3 => {
      let i = words.iter().position(|w| w == "-c")?;
      let script = words.get(i + 1)?;
      let mut inner = cwd.clone();
      guard_commands(script)
        .iter()
        .find_map(|c| command_stop(c, &mut inner, home, depth + 1))
    }
    "rm" if has(&["--no-preserve-root"]) => Some("rm with --no-preserve-root".into()),
    "rm" if has(&["-r", "-R", "--recursive"]) => operands()
      .find_map(|t| whole(t, cwd.as_deref(), home))
      .map(|what| format!("rm -r of {what}")),
    "find" => {
      let deletes = words.iter().any(|w| w == "-delete")
        || words.windows(2).any(|w| {
          matches!(w[0].as_str(), "-exec" | "-execdir" | "-ok")
            && matches!(w[1].rsplit('/').next(), Some("rm" | "shred" | "unlink"))
        });
      if !deletes {
        return None;
      }
      // Its starting points: the words before the first expression.
      words
        .iter()
        .skip(1)
        .take_while(|w| !w.starts_with('-') && *w != "(" && *w != "!")
        .find_map(|t| whole(t, cwd.as_deref(), home))
        .map(|what| format!("find deleting from {what}"))
    }
    "dd" => words
      .iter()
      .find_map(|w| w.strip_prefix("of=/dev/"))
      .filter(|d| is_disk(d))
      .map(|d| format!("dd onto the device /dev/{d}")),
    p if p.starts_with("mkfs") || p == "wipefs" => Some(format!("{p} formats or wipes a disk")),
    "fdisk" | "sfdisk" | "sgdisk" | "parted" | "gdisk" | "diskutil"
      if words.iter().any(|w| {
        w.starts_with("/dev/")
          || matches!(
            w.as_str(),
            "eraseDisk" | "eraseVolume" | "partitionDisk" | "zeroDisk" | "secureErase"
          )
      }) && !has(&["-l", "--list"])
        && !matches!(sub, "list" | "info") =>
    {
      Some(format!("{p} changes a disk's partitions", p = program))
    }
    "chmod" | "chown" | "chgrp" if has(&["-R", "--recursive"]) => operands()
      .skip(1)
      .find_map(|t| whole(t, cwd.as_deref(), home))
      .map(|what| format!("{program} -R of {what}")),
    "git" if sub == "push" => force_to_main(words),
    "gh" if sub == "repo" && words.iter().any(|w| w == "delete") => Some("gh repo delete".into()),
    "gh"
      if sub == "api"
        && words.windows(2).any(|w| {
          matches!(w[0].as_str(), "-X" | "--method") && w[1].eq_ignore_ascii_case("delete")
        }) =>
    {
      Some("a DELETE through the GitHub API".into())
    }
    _ => None,
  }
}

/// Where `cd <to>` leads from `cwd`.
fn moved(cwd: Option<&Path>, to: &str, home: Option<&Path>) -> Option<PathBuf> {
  if to == "~" || to.is_empty() || to == "$HOME" {
    return home.map(Path::to_owned);
  }
  if let Some(rest) = to.strip_prefix("~/").or_else(|| to.strip_prefix("$HOME/")) {
    return home.map(|h| h.join(rest));
  }
  if to.starts_with('/') {
    return Some(super::attach::lexical(Path::new(to)));
  }
  if to == "-" || to.contains('$') {
    return None;
  }
  cwd.map(|c| super::attach::lexical(&c.join(to)))
}

/// What a deletion of `target` takes when it is a whole tree no task
/// deletes: the root, a system directory, the home, the place it runs in
/// when that is one of those (`*`, `.`), or a path that is only a
/// variable (empty or unset, it is `/` or the current directory).
fn whole(target: &str, cwd: Option<&Path>, home: Option<&Path>) -> Option<String> {
  let t = target.trim();
  // The path it names, `.` and `..` and doubled slashes taken out
  // (`/home/u/../u`, `../..` from a directory below the home).
  if !t.contains(['$', '*', '?', '[']) {
    let named = if let Some(rest) = t.strip_prefix("~/") {
      home.map(|h| h.join(rest))
    } else if t == "~" {
      home.map(Path::to_owned)
    } else if t.starts_with('/') {
      Some(PathBuf::from(t))
    } else {
      cwd.map(|c| c.join(t))
    };
    if let Some(p) = named.map(|p| super::attach::lexical(&p)) {
      let s = p.to_string_lossy();
      let s = if s.is_empty() { "/".into() } else { s };
      if home.is_some_and(|h| super::attach::lexical(h) == p) {
        return Some(format!("`{target}`: the home directory"));
      }
      if SYSTEM.contains(&s.as_ref()) {
        return Some(format!("`{target}`: {s}"));
      }
    }
  }
  // `$HOME` and `${HOME}` are the home; any other variable may be empty.
  let homed = t
    .replace("${HOME}", "~")
    .replace("$HOME", "~")
    .replace("${HOME:?}", "~");
  let had_var = homed.contains('$');
  let bare = strip_vars(&homed);
  let trimmed = bare.trim_end_matches(['*', '.']).trim_end_matches('/');
  let trimmed = if trimmed.is_empty() && bare.starts_with('/') {
    "/"
  } else {
    trimmed
  };
  if had_var && matches!(trimmed, "" | "/") {
    return Some(format!(
      "`{target}`, which is all of {} when the variable is empty or unset",
      if trimmed == "/" {
        "/"
      } else {
        "the current directory"
      }
    ));
  }
  if matches!(trimmed, "~" | "") && homed.starts_with('~') {
    return Some(format!("`{target}`: the home directory"));
  }
  if let Some(home) = home
    && Path::new(trimmed) == home
  {
    return Some(format!("`{target}`: the home directory"));
  }
  if SYSTEM.contains(&trimmed) {
    return Some(format!("`{target}`"));
  }
  // `*`, `.`, `./*`: where it runs.
  if trimmed.is_empty() && !t.is_empty() {
    let here = cwd?;
    if here == Path::new("/") || home.is_some_and(|h| here == h) {
      return Some(format!("`{target}` in {}", here.display()));
    }
    if let Some(s) = here.to_str()
      && SYSTEM.contains(&s)
    {
      return Some(format!("`{target}` in {s}"));
    }
  }
  None
}

/// `t` without its variables (`$X`, `${X}`, `${X:-y}`).
fn strip_vars(t: &str) -> String {
  let mut out = String::new();
  let mut chars = t.chars().peekable();
  while let Some(c) = chars.next() {
    if c != '$' {
      out.push(c);
      continue;
    }
    if chars.peek() == Some(&'{') {
      for c in chars.by_ref() {
        if c == '}' {
          break;
        }
      }
    } else {
      while chars
        .peek()
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
      {
        chars.next();
      }
    }
  }
  out
}

/// A forced push (or a deletion) that reaches `main` or `master`, or a
/// mirror push.
fn force_to_main(words: &[String]) -> Option<String> {
  let force = guard_has_option(words, &["-f", "--force", "--force-with-lease"]);
  let delete = guard_has_option(words, &["-d", "--delete"]);
  if words.iter().any(|w| w == "--mirror") {
    return Some("git push --mirror".into());
  }
  // The words after `push` that are not options: the remote, then refspecs.
  let at = words.iter().position(|w| w == "push")?;
  let refspecs: Vec<&String> = words[at + 1..]
    .iter()
    .filter(|w| !w.starts_with('-'))
    .skip(1)
    .collect();
  let main = |r: &str| {
    let dst = r.rsplit(':').next().unwrap_or(r);
    let dst = dst.trim_start_matches('+');
    let dst = dst.strip_prefix("refs/heads/").unwrap_or(dst);
    matches!(dst, "main" | "master")
  };
  let forced_main = refspecs
    .iter()
    .any(|r| main(r) && (force || delete || r.starts_with('+') || r.starts_with(':')));
  if forced_main {
    return Some(if delete || refspecs.iter().any(|r| r.starts_with(':')) {
      "git push deleting main or master".into()
    } else {
      "git push --force to main or master".into()
    });
  }
  (force && words.iter().any(|w| w == "--all")).then(|| "git push --force --all".into())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn stop(line: &str) -> Option<String> {
    hard_stop(
      line,
      Some(Path::new("/home/u/devel/app")),
      Some(Path::new("/home/u")),
    )
  }

  #[test]
  fn whole_trees_stop() {
    for line in [
      "rm -rf /",
      "rm -rf /*",
      "sudo rm -rf --no-preserve-root /",
      "rm -rf ~",
      "rm -rf ~/",
      "rm -fr ~/*",
      "rm -rf $HOME",
      "rm -rf \"$HOME/\"",
      "rm -rf ${HOME}/*",
      "rm -rf $BUILD_DIR/",
      "rm -rf \"${OUT}\"/*",
      "rm -rf $X",
      "cd ~ && rm -rf *",
      "cd / && rm -rf .",
      "rm -r /home/u",
      "sudo rm -rf /usr",
      "rm -rf /etc/",
      "find / -type f -mtime +30 -exec rm {} \\;",
      "find ~ -name '*.md' -delete",
      "bash -c 'rm -rf ~'",
      "sh -c \"cd ~ && rm -rf *\"",
      "chmod -R 000 ~",
      "sudo chown -R nobody /etc",
      // The same trees by other names.
      "rm -rf /home/u/../u",
      "rm -rf //home//u/",
      "rm -rf /home/u/.",
      "cd /home/u/devel/.. && rm -rf *",
      "cd /home/u/devel && rm -rf ../",
      "rm -rf /usr/local/..",
    ] {
      assert!(stop(line).is_some(), "{line}");
    }
  }

  #[test]
  fn ordinary_deletions_run_on() {
    for line in [
      "rm -rf target",
      "rm -rf ./build dist/",
      "rm -rf /tmp/x",
      "rm -rf ~/devel/app/target",
      "rm -rf \"$TMPDIR/stitch-test\"",
      "rm -rf $OUT/cache",
      "rm -f ~/.cache/x.log",
      "rm *",
      "rm -rf *",
      "find . -name '*.o' -delete",
      "find /tmp/x -delete",
      "chmod -R u+w target",
      "git push origin main",
      "git push --force origin feature-x",
      "git push -f origin HEAD:refs/heads/fix-1",
      "git branch -D old",
      "dd if=/dev/zero of=disk.img bs=1M count=10",
      "echo hi > /dev/null",
      "cargo build",
    ] {
      assert_eq!(stop(line), None, "{line}");
    }
  }

  #[test]
  fn devices_pushes_and_the_guard_stop() {
    for line in [
      "sudo dd if=/dev/zero of=/dev/nvme0n1 bs=1M count=100",
      "sudo mkfs.ext4 /dev/sda1",
      "wipefs -a /dev/sdb",
      "cat x.img > /dev/sdb",
      "sudo parted /dev/sda mklabel gpt",
      "git push --force origin main",
      "git push -f origin HEAD:main",
      "git push origin +master",
      "git push origin --delete main",
      "git push origin :main",
      "git push --force-with-lease origin feature:refs/heads/main",
      "git push --mirror backup",
      "git push --force --all",
      "gh repo delete sinkingsugar/stitch --yes",
      "gh api -X DELETE repos/a/b",
    ] {
      assert!(stop(line).is_some(), "{line}");
    }
  }

  #[test]
  fn reading_lines_skip_the_model() {
    for line in [
      "ls -la",
      "cat README.md | head -20",
      "rg -n foo src/ 2>/dev/null",
      "cd crates && ls",
      "ps aux | grep stitch",
    ] {
      assert!(reads_only(line), "{line}");
    }
    for line in [
      "rm x",
      "cat a > b",
      "git branch -D x",
      "find . -delete",
      "sed -i s/a/b/ f",
      "cargo build",
      "python3 x.py",
      // What can make a reading command run something: scored.
      "env LD_PRELOAD=/tmp/p.so ls",
      "LD_PRELOAD=/tmp/p.so ls",
      "GIT_EXTERNAL_DIFF=./helper git diff --ext-diff",
      "PAGER=./x man ls",
      "git status",
      "git log --oneline -5",
      "ls | xargs cat",
      "find . -name x -exec cat {} +",
      "timeout 5 cat x",
    ] {
      assert!(!reads_only(line), "{line}");
    }
  }
}
