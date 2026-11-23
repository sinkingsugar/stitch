//! The build's identity (docs/self-improve.md §2.1): `STITCH_BUILD`, the
//! commit of Stitch (`-dirty` when the checkout had changes to tracked
//! files), then Otter's and engram's as Cargo.lock pins them:
//! `<stitch> otter <rev> engram <rev>`, each 12 hex digits. While `just
//! dev-link` points one of them at a checkout beside this one, that
//! checkout's commit and "(linked)": `otter <commit>[-dirty] (linked)`.
//! tools/build-label.sh says the same for the shell's builds and the
//! self-update (a test holds them together). `STITCH_BUILD` set in the
//! environment of the build wins. Run again when the checkout's HEAD
//! moves, Cargo.lock or the link changes, or any file under `crates/`
//! changes (cargo rebuilds this crate then anyway: it depends on all of
//! them).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where Otter and engram come from (Cargo.lock's `source` lines).
const DEPS: [&str; 2] = ["otter", "engram"];

fn git(dir: &Path, args: &[&str]) -> Option<String> {
  let out = Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(args)
    .output()
    .ok()?;
  out
    .status
    .success()
    .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// `<commit, 12>`, with `-dirty` when tracked files changed; none outside a
/// checkout.
fn commit(dir: &Path) -> Option<String> {
  let head = git(dir, &["rev-parse", "--short=12", "HEAD"])?;
  let dirty =
    git(dir, &["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
  Some(if dirty { format!("{head}-dirty") } else { head })
}

/// The commit Cargo.lock pins `dep` at: the first
/// `source = "git+https://github.com/sinkingsugar/<dep>[?…]#<sha>"`.
fn pinned(lock: &str, dep: &str) -> Option<String> {
  let prefix = format!("source = \"git+https://github.com/sinkingsugar/{dep}");
  lock.lines().find_map(|line| {
    let rest = line.strip_prefix(&prefix)?;
    if !rest.starts_with(['#', '?']) {
      return None;
    }
    let sha = rest.rsplit_once('#')?.1.strip_suffix('"')?;
    (sha.len() >= 12 && sha.bytes().all(|b| b.is_ascii_hexdigit())).then(|| sha[..12].to_owned())
  })
}

/// The checkout `just dev-link` points `dep` at (`# link <dep> <dir>` in
/// .cargo/dev-link.toml), if it does.
fn linked(link: &str, dep: &str) -> Option<PathBuf> {
  let prefix = format!("# link {dep} ");
  link
    .lines()
    .find_map(|l| l.strip_prefix(&prefix).map(|d| PathBuf::from(d.trim())))
}

fn main() {
  println!("cargo:rerun-if-env-changed=STITCH_BUILD");
  let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
  let root = Path::new(&manifest).join("../..");
  let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap_or_default();
  let link_file = root.join(".cargo/dev-link.toml");
  let link = std::fs::read_to_string(&link_file).unwrap_or_default();
  let mut repos = vec![root.clone()];
  let build = match std::env::var("STITCH_BUILD") {
    Ok(b) if !b.trim().is_empty() => b,
    _ => {
      let mut parts = vec![commit(&root).unwrap_or_else(|| "unknown".into())];
      for dep in DEPS {
        if let Some(dir) = linked(&link, dep) {
          parts.push(format!(
            "{dep} {} (linked)",
            commit(&dir).unwrap_or_else(|| "unknown".into())
          ));
          repos.push(dir);
        } else if let Some(rev) = pinned(&lock, dep) {
          parts.push(format!("{dep} {rev}"));
        }
      }
      parts.join(" ")
    }
  };
  println!("cargo:rustc-env=STITCH_BUILD={build}");
  println!("cargo:rerun-if-changed={}", root.join("crates").display());
  println!(
    "cargo:rerun-if-changed={}",
    root.join("Cargo.lock").display()
  );
  // The link coming or going: .cargo/ (a file added or removed changes
  // it; unlinking rewrites Cargo.lock too).
  println!("cargo:rerun-if-changed={}", root.join(".cargo").display());
  // The commits of Stitch, and of a linked checkout: a commit there alone
  // changes the label too. (The node knows a build by its binary's hash
  // anyway: this only keeps the label true.)
  for repo in repos {
    for path in ["HEAD", "packed-refs"] {
      if let Some(p) = git(&repo, &["rev-parse", "--git-path", path]) {
        println!("cargo:rerun-if-changed={}", repo.join(p).display());
      }
    }
    if let Some(r) = git(&repo, &["symbolic-ref", "-q", "HEAD"])
      && let Some(p) = git(&repo, &["rev-parse", "--git-path", &r])
    {
      println!("cargo:rerun-if-changed={}", repo.join(p).display());
    }
  }
}
