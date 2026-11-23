//! Seeing local files: `view_file` shows a file on this node the way
//! `browse_web` shows a download (images to the model, PDFs as text with
//! page images, spreadsheets and CSV as tables), and `zoom_image` crops a
//! region of an image the model saw from its full-resolution original.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};
use stitch_providers::ToolDef;
use stitch_store::BlobStore;

use crate::tools::{ToolImage, ToolOutcome};
use crate::web::files;

/// Largest file shown.
const MAX_FILE: u64 = files::MAX_DOWNLOAD;
/// Most entries a directory listing shows.
const MAX_ENTRIES: usize = 1000;
/// Deepest a listing goes.
const MAX_DEPTH: usize = 5;
/// Entries walked past the shown ones to count those left out.
const MAX_COUNTED: usize = 100_000;
/// Most text a listing shows, under what `cut` keeps.
const MAX_LISTING: usize = 30_000;

pub(crate) fn definitions() -> Vec<ToolDef> {
  vec![
    ToolDef {
      name: "view_file".into(),
      description: "Look at a file on this machine: images come back as images you see; PDFs as \
        text by page, with page images where pages have no text or large pictures (read long \
        ones in ranges with start_page and max_pages); spreadsheets and CSV as Markdown tables; \
        text as text; part of a text file with start_line and lines, numbered (no sed or awk \
        needed). Use it for screenshots, scans, charts and documents. A directory comes back \
        as a listing: one entry per line, sorted, directories ending in `/`, files with their size, symlinks with their target; at most \
        1000 entries and 30,000 characters, saying how many were left out; only the directory itself unless depth \
        says more levels. A relative path is from where the thread's shell sessions start."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "path": {"type": "string", "description": "The file or directory: absolute, `~/…`, or relative."},
          "start_page": {"type": "integer", "minimum": 1, "description": "First PDF page, default 1."},
          "max_pages": {"type": "integer", "minimum": 1, "maximum": 20, "description": "PDF pages to read, default 10."},
          "depth": {"type": "integer", "minimum": 1, "maximum": MAX_DEPTH, "description": "For a directory: levels to list, default 1 (its own entries); deeper entries are shown by their path within it."},
          "start_line": {"type": "integer", "minimum": 1, "description": "For a text file: the first line to show (1 is the first), its lines numbered. Default 1 when lines is given."},
          "lines": {"type": "integer", "minimum": 1, "description": "For a text file: how many lines to show from start_line (default: to the end, within the 32,000 characters shown, which says where to go on)."}
        },
        "required": ["path"]
      }),
    },
    ToolDef {
      name: "zoom_image".into(),
      description: "Look closer at part of an image you saw: the region comes back cropped from \
        the original at full resolution (images you get are often scaled down, losing small \
        text and detail). Name the image by its ID (`img:…`, shown before each image) and the \
        region by fractions of its width and height: left, top, right, bottom from 0 to 1, \
        e.g. 0.5, 0, 1, 0.5 for the top right quarter."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "image": {"type": "string", "description": "The image's ID, `img:…`."},
          "left": {"type": "number", "minimum": 0, "maximum": 1},
          "top": {"type": "number", "minimum": 0, "maximum": 1},
          "right": {"type": "number", "minimum": 0, "maximum": 1},
          "bottom": {"type": "number", "minimum": 0, "maximum": 1}
        },
        "required": ["image", "left", "top", "right", "bottom"]
      }),
    },
  ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct View {
  path: String,
  #[serde(default)]
  start_page: Option<usize>,
  #[serde(default)]
  max_pages: Option<usize>,
  #[serde(default)]
  depth: Option<usize>,
  #[serde(default)]
  start_line: Option<usize>,
  #[serde(default)]
  lines: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Zoom {
  image: String,
  left: f64,
  top: f64,
  right: f64,
  bottom: f64,
}

/// Shows a local file; `base` is where relative paths start, `aliases`
/// what other nodes' paths of the place name here.
pub(crate) async fn view(
  input: &Value,
  base: &Path,
  aliases: &crate::place::Aliases,
  scratch: &Path,
) -> ToolOutcome {
  let req: View = match serde_json::from_value(input.clone()) {
    Ok(r) => r,
    Err(e) => return ToolOutcome::err(e.to_string()),
  };
  let path = aliases
    .map(&req.path)
    .unwrap_or_else(|| resolve(&req.path, base));
  let pages = (
    req.start_page.unwrap_or(1).max(1),
    req.max_pages.unwrap_or(10).clamp(1, 20),
  );
  let scratch = scratch.to_owned();
  let depth = req.depth.unwrap_or(1).clamp(1, MAX_DEPTH);
  let range = (req.start_line.is_some() || req.lines.is_some())
    .then(|| (req.start_line.unwrap_or(1).max(1), req.lines));
  let shown = tokio::task::spawn_blocking(move || -> Result<files::Reading, String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some((start, count)) = range {
      if meta.is_dir() {
        return Err(format!(
          "{} is a directory: start_line and lines are for text files",
          path.display()
        ));
      }
      return Ok(files::Reading {
        text: lines_of(&path, start, count)?,
        images: Vec::new(),
        long: false,
      });
    }
    if meta.is_dir() {
      return Ok(files::Reading {
        text: list(&path, depth, MAX_ENTRIES)?,
        images: Vec::new(),
        long: false,
      });
    }
    if meta.len() > MAX_FILE {
      return Err(format!("{} is over {} MiB", path.display(), MAX_FILE >> 20));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let local = files::Fetched::local(path, bytes);
    Ok(files::read(&local, &scratch, pages))
  })
  .await;
  match shown {
    Ok(Ok(r)) => ToolOutcome {
      ok: true,
      output: cut(r.text),
      images: r.images,
      spent: None,
      attachments: Vec::new(),
      blobs: Vec::new(),
      generated: Vec::new(),
    },
    Ok(Err(e)) => ToolOutcome::err(e),
    Err(e) => ToolOutcome::err(e.to_string()),
  }
}

/// Most characters of numbered lines shown, under what `cut` keeps.
const MAX_LINES_TEXT: usize = 31_000;

/// Lines `start`… of a text file (`count` of them, else to its end),
/// numbered as `cat -n` does, under a header saying which and of how
/// many; within `MAX_LINES_TEXT` characters, saying where to go on when
/// they stop short. Read as it streams, so a log over `MAX_FILE` reads
/// too. A file that is not text (a NUL byte, a PDF) is refused.
fn lines_of(path: &Path, start: usize, count: Option<usize>) -> Result<String, String> {
  use std::io::{BufRead, Read};
  let err = |e: std::io::Error| format!("{}: {e}", path.display());
  let mut file = std::fs::File::open(path).map_err(err)?;
  let mut head = [0u8; 8192];
  let n = file.read(&mut head).map_err(err)?;
  let head = &head[..n];
  if head.contains(&0) || head.starts_with(b"%PDF-") {
    return Err(format!(
      "{} is not a text file: start_line and lines are for text (leave them out to see it)",
      path.display()
    ));
  }
  let file = std::fs::File::open(path).map_err(err)?;
  let mut reader = std::io::BufReader::new(file);
  let end = count.map(|c| start.saturating_add(c));
  let mut body = String::new();
  let mut total = 0usize;
  let mut last = None;
  let mut stopped = None;
  let mut buf = Vec::new();
  loop {
    buf.clear();
    if reader.read_until(b'\n', &mut buf).map_err(err)? == 0 {
      break;
    }
    total += 1;
    let n = total;
    if n < start || end.is_some_and(|e| n >= e) || stopped.is_some() {
      continue;
    }
    let line = String::from_utf8_lossy(&buf);
    let line = line.trim_end_matches(['\n', '\r']);
    let numbered = format!("{n:>6}\t{line}\n");
    if body.len() + numbered.len() > MAX_LINES_TEXT {
      if last.is_none() {
        // One line longer than all that is shown: its start.
        let mut cut: String = numbered.chars().take(MAX_LINES_TEXT).collect();
        cut.push_str("\n[… line cut: it is longer than what is shown; read it with the shell]\n");
        body.push_str(&cut);
        last = Some(n);
        stopped = Some(n + 1);
      } else {
        stopped = Some(n);
      }
      continue;
    }
    body.push_str(&numbered);
    last = Some(n);
  }
  let header = match last {
    Some(last) => format!("[{}: lines {start}–{last} of {total}]", path.display()),
    None => format!(
      "[{}: {total} lines, none from line {start}]",
      path.display()
    ),
  };
  let mut out = format!("{header}\n\n{body}");
  if let Some(next) = stopped.filter(|n| *n <= total && end.is_none_or(|e| *n < e)) {
    out.push_str(&format!(
      "\n[… stopped at {MAX_LINES_TEXT} characters: go on with start_line {next}]"
    ));
  }
  Ok(out)
}

/// A directory's listing: its entries sorted by name, one a line, deeper
/// ones (to `depth` levels) by their path within it after their directory;
/// directories end in `/`, files have their size, symlinks their target
/// (never followed). At most `cap` lines (and `MAX_LISTING` characters);
/// the rest are counted. An error when `dir` itself cannot be read.
fn list(dir: &Path, depth: usize, cap: usize) -> Result<String, String> {
  struct Walk {
    lines: Vec<String>,
    chars: usize,
    seen: usize,
    cap: usize,
    /// A line was left out: none after it is shown, so they stay in order.
    full: bool,
    stopped: bool,
  }

  fn walk(dir: &Path, prefix: &str, depth: usize, w: &mut Walk) -> std::io::Result<()> {
    let mut entries: Vec<(String, std::fs::DirEntry)> = std::fs::read_dir(dir)?
      .filter_map(Result::ok)
      .map(|e| (e.file_name().to_string_lossy().into_owned(), e))
      .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, entry) in entries {
      if w.seen >= MAX_COUNTED {
        w.stopped = true;
        return Ok(());
      }
      w.seen += 1;
      let path = format!("{prefix}{name}");
      let kind = entry.file_type();
      let is_dir = kind.as_ref().is_ok_and(|k| k.is_dir());
      let mut shown = !w.full && w.lines.len() < w.cap;
      if shown {
        let line = match &kind {
          Ok(k) if k.is_dir() => format!("{path}/"),
          Ok(k) if k.is_symlink() => match std::fs::read_link(entry.path()) {
            Ok(to) => format!("{path} -> {}  (symlink)", to.display()),
            Err(_) => format!("{path}  (symlink)"),
          },
          Ok(k) if k.is_file() => match entry.metadata() {
            Ok(m) => format!("{path}  {}", size(m.len())),
            Err(e) => format!("{path}  ({e})"),
          },
          Ok(_) => format!("{path}  (special file)"),
          Err(e) => format!("{path}  ({e})"),
        };
        if w.chars + line.len() < MAX_LISTING {
          w.chars += line.len() + 1;
          w.lines.push(line);
        } else {
          shown = false;
        }
      }
      w.full |= !shown;
      if is_dir && depth > 1 {
        let at = w.lines.len();
        if let Err(e) = walk(&entry.path(), &format!("{path}/"), depth - 1, w)
          && shown
        {
          w.lines[at - 1] = format!("{path}/  (unreadable: {e})");
        }
        if w.stopped {
          return Ok(());
        }
      }
    }
    Ok(())
  }

  let mut w = Walk {
    lines: Vec::new(),
    chars: 0,
    seen: 0,
    cap,
    full: false,
    stopped: false,
  };
  walk(dir, "", depth, &mut w).map_err(|e| format!("{}: {e}", dir.display()))?;
  let shown = w.lines.len();
  let more = w.seen - shown;
  let total = if w.stopped {
    format!("over {}", w.seen)
  } else {
    w.seen.to_string()
  };
  let levels = if depth > 1 {
    format!(", {depth} levels deep")
  } else {
    String::new()
  };
  let mut out = format!(
    "Directory {}: {total} entr{}{levels}.\n",
    dir.display(),
    if w.seen == 1 { "y" } else { "ies" }
  );
  if shown == 0 {
    out.push_str("(empty)\n");
  }
  for line in &w.lines {
    out.push_str(line);
    out.push('\n');
  }
  if more > 0 || w.stopped {
    let more = if w.stopped {
      format!("over {more}")
    } else {
      more.to_string()
    };
    out.push_str(&format!(
      "[… {more} more left out after the first {shown}: list a subdirectory, or use less depth]\n"
    ));
  }
  Ok(out)
}

/// A file size, readable.
fn size(n: u64) -> String {
  const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
  if n < 1024 {
    return format!("{n} B");
  }
  let mut v = n as f64 / 1024.0;
  let mut unit = 0;
  while v >= 1024.0 && unit < UNITS.len() - 1 {
    v /= 1024.0;
    unit += 1;
  }
  format!("{v:.1} {}", UNITS[unit])
}

/// Long text, cut; the file itself is there for the shell.
fn cut(text: String) -> String {
  const MAX: usize = 32_000;
  if text.chars().count() <= MAX {
    return text;
  }
  let cut: String = text.chars().take(MAX).collect();
  format!("{cut}\n\n[… cut at {MAX} characters; read the rest with the shell]")
}

/// Why `view_file` does not read what `input` names for a fixer: a path
/// its sandbox keeps from being read (`Sandbox::unreadable`: the
/// credentials), however it is spelled (links followed).
pub(crate) fn unreadable(input: &Value, base: &Path, unreadable: &[PathBuf]) -> Option<String> {
  if unreadable.is_empty() {
    return None;
  }
  let path = resolve(input.get("path")?.as_str()?, base);
  let real = |p: &Path| {
    let mut rest = Vec::new();
    let mut cur = p.to_owned();
    loop {
      if let Ok(r) = std::fs::canonicalize(&cur) {
        return rest.into_iter().rev().fold(r, |a: PathBuf, n| a.join(n));
      }
      match (cur.parent().map(Path::to_owned), cur.file_name()) {
        (Some(parent), Some(name)) => {
          rest.push(name.to_owned());
          cur = parent;
        }
        _ => return p.to_owned(),
      }
    }
  };
  // `..` taken lexically too (an ancestor that does not exist resolves
  // nothing).
  let lexical = path.components().fold(PathBuf::new(), |mut p, c| match c {
    std::path::Component::ParentDir => {
      p.pop();
      p
    }
    std::path::Component::CurDir => p,
    c => {
      p.push(c);
      p
    }
  });
  let got = real(&path);
  unreadable
    .iter()
    .any(|u| {
      let u_real = real(u);
      got.starts_with(&u_real) || lexical.starts_with(u) || real(&lexical).starts_with(&u_real)
    })
    .then(|| {
      format!(
        "{} is not read for a fixer of Stitch (credentials: docs/self-improve.md §5)",
        path.display()
      )
    })
}

pub(crate) fn resolve(path: &str, base: &Path) -> PathBuf {
  let path = path.trim();
  let expanded = match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
    (Some(rest), Some(h)) => Path::new(&h).join(rest),
    _ => PathBuf::from(path),
  };
  if expanded.is_absolute() {
    expanded
  } else {
    base.join(expanded)
  }
}

/// Crops a region of a stored image; `scratch` holds the crop until the
/// loop stores it.
pub(crate) async fn zoom(input: &Value, blobs: &BlobStore, scratch: &Path) -> ToolOutcome {
  let req: Zoom = match serde_json::from_value(input.clone()) {
    Ok(r) => r,
    Err(e) => return ToolOutcome::err(e.to_string()),
  };
  let Some(hash) = crate::images::resolve(blobs, &req.image) else {
    return ToolOutcome::err(format!(
      "no image {} here: use the `img:…` ID shown with it",
      req.image
    ));
  };
  let source = blobs.path(&hash);
  let scratch = scratch.to_owned();
  let region = [req.left, req.top, req.right, req.bottom];
  let cropped =
    tokio::task::spawn_blocking(move || -> Result<(PathBuf, &'static str, String), String> {
      let bytes = std::fs::read(&source).map_err(|e| e.to_string())?;
      let (w, h) = crate::images::dimensions(&bytes).ok_or("not an image I can read")?;
      let ((out, media), (cw, ch)) = crate::images::crop(&bytes, region)?;
      let ext = if media == "image/png" { "png" } else { "jpg" };
      let path = files::save(&scratch, &out, ext)?;
      Ok((
        path,
        media,
        format!("{cw}×{ch} pixels of the {w}×{h} original"),
      ))
    })
    .await;
  match cropped {
    Ok(Ok((path, media, size))) => ToolOutcome {
      ok: true,
      output: format!(
        "Region {:.2}, {:.2} to {:.2}, {:.2} of {}: {size}. It follows.",
        req.left,
        req.top,
        req.right,
        req.bottom,
        crate::images::short_id(&hash)
      ),
      images: vec![ToolImage {
        path,
        media_type: media.into(),
      }],
      spent: None,
      attachments: Vec::new(),
      blobs: Vec::new(),
      generated: Vec::new(),
    },
    Ok(Err(e)) => ToolOutcome::err(e),
    Err(e) => ToolOutcome::err(e.to_string()),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, _| {
      image::Rgb([if x < w / 2 { 0 } else { 255 }, 0, 0])
    }));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
  }

  #[tokio::test]
  async fn local_files_are_seen_and_images_zoomed() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let scratch = d.join("scratch");
    std::fs::write(d.join("shot.png"), png(800, 600)).unwrap();
    std::fs::write(d.join("notes.rs"), "fn main() {}\n").unwrap();
    std::fs::write(d.join("table.csv"), "a,b\n1,2\n").unwrap();
    std::fs::write(d.join("blob.bin"), [0u8, 159, 146, 150]).unwrap();

    let out = view(
      &json!({"path": "shot.png"}),
      d,
      &Default::default(),
      &scratch,
    )
    .await;
    assert!(out.ok, "{}", out.output);
    assert_eq!(out.images.len(), 1);
    assert_eq!(out.images[0].media_type, "image/png");
    let out = view(
      &json!({"path": d.join("notes.rs")}),
      Path::new("/"),
      &Default::default(),
      &scratch,
    )
    .await;
    // Another node's path of the place names it here.
    let aliases = crate::place::Aliases::new(vec!["~/house".into()], d);
    let there = view(
      &json!({"path": "/Users/someone/house/notes.rs"}),
      Path::new("/"),
      &aliases,
      &scratch,
    )
    .await;
    assert_eq!(there.output, out.output);
    assert!(
      out.output.contains("text/plain") && out.output.ends_with("fn main() {}\n"),
      "{}",
      out.output
    );
    let out = view(
      &json!({"path": "table.csv"}),
      d,
      &Default::default(),
      &scratch,
    )
    .await;
    assert!(out.output.contains("| a | b |"), "{}", out.output);
    let out = view(
      &json!({"path": "blob.bin"}),
      d,
      &Default::default(),
      &scratch,
    )
    .await;
    assert!(
      out.ok && out.output.contains("use the shell"),
      "{}",
      out.output
    );
    let out = view(&json!({"path": "."}), d, &Default::default(), &scratch).await;
    assert!(
      out.ok && out.output.contains("\nshot.png  "),
      "{}",
      out.output
    );
    assert!(
      !view(
        &json!({"path": "missing.png"}),
        d,
        &Default::default(),
        &scratch
      )
      .await
      .ok
    );

    // Zoomed by its short ID, from the stored original.
    let blobs = BlobStore::open(&d.join("blobs")).unwrap();
    let hash = blobs.put_bytes(&png(800, 600)).unwrap();
    let id = crate::images::short_id(&hash);
    let out = zoom(
      &json!({"image": id, "left": 0.5, "top": 0.0, "right": 1.0, "bottom": 0.5}),
      &blobs,
      &scratch,
    )
    .await;
    assert!(out.ok, "{}", out.output);
    assert!(
      out
        .output
        .contains("400×300 pixels of the 800×600 original"),
      "{}",
      out.output
    );
    let region = std::fs::read(&out.images[0].path).unwrap();
    let img = image::load_from_memory(&region).unwrap().to_rgb8();
    let [r, g, _] = img.get_pixel(10, 10).0;
    assert!(r > 240 && g < 15, "the right half (red), JPEG-encoded");
    let out = zoom(
      &json!({"image": "img:000000000000", "left": 0, "top": 0, "right": 1, "bottom": 1}),
      &blobs,
      &scratch,
    )
    .await;
    assert!(!out.ok && out.output.contains("no image"));
  }

  /// 2026-11-08: read-only threads reached for `sed -n a,bp` (76 of 304
  /// refusals) to read part of a file.
  #[tokio::test]
  async fn text_files_are_read_by_line_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let scratch = d.join("scratch");
    let text: String = (1..=100).map(|n| format!("line {n}\n")).collect();
    std::fs::write(d.join("a.rs"), &text).unwrap();
    let view_with = |input: Value| {
      let scratch = scratch.clone();
      async move { view(&input, d, &Default::default(), &scratch).await }
    };
    let out = view_with(json!({"path": "a.rs", "start_line": 10, "lines": 3})).await;
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("lines 10–12 of 100"), "{}", out.output);
    assert!(
      out
        .output
        .ends_with("    10\tline 10\n    11\tline 11\n    12\tline 12\n"),
      "{}",
      out.output
    );
    assert!(!out.output.contains("line 13"), "{}", out.output);
    // To the end; from the start.
    let out = view_with(json!({"path": "a.rs", "start_line": 99})).await;
    assert!(
      out.output.contains("lines 99–100 of 100") && out.output.ends_with("   100\tline 100\n"),
      "{}",
      out.output
    );
    let out = view_with(json!({"path": "a.rs", "lines": 1})).await;
    assert!(out.output.ends_with("     1\tline 1\n"), "{}", out.output);
    let out = view_with(json!({"path": "a.rs", "start_line": 500})).await;
    assert!(
      out.output.contains("100 lines, none from line 500"),
      "{}",
      out.output
    );
    // Without them, the file whole, as before.
    let out = view_with(json!({"path": "a.rs"})).await;
    assert!(out.output.ends_with("line 100\n") && !out.output.contains("\tline"));
    // Long: stopped, saying where to go on.
    let long: String = (1..=5000)
      .map(|n| format!("{n} {}\n", "x".repeat(20)))
      .collect();
    std::fs::write(d.join("long.txt"), long).unwrap();
    let out = view_with(json!({"path": "long.txt", "start_line": 1})).await;
    assert!(out.output.len() < 32_000, "{}", out.output.len());
    let next: usize = out
      .output
      .rsplit("go on with start_line ")
      .next()
      .and_then(|s| s.trim_end_matches(']').parse().ok())
      .expect("where to go on");
    assert!(
      out
        .output
        .contains(&format!("lines 1–{} of 5000", next - 1))
    );
    // Not text, or a directory: refused.
    std::fs::write(d.join("blob.bin"), [0u8, 1, 2]).unwrap();
    let out = view_with(json!({"path": "blob.bin", "start_line": 1})).await;
    assert!(
      !out.ok && out.output.contains("not a text file"),
      "{}",
      out.output
    );
    let out = view_with(json!({"path": ".", "lines": 5})).await;
    assert!(!out.ok, "{}", out.output);
  }

  #[tokio::test]
  async fn directories_are_listed() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let scratch = d.join("scratch");
    std::fs::create_dir_all(d.join("work/src/deep")).unwrap();
    std::fs::create_dir(d.join("work/empty")).unwrap();
    std::fs::write(d.join("work/b.txt"), "hello").unwrap();
    std::fs::write(d.join("work/a.bin"), vec![0u8; 3 * 1024]).unwrap();
    std::fs::write(d.join("work/src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(d.join("work/src/deep/x"), "").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("b.txt", d.join("work/link")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("src", d.join("work/dirlink")).unwrap();

    // Sorted, directories ending in `/`, files with sizes, symlinks marked
    // and not followed; a relative path from the shells' start.
    let out = view(&json!({"path": "work"}), d, &Default::default(), &scratch).await;
    assert!(out.ok, "{}", out.output);
    let lines: Vec<&str> = out.output.lines().skip(1).collect();
    let mut want = vec!["a.bin  3.0 KiB", "b.txt  5 B"];
    #[cfg(unix)]
    want.push("dirlink -> src  (symlink)");
    want.push("empty/");
    #[cfg(unix)]
    want.push("link -> b.txt  (symlink)");
    want.push("src/");
    assert_eq!(lines, want, "{}", out.output);
    assert!(
      out.output.starts_with(&format!(
        "Directory {}: {} entries.\n",
        d.join("work").display(),
        want.len()
      )),
      "{}",
      out.output
    );
    let absolute = view(
      &json!({"path": d.join("work")}),
      Path::new("/"),
      &Default::default(),
      &scratch,
    )
    .await;
    assert_eq!(absolute.output, out.output);

    // Deeper levels by their path within, after their directory.
    let out = view(
      &json!({"path": "work/src", "depth": 2}),
      d,
      &Default::default(),
      &scratch,
    )
    .await;
    let lines: Vec<&str> = out.output.lines().skip(1).collect();
    assert_eq!(
      lines,
      ["deep/", "deep/x  0 B", "main.rs  13 B"],
      "{}",
      out.output
    );
    assert!(
      out.output.contains("3 entries, 2 levels deep"),
      "{}",
      out.output
    );
    let out = view(
      &json!({"path": "work/src"}),
      d,
      &Default::default(),
      &scratch,
    )
    .await;
    assert!(!out.output.contains("deep/x"), "{}", out.output);

    let out = view(
      &json!({"path": "work/empty"}),
      d,
      &Default::default(),
      &scratch,
    )
    .await;
    assert!(out.ok, "{}", out.output);
    assert!(
      out.output.ends_with(": 0 entries.\n(empty)\n"),
      "{}",
      out.output
    );

    // A directory that cannot be read is an error, not a listing.
    let err = list(&d.join("work/missing"), 1, MAX_ENTRIES).unwrap_err();
    assert!(err.contains("missing"), "{err}");
  }

  #[test]
  fn listings_are_capped_and_count_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for i in 0..10 {
      std::fs::write(d.join(format!("f{i}")), "").unwrap();
    }
    let out = list(d, 1, 4).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].ends_with(": 10 entries."), "{out}");
    assert_eq!(lines[1..5], ["f0  0 B", "f1  0 B", "f2  0 B", "f3  0 B"]);
    assert!(
      lines[5].starts_with("[… 6 more left out after the first 4"),
      "{out}"
    );
    assert_eq!(lines.len(), 6, "{out}");
  }

  #[test]
  fn long_listings_stop_at_their_characters_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    // 200 names of 200 characters: about 41,000 characters of lines.
    for i in 0..200 {
      std::fs::write(d.join(format!("{i:03}{}", "x".repeat(197))), "").unwrap();
    }
    let out = list(d, 1, MAX_ENTRIES).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].ends_with(": 200 entries."), "{}", lines[0]);
    let shown = &lines[1..lines.len() - 1];
    assert!(shown.len() < 200 && !shown.is_empty());
    // The first ones, in order, nothing skipped.
    for (i, l) in shown.iter().enumerate() {
      assert!(l.starts_with(&format!("{i:03}x")), "{l}");
    }
    let listed: usize = shown.iter().map(|l| l.len() + 1).sum();
    assert!(listed < MAX_LISTING, "{listed}");
    assert_eq!(
      lines.last().unwrap(),
      &format!(
        "[… {} more left out after the first {}: list a subdirectory, or use less depth]",
        200 - shown.len(),
        shown.len()
      )
    );
  }

  #[test]
  fn sizes_read_well() {
    assert_eq!(size(0), "0 B");
    assert_eq!(size(1023), "1023 B");
    assert_eq!(size(1536), "1.5 KiB");
    assert_eq!(size(5 << 20), "5.0 MiB");
  }
}
