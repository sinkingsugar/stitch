//! An app's data folder (design §2.23): the user's files of that app,
//! apart from its code: its threads' place, where its commands' outputs
//! go, and what its page reads and writes through `stitch.files`
//! (`/_stitch/files…`), scoped to that folder alone.

use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use super::Manifest;
use super::containers::spec::plain_rel;
use super::serve::{Request, Response};
use super::threads::{self, Consent, Grant, ThreadNeeds};
use crate::node::Shared;

/// A write from the page, at most (a request holds 16 MiB).
pub const MAX_WRITE: usize = 15 * 1024 * 1024;
/// Files listed at most.
const MAX_LIST: usize = 2000;

/// The data folder of granted needs, resolved here.
pub fn data_dir(needs: &ThreadNeeds) -> Option<PathBuf> {
  let f = needs.data.as_ref()?.folder.as_ref()?;
  Some(crate::place::expand_home(f))
}

/// Its quota in bytes.
pub fn quota(needs: &ThreadNeeds) -> u64 {
  needs
    .data
    .as_ref()
    .and_then(|d| d.quota_mb)
    .unwrap_or(threads::DATA_QUOTA_MB)
    << 20
}

/// Makes the data folder when it is not there, with its `.stitch/ignore`
/// listing the subfolders that stay on this node; refused inside the
/// node's data or a git checkout.
pub(crate) fn ensure(s: &Shared, needs: &ThreadNeeds) -> Result<Option<PathBuf>, String> {
  let Some(dir) = data_dir(needs) else {
    return Ok(None);
  };
  if threads::holds_data(&s.data, &dir) {
    return Err(format!("{}: it holds the node's data", dir.display()));
  }
  if threads::in_checkout(&dir) {
    return Err(format!(
      "{}: in a git checkout; an app's data folder is a folder of its own",
      dir.display()
    ));
  }
  if let Ok(m) = std::fs::symlink_metadata(&dir)
    && !m.is_dir()
  {
    return Err(format!("{}: not a folder", dir.display()));
  }
  std::fs::create_dir_all(dir.join(".stitch")).map_err(|e| format!("{}: {e}", dir.display()))?;
  let local = needs
    .data
    .as_ref()
    .and_then(|d| d.local.clone())
    .unwrap_or_default();
  let ignore = dir.join(".stitch").join("ignore");
  let mut text = std::fs::read_to_string(&ignore).unwrap_or_default();
  let mut changed = false;
  for l in &local {
    let line = format!("{l}/*");
    if !text.lines().any(|x| x.trim() == line) {
      if text.is_empty() {
        text.push_str("# Not synced: this app's outputs and caches, rebuilt here (Stitch).\n");
      }
      text.push_str(&line);
      text.push('\n');
      changed = true;
    }
  }
  if changed {
    std::fs::write(&ignore, text).map_err(|e| e.to_string())?;
  }
  Ok(Some(dir))
}

/// What the folder holds: bytes and files, links not followed.
pub fn usage(dir: &Path) -> (u64, u64) {
  let (mut bytes, mut files) = (0, 0);
  for e in walkdir::WalkDir::new(dir)
    .follow_links(false)
    .max_depth(32)
    .into_iter()
    .flatten()
  {
    if e.file_type().is_file()
      && let Ok(m) = e.metadata()
    {
      bytes += m.len();
      files += 1;
    }
  }
  (bytes, files)
}

/// The path `rel` names in `root`, every existing part a real folder (no
/// link), the last part what it is, or not there.
pub fn resolve(root: &Path, rel: &str) -> Result<PathBuf, String> {
  if !plain_rel(rel) {
    return Err(format!("{rel}: a plain path (no `..`, no hidden parts)"));
  }
  let mut p = root.to_owned();
  let parts: Vec<&str> = rel.split('/').collect();
  for (i, c) in parts.iter().enumerate() {
    p.push(c);
    match std::fs::symlink_metadata(&p) {
      Ok(m) if m.file_type().is_symlink() => {
        return Err(format!("{rel}: {c} is a link; links are not followed"));
      }
      Ok(m) if i + 1 < parts.len() && !m.is_dir() => {
        return Err(format!("{rel}: {c} is not a folder"));
      }
      Ok(_) => {}
      Err(_) => {
        // The rest is not there: nothing below can be a link.
        for c in &parts[i + 1..] {
          p.push(c);
        }
        return Ok(p);
      }
    }
  }
  Ok(p)
}

/// Makes the new folder `rel` (it must not be there), its parents as
/// needed, none a link.
pub fn make_dir(root: &Path, rel: &str) -> Result<PathBuf, String> {
  let p = resolve(root, rel)?;
  if let Some(parent) = p.parent() {
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  }
  // Checked again once made: nothing turned into a link meanwhile.
  let again = resolve(root, rel)?;
  std::fs::create_dir(&again).map_err(|e| format!("{rel}: {e}"))?;
  Ok(again)
}

/// Reads a regular file of the folder, without following a link.
pub fn read(root: &Path, rel: &str, max: u64) -> Result<Vec<u8>, String> {
  let p = resolve(root, rel)?;
  let f = std::fs::OpenOptions::new()
    .read(true)
    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
    .open(&p)
    .map_err(|_| format!("{rel}: not there"))?;
  let m = f.metadata().map_err(|e| e.to_string())?;
  if !m.is_file() {
    return Err(format!("{rel}: not a regular file"));
  }
  if m.len() > max {
    return Err(format!("{rel}: over {} MiB", max >> 20));
  }
  let mut b = Vec::new();
  f.take(max).read_to_end(&mut b).map_err(|e| e.to_string())?;
  Ok(b)
}

/// Writes a file of the folder through a temporary file beside it; never
/// over a link or a folder; within `quota` for the whole folder.
pub fn write(root: &Path, rel: &str, bytes: &[u8], quota: u64) -> Result<(), String> {
  if crate::apps::guard::steers_agents(Path::new(rel)) {
    return Err(format!(
      "{rel}: instructions for agents are not written from a page"
    ));
  }
  let p = resolve(root, rel)?;
  let old = match std::fs::symlink_metadata(&p) {
    Ok(m) if m.is_file() => m.len(),
    Ok(_) => return Err(format!("{rel}: not a regular file")),
    Err(_) => 0,
  };
  let (used, _) = usage(root);
  if used.saturating_sub(old) + bytes.len() as u64 > quota {
    return Err(format!(
      "the app's data folder would pass its quota of {} MiB",
      quota >> 20
    ));
  }
  let parent = p.parent().ok_or("no folder")?;
  std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  // Its parents made: checked again for links.
  let p = resolve(root, rel)?;
  let tmp = parent.join(format!(
    ".{}.stitch-tmp",
    p.file_name().and_then(|n| n.to_str()).unwrap_or("f")
  ));
  std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
  std::fs::rename(&tmp, &p).map_err(|e| {
    let _ = std::fs::remove_file(&tmp);
    e.to_string()
  })
}

/// Removes a regular file of the folder (never a link's target).
pub fn remove(root: &Path, rel: &str) -> Result<(), String> {
  let p = resolve(root, rel)?;
  match std::fs::symlink_metadata(&p) {
    Ok(m) if m.is_file() => std::fs::remove_file(&p).map_err(|e| e.to_string()),
    Ok(_) => Err(format!("{rel}: not a regular file")),
    Err(_) => Err(format!("{rel}: not there")),
  }
}

/// The files under `rel` (the folder's top when empty): path, size,
/// modified (seconds), at most 2000, hidden parts and links left out.
pub fn list(root: &Path, rel: &str) -> Result<Vec<Value>, String> {
  let base = if rel.is_empty() {
    root.to_owned()
  } else {
    resolve(root, rel)?
  };
  let mut out = Vec::new();
  let walk = walkdir::WalkDir::new(&base)
    .follow_links(false)
    .max_depth(16)
    .sort_by_file_name()
    .into_iter()
    .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));
  for e in walk.flatten() {
    if !e.file_type().is_file() {
      continue;
    }
    let Ok(rp) = e.path().strip_prefix(root) else {
      continue;
    };
    let m = e.metadata().ok();
    out.push(json!({
      "path": rp.to_string_lossy(),
      "size": m.as_ref().map(|m| m.len()),
      "modified": m
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs()),
    }));
    if out.len() >= MAX_LIST {
      break;
    }
  }
  Ok(out)
}

/// A mark that changes when the folder's files change (paths, sizes,
/// times), for a page polling it.
pub fn mark(root: &Path) -> String {
  let mut h = blake3::Hasher::new();
  for v in list(root, "").unwrap_or_default() {
    h.update(v.to_string().as_bytes());
  }
  h.finalize().to_hex()[..16].to_owned()
}

/// The type a file is served as: images by their bytes, a few safe types
/// by name; everything else bytes to download, never HTML or SVG.
pub fn served_type(rel: &str, bytes: &[u8]) -> (&'static str, bool) {
  if let Some(t) = threads::sniff(bytes) {
    return (t, false);
  }
  let ext = rel
    .rsplit_once('.')
    .map(|(_, e)| e.to_ascii_lowercase())
    .unwrap_or_default();
  match ext.as_str() {
    "glb" => ("model/gltf-binary", false),
    "json" | "gltf" => ("application/json", false),
    "txt" | "md" | "csv" | "log" => ("text/plain; charset=utf-8", false),
    _ => ("application/octet-stream", true),
  }
}

fn decode(s: &str) -> Option<String> {
  let b = s.as_bytes();
  let mut out = Vec::with_capacity(b.len());
  let mut i = 0;
  while i < b.len() {
    if b[i] == b'%' {
      let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
      out.push(u8::from_str_radix(h, 16).ok()?);
      i += 3;
    } else {
      out.push(b[i]);
      i += 1;
    }
  }
  String::from_utf8(out).ok()
}

fn reply(v: Value) -> Response {
  Response::json(&v).header("Cross-Origin-Resource-Policy", "same-origin")
}

fn refused(status: u16, error: impl Into<String>) -> Response {
  let mut r = reply(json!({"error": error.into()}));
  r.status = status;
  r
}

/// `/_stitch/files…`: `GET /_stitch/files?dir=<rel>` lists, `GET
/// /_stitch/files?mark` gives the mark, `GET /_stitch/files/<rel>` reads,
/// `POST /_stitch/files/<rel>` writes the body, `POST
/// /_stitch/files/<rel>?remove` removes. Under the app's standing threads
/// grant; changes need this version's.
pub(crate) async fn api(
  s: &Arc<Shared>,
  m: &Manifest,
  rest: &str,
  query: &str,
  req: &Request,
) -> Response {
  let post = req.method.eq_ignore_ascii_case("POST");
  let Some(thread) = &m.thread else {
    return refused(409, "this app has no conversation, so no data folder");
  };
  let (granted, consent) = {
    let index = s.state().index.lock().expect("index lock");
    (
      threads::grant(&index, thread, &m.name),
      threads::consent(&index, m),
    )
  };
  let needs = match granted {
    Some(Grant::Allowed { needs, .. }) => *needs,
    _ => {
      return refused(
        403,
        "the user has not granted this app its threads and data folder",
      );
    }
  };
  let Some(root) = data_dir(&needs) else {
    return refused(
      404,
      "this app declares no data folder (`threads.data` in stitch-app.json)",
    );
  };
  if post && consent != Consent::Allowed {
    return refused(
      403,
      "this version's needs are not granted: the page may read, not write",
    );
  }
  let root = match ensure(s, &needs) {
    Ok(Some(r)) => r,
    Ok(None) => root,
    Err(e) => return refused(409, e),
  };
  let quota = quota(&needs);
  let rel = match decode(rest.trim_start_matches('/')) {
    Some(r) => r,
    None => return refused(400, "a path that does not decode"),
  };
  let (q1, q2) = (query.to_owned(), query.to_owned());
  let has = move |k: &str| {
    q1.split('&')
      .any(|kv| kv == k || kv.starts_with(&format!("{k}=")))
  };
  let arg = move |k: &str| {
    q2.split('&')
      .find_map(|kv| kv.strip_prefix(&format!("{k}=")))
      .and_then(decode)
  };
  let r2 = root.clone();
  let out = tokio::task::spawn_blocking({
    let body = req.body.clone();
    move || -> Response {
      match (rel.is_empty(), post) {
        (true, false) if has("mark") => reply(json!({"mark": mark(&r2)})),
        (true, false) => {
          let (bytes, files) = usage(&r2);
          match list(&r2, &arg("dir").unwrap_or_default()) {
            Ok(l) => reply(json!({"files": l, "used": bytes, "count": files, "quota": quota})),
            Err(e) => refused(400, e),
          }
        }
        (true, true) => refused(400, "name a file"),
        (false, false) => match read(&r2, &rel, 64 << 20) {
          Ok(b) => {
            let (t, download) = served_type(&rel, &b);
            let mut r = Response::new(200, t, b)
              .header("Cross-Origin-Resource-Policy", "same-origin")
              .header("X-Content-Type-Options", "nosniff")
              .header("Content-Security-Policy", "sandbox; default-src 'none'")
              .header("Cache-Control", "no-store");
            if download {
              r = r.header("Content-Disposition", "attachment");
            }
            r
          }
          Err(e) => refused(404, e),
        },
        (false, true) if has("remove") => match remove(&r2, &rel) {
          Ok(()) => reply(json!({"ok": true})),
          Err(e) => refused(400, e),
        },
        (false, true) => {
          if body.len() > MAX_WRITE {
            return refused(413, format!("at most {} MiB a file", MAX_WRITE >> 20));
          }
          match write(&r2, &rel, &body, quota) {
            Ok(()) => reply(json!({"ok": true, "path": rel})),
            Err(e) => refused(400, e),
          }
        }
      }
    }
  })
  .await;
  out.unwrap_or_else(|e| refused(500, e.to_string()))
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::os::unix::fs::symlink;

  #[test]
  fn scoped_to_the_folder() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("Atelier");
    std::fs::create_dir_all(root.join("projects/a")).unwrap();
    std::fs::write(d.path().join("secret.txt"), "KEY").unwrap();
    // Another app's folder beside it.
    std::fs::create_dir_all(d.path().join("Other")).unwrap();
    std::fs::write(d.path().join("Other/x.json"), "{}").unwrap();
    symlink(
      d.path().join("secret.txt"),
      root.join("projects/a/leak.txt"),
    )
    .unwrap();
    symlink(d.path().join("Other"), root.join("other")).unwrap();
    let q = 1 << 20;
    write(&root, "projects/a/recipe.json", b"{}", q).unwrap();
    assert_eq!(read(&root, "projects/a/recipe.json", q).unwrap(), b"{}");
    for bad in [
      "../secret.txt",
      "projects/../../secret.txt",
      "/etc/passwd",
      ".stitch/ignore",
      "projects/a/leak.txt",
      "other/x.json",
      "",
    ] {
      assert!(read(&root, bad, q).is_err(), "read {bad}");
      assert!(write(&root, bad, b"x", q).is_err(), "write {bad}");
    }
    // Writing over a link never writes its target.
    assert!(write(&root, "projects/a/leak.txt", b"x", q).is_err());
    assert_eq!(std::fs::read(d.path().join("secret.txt")).unwrap(), b"KEY");
    assert!(remove(&root, "projects/a/leak.txt").is_err());
    assert!(d.path().join("secret.txt").exists());
    // Steering files.
    assert!(write(&root, "AGENTS.md", b"x", q).is_err());
    // The quota.
    assert!(write(&root, "big.bin", &vec![0u8; 2 << 20], q).is_err());
    write(&root, "small.bin", &vec![0u8; 1000], q).unwrap();
    // Listed: no links, no hidden files.
    std::fs::create_dir_all(root.join(".stitch")).unwrap();
    std::fs::write(root.join(".stitch/ignore"), "out/*\n").unwrap();
    let l = list(&root, "").unwrap();
    let paths: Vec<&str> = l.iter().map(|v| v["path"].as_str().unwrap()).collect();
    assert_eq!(paths, vec!["projects/a/recipe.json", "small.bin"]);
    // A new folder for outputs, never an existing one.
    make_dir(&root, "out/run-1").unwrap();
    assert!(make_dir(&root, "out/run-1").is_err());
    assert!(make_dir(&root, "other/run").is_err());
  }

  #[test]
  fn served_safely() {
    assert_eq!(served_type("a.glb", b"glTF"), ("model/gltf-binary", false));
    assert_eq!(served_type("x.png", b"\x89PNG\r\n\x1a\n...").0, "image/png");
    assert_eq!(
      served_type("x.html", b"<script>"),
      ("application/octet-stream", true)
    );
    assert_eq!(
      served_type("x.svg", b"<svg>"),
      ("application/octet-stream", true)
    );
    assert_eq!(
      served_type("x.blend", b"BLENDER"),
      ("application/octet-stream", true)
    );
  }
}
