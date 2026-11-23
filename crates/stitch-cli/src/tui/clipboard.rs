//! Files and images from the clipboard and from paths dropped on the
//! terminal, for attaching (Ctrl+V).
//!
//! A terminal pastes only text, so an image on the clipboard is read from
//! the platform: macOS through `osascript` (a copied file as its path, else
//! the image as PNG), Wayland through `wl-paste`, X11 through `xclip`. Over
//! SSH that is the remote machine's clipboard.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What Ctrl+V found on the clipboard.
#[derive(Debug, PartialEq)]
pub enum Pasted {
  /// Files copied (in Finder): attached where they are.
  Files(Vec<PathBuf>),
  /// An image, written to this PNG.
  Image(PathBuf),
}

/// Reads the clipboard's files or image; `dir` is where an image is
/// written, as `image-<n>.png` (its name as attached). Else why not.
pub fn read(dir: &Path, n: usize) -> Result<Pasted, String> {
  std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
  let path = dir.join(format!("image-{n}.png"));
  if cfg!(target_os = "macos") {
    if let Some(files) = mac_files() {
      return Ok(Pasted::Files(files));
    }
    let wrote = Command::new("osascript")
      .args([
        "-e",
        "on run argv",
        "-e",
        "set f to open for access (POSIX file (item 1 of argv)) with write permission",
        "-e",
        "try",
        "-e",
        "set eof f to 0",
        "-e",
        "write (the clipboard as «class PNGf») to f",
        "-e",
        "on error e",
        "-e",
        "close access f",
        "-e",
        "error e",
        "-e",
        "end try",
        "-e",
        "close access f",
        "-e",
        "end run",
      ])
      .arg(&path)
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status()
      .is_ok_and(|s| s.success());
    if wrote && is_png(&path) {
      return Ok(Pasted::Image(path));
    }
    let _ = std::fs::remove_file(&path);
    return Err("no image or file on the clipboard".into());
  }
  let mut tool = false;
  for (program, args) in [
    ("wl-paste", &["--no-newline", "--type", "image/png"][..]),
    (
      "xclip",
      &["-selection", "clipboard", "-target", "image/png", "-out"][..],
    ),
  ] {
    let Ok(out) = Command::new(program)
      .args(args)
      .stdin(Stdio::null())
      .stderr(Stdio::null())
      .output()
    else {
      continue;
    };
    tool = true;
    if out.status.success() && out.stdout.starts_with(PNG) {
      std::fs::write(&path, &out.stdout).map_err(|e| e.to_string())?;
      return Ok(Pasted::Image(path));
    }
  }
  Err(if tool {
    "no image on the clipboard".into()
  } else {
    "no clipboard tool (wl-paste or xclip) to read an image with".into()
  })
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

fn is_png(path: &Path) -> bool {
  std::fs::read(path).is_ok_and(|b| b.len() > PNG.len() && b.starts_with(PNG))
}

/// Files copied in Finder, as their paths.
fn mac_files() -> Option<Vec<PathBuf>> {
  let out = Command::new("osascript")
    .args([
      "-e",
      "set out to \"\"",
      "-e",
      "repeat with u in (the clipboard as list)",
      "-e",
      "try",
      "-e",
      "set out to out & (POSIX path of (u as «class furl»)) & linefeed",
      "-e",
      "end try",
      "-e",
      "end repeat",
      "-e",
      "out",
    ])
    .stdin(Stdio::null())
    .stderr(Stdio::null())
    .output()
    .ok()?;
  let files: Vec<PathBuf> = String::from_utf8_lossy(&out.stdout)
    .lines()
    .map(PathBuf::from)
    .filter(|p| p.is_file())
    .collect();
  (!files.is_empty()).then_some(files)
}

/// The files a paste names, when it is nothing but paths to existing files
/// that are not text (images, PDFs, media: what a file dropped on the
/// terminal pastes, escaped or quoted, or as `file://` URLs). A pasted path
/// to a text file stays text: it is more often meant as a name.
pub fn dropped(text: &str) -> Option<Vec<PathBuf>> {
  let words = split(text.trim())?;
  if words.is_empty() {
    return None;
  }
  let mut files = Vec::new();
  for w in words {
    let w = match w.strip_prefix("file://") {
      Some(rest) => percent_decode(rest)?,
      None => w,
    };
    let w = match w.strip_prefix("~/") {
      Some(rest) => std::env::var_os("HOME").map(|h| Path::new(&h).join(rest))?,
      None => PathBuf::from(w),
    };
    if !w.is_absolute() || !w.is_file() || super::files::attachable(&w, u64::MAX).is_ok() {
      return None;
    }
    files.push(w);
  }
  Some(files)
}

/// Splits a shell-like line into words: backslash escapes, single and
/// double quotes. `None` for an unclosed quote.
fn split(text: &str) -> Option<Vec<String>> {
  let mut words = Vec::new();
  let mut word = String::new();
  let mut has = false;
  let mut chars = text.chars();
  let mut quote: Option<char> = None;
  while let Some(c) = chars.next() {
    match (quote, c) {
      (Some(q), c) if c == q => quote = None,
      (Some('"'), '\\') => word.push(chars.next()?),
      (Some(_), c) => word.push(c),
      (None, '\'' | '"') => {
        quote = Some(c);
        has = true;
      }
      (None, '\\') => {
        word.push(chars.next()?);
        has = true;
      }
      (None, c) if c.is_whitespace() => {
        if has {
          words.push(std::mem::take(&mut word));
          has = false;
        }
      }
      (None, c) => {
        word.push(c);
        has = true;
      }
    }
  }
  if quote.is_some() {
    return None;
  }
  if has {
    words.push(word);
  }
  Some(words)
}

fn percent_decode(s: &str) -> Option<String> {
  let b = s.as_bytes();
  let mut out = Vec::with_capacity(b.len());
  let mut i = 0;
  while i < b.len() {
    if b[i] == b'%' {
      let hex = s.get(i + 1..i + 3)?;
      out.push(u8::from_str_radix(hex, 16).ok()?);
      i += 3;
    } else {
      out.push(b[i]);
      i += 1;
    }
  }
  String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn splits_as_terminals_paste_dropped_files() {
    assert_eq!(
      split(r"/a/My\ Shot.png '/b/c d.pdf' x").unwrap(),
      ["/a/My Shot.png", "/b/c d.pdf", "x"]
    );
    assert_eq!(split(r#""/a/\"q\".png""#).unwrap(), [r#"/a/"q".png"#]);
    assert!(split("'open").is_none());
    assert_eq!(percent_decode("/a/b%20c.png").unwrap(), "/a/b c.png");
  }

  #[test]
  fn only_paths_to_files_that_are_not_text_are_dropped() {
    let dir = std::env::temp_dir().join(format!("stitch-drop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("a shot.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n\0\0\0").unwrap();
    let txt = dir.join("notes.txt");
    std::fs::write(&txt, "hello").unwrap();
    let escaped = png.display().to_string().replace(' ', "\\ ");
    assert_eq!(dropped(&format!("{escaped} ")), Some(vec![png.clone()]));
    assert_eq!(
      dropped(&format!("'{}'", png.display())),
      Some(vec![png.clone()])
    );
    assert_eq!(
      dropped(&format!(
        "file://{}",
        png.display().to_string().replace(' ', "%20")
      )),
      Some(vec![png.clone()])
    );
    // A text file, a missing one, words: text.
    assert_eq!(dropped(&txt.display().to_string()), None);
    assert_eq!(
      dropped(&format!("{escaped} {}", dir.join("nope.png").display())),
      None
    );
    assert_eq!(dropped("look at this"), None);
    assert_eq!(dropped(""), None);
    std::fs::remove_dir_all(&dir).unwrap();
  }
}
