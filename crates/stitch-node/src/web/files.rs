//! Files the web tools fetch: downloaded under a size cap into
//! `<data>/downloads/` (named by their content's hash), then read for the
//! model: PDFs as text with page images where text is missing or pictures
//! matter, spreadsheets and CSV as Markdown tables, images as images, text
//! as text. Anything else is saved and its path given.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use crate::tools::ToolImage;

/// Largest download.
pub(crate) const MAX_DOWNLOAD: u64 = 50 * 1024 * 1024;
/// Largest image handed to the model.
const MAX_IMAGE: usize = 20 * 1024 * 1024;
/// Long side of a rendered PDF page, in pixels.
const PAGE_PIXELS: f32 = 1600.0;
/// Rendered pages per call, at most.
const MAX_PAGE_IMAGES: usize = 10;
/// A picture this large (in pixels) on a page with text gets the page
/// rendered too.
const PICTURE_PIXELS: f64 = 150_000.0;
/// Spreadsheet preview limits.
const MAX_SHEETS: usize = 20;
const MAX_ROWS: usize = 200;
const MAX_COLS: usize = 50;

pub(crate) struct Fetched {
  pub url: String,
  pub mime: String,
  pub path: PathBuf,
  pub bytes: Vec<u8>,
  /// A file on this machine (`view_file`), not a download.
  pub local: bool,
}

impl Fetched {
  /// A file on this machine; its type from its name, or text when it
  /// reads as UTF-8.
  pub(crate) fn local(path: PathBuf, bytes: Vec<u8>) -> Fetched {
    let ext = path
      .extension()
      .and_then(|e| e.to_str())
      .unwrap_or("")
      .to_ascii_lowercase();
    let mime = match ext.as_str() {
      "png" => "image/png",
      "jpg" | "jpeg" => "image/jpeg",
      "gif" => "image/gif",
      "webp" => "image/webp",
      "pdf" => "application/pdf",
      "csv" => "text/csv",
      "tsv" => "text/tab-separated-values",
      _ if std::str::from_utf8(&bytes).is_ok() => "text/plain",
      _ => "",
    };
    Fetched {
      url: path.display().to_string(),
      mime: mime.into(),
      path,
      bytes,
      local: true,
    }
  }
}

/// Downloads `url` into `dir`.
pub(super) async fn download(
  http: &reqwest::Client,
  url: &str,
  dir: &Path,
) -> Result<Fetched, String> {
  let mut response = http.get(url).send().await.map_err(|e| e.to_string())?;
  let status = response.status();
  let final_url = response.url().to_string();
  if !status.is_success() {
    return Err(format!("HTTP {status} at {final_url}"));
  }
  if response.content_length().is_some_and(|n| n > MAX_DOWNLOAD) {
    return Err(format!(
      "the file at {final_url} is over {} MiB",
      MAX_DOWNLOAD >> 20
    ));
  }
  let mime = response
    .headers()
    .get(reqwest::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .and_then(|v| v.split(';').next())
    .unwrap_or("")
    .trim()
    .to_ascii_lowercase();
  let mut bytes = Vec::new();
  while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
    if (bytes.len() + chunk.len()) as u64 > MAX_DOWNLOAD {
      return Err(format!(
        "the file at {final_url} is over {} MiB",
        MAX_DOWNLOAD >> 20
      ));
    }
    bytes.extend_from_slice(&chunk);
  }
  let ext = extension(&final_url, &mime, &bytes);
  let path = save(dir, &bytes, &ext)?;
  Ok(Fetched {
    url: final_url,
    mime,
    path,
    bytes,
    local: false,
  })
}

/// Writes `bytes` as `<hash>.<ext>` in `dir`; its path.
pub(crate) fn save(dir: &Path, bytes: &[u8], ext: &str) -> Result<PathBuf, String> {
  std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
  let path = dir.join(format!("{}.{ext}", blake3::hash(bytes).to_hex()));
  if !path.exists() {
    let tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    std::fs::write(tmp.path(), bytes).map_err(|e| e.to_string())?;
    tmp.persist(&path).map_err(|e| e.to_string())?;
  }
  Ok(path)
}

/// The file's kind as an extension: from its content, its type, then its
/// URL.
pub(crate) fn extension(url: &str, mime: &str, bytes: &[u8]) -> String {
  if bytes.starts_with(b"%PDF-") {
    return "pdf".into();
  }
  let by_type = match mime {
    "image/png" => Some("png"),
    "image/jpeg" => Some("jpg"),
    "image/gif" => Some("gif"),
    "image/webp" => Some("webp"),
    "text/csv" => Some("csv"),
    "text/tab-separated-values" => Some("tsv"),
    "application/json" => Some("json"),
    _ => None,
  };
  if let Some(e) = by_type {
    return e.into();
  }
  let path = url::Url::parse(url)
    .map(|u| u.path().to_owned())
    .unwrap_or_default();
  Path::new(&path)
    .extension()
    .and_then(|e| e.to_str())
    .map(str::to_ascii_lowercase)
    .filter(|e| e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()))
    .unwrap_or_else(|| "bin".into())
}

/// What the model is told about a fetched file, and the images it sees.
pub(crate) struct Reading {
  pub text: String,
  pub images: Vec<ToolImage>,
  /// Long text the caller cuts (and saves whole).
  pub long: bool,
}

/// Reads a fetched file; `pages` is the PDF page range (first, count).
pub(crate) fn read(f: &Fetched, dir: &Path, pages: (usize, usize)) -> Reading {
  let ext = if f.bytes.starts_with(b"%PDF-") {
    "pdf".to_owned()
  } else {
    f.path
      .extension()
      .and_then(|e| e.to_str())
      .unwrap_or("")
      .to_ascii_lowercase()
  };
  let kind = if f.mime.is_empty() { "file" } else { &f.mime };
  let saved = if f.local {
    format!("[{} ({kind}, {} bytes)]", f.url, f.bytes.len())
  } else {
    format!(
      "[{kind} from {} ({} bytes), saved to {}]",
      f.url,
      f.bytes.len(),
      f.path.display()
    )
  };
  let plain = |text: String| Reading {
    text,
    images: Vec::new(),
    long: false,
  };
  match ext.as_str() {
    "pdf" => match pdf(&f.bytes, dir, pages) {
      Ok((text, images)) => Reading {
        text: format!("{saved}\n\n{text}"),
        images,
        long: true,
      },
      Err(e) => plain(format!("{saved}\n\nThe PDF could not be read: {e}")),
    },
    "png" | "jpg" | "jpeg" | "gif" | "webp" if f.bytes.len() <= MAX_IMAGE => Reading {
      text: format!("{saved}\n\nThe image follows."),
      images: vec![ToolImage {
        path: f.path.clone(),
        media_type: match ext.as_str() {
          "png" => "image/png",
          "gif" => "image/gif",
          "webp" => "image/webp",
          _ => "image/jpeg",
        }
        .into(),
      }],
      long: false,
    },
    "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => match sheets(&f.bytes) {
      Ok(md) => Reading {
        text: format!("{saved}\n\n{md}"),
        images: Vec::new(),
        long: true,
      },
      Err(e) => plain(format!("{saved}\n\nThe spreadsheet could not be read: {e}")),
    },
    "csv" | "tsv" => match delimited(&f.bytes, if ext == "tsv" { b'\t' } else { b',' }) {
      Ok(md) => Reading {
        text: format!("{saved}\n\n{md}"),
        images: Vec::new(),
        long: true,
      },
      Err(e) => plain(format!("{saved}\n\nThe table could not be read: {e}")),
    },
    _ if f.mime.starts_with("text/")
      || f.mime.contains("json")
      || f.mime.contains("xml")
      || matches!(ext.as_str(), "txt" | "md" | "json" | "xml") =>
    {
      Reading {
        text: format!("{saved}\n\n{}", String::from_utf8_lossy(&f.bytes)),
        images: Vec::new(),
        long: true,
      }
    }
    _ => plain(format!(
      "{saved}\n\nNot shown here; {}",
      if stitch_shell::CAN_SPAWN {
        "use the shell on the file."
      } else {
        "no program here opens it: hand it to another of the user's machines."
      }
    )),
  }
}

/// A PDF's text by page, from `first` (1-based) for `count` pages, with the
/// pages that need seeing rendered as images.
fn pdf(
  bytes: &[u8],
  dir: &Path,
  (first, count): (usize, usize),
) -> Result<(String, Vec<ToolImage>), String> {
  let owned = bytes.to_vec();
  let texts = std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem_by_pages(&owned))
    .map_err(|_| "the PDF reader failed on this file".to_owned())?
    .map_err(|e| e.to_string())?;
  let total = texts.len();
  if total == 0 {
    return Err("it has no pages".into());
  }
  if first > total {
    return Err(format!(
      "it has {total} pages; start_page {first} is past the end"
    ));
  }
  let last = (first + count - 1).min(total);
  let wanted: Vec<usize> = (first..=last)
    .filter(|&n| texts[n - 1].trim().len() < 50)
    .collect();
  let (images, pictured) = render(bytes, dir, first, last, &wanted);
  let mut out = format!("PDF, {total} pages; pages {first}–{last} here");
  if last < total {
    out.push_str(&format!(" (next: start_page {})", last + 1));
  }
  out.push('.');
  for n in first..=last {
    let text = texts[n - 1].trim();
    out.push_str(&format!("\n\n## Page {n}\n\n"));
    if text.is_empty() {
      out.push_str("(no text layer)");
    } else {
      out.push_str(text);
    }
    if pictured.contains(&n) {
      out.push_str(&format!("\n\n(page {n} is also attached as an image)"));
    }
  }
  Ok((out, images))
}

/// Renders the pages in `wanted` and those in `first..=last` with large
/// pictures, up to `MAX_PAGE_IMAGES`; the images and their page numbers.
fn render(
  bytes: &[u8],
  dir: &Path,
  first: usize,
  last: usize,
  wanted: &[usize],
) -> (Vec<ToolImage>, Vec<usize>) {
  let owned = bytes.to_vec();
  let dir = dir.to_owned();
  let wanted = wanted.to_vec();
  std::panic::catch_unwind(move || {
    let mut images = Vec::new();
    let mut pages = Vec::new();
    let Ok(pdf) = hayro::hayro_syntax::Pdf::new(owned) else {
      return (images, pages);
    };
    let cache = hayro::RenderCache::new();
    for (i, page) in pdf.pages().iter().enumerate() {
      let n = i + 1;
      if n < first || n > last || images.len() == MAX_PAGE_IMAGES {
        continue;
      }
      if !wanted.contains(&n) && !has_picture(page) {
        continue;
      }
      let (w, h) = page.render_dimensions();
      let scale = PAGE_PIXELS / w.max(h).max(1.0);
      let pixmap = hayro::render(
        page,
        &cache,
        &hayro::hayro_interpret::InterpreterSettings::default(),
        &hayro::RenderSettings::default(),
        &hayro::PixmapSettings {
          x_scale: scale,
          y_scale: scale,
          bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
      );
      if let Ok(png) = pixmap.into_png()
        && let Ok(path) = save(&dir, &png, "png")
      {
        images.push(ToolImage {
          path,
          media_type: "image/png".into(),
        });
        pages.push(n);
      }
    }
    (images, pages)
  })
  .unwrap_or_default()
}

/// Whether a page shows a picture large enough to matter.
fn has_picture(page: &hayro::hayro_syntax::page::Page<'_>) -> bool {
  let objects = &page.resources().x_objects;
  objects.keys().any(|name| {
    objects
      .get::<hayro::hayro_syntax::object::Stream<'_>>(name.as_ref())
      .is_some_and(|s| {
        let d = s.dict();
        let is_image = d
          .get::<hayro::hayro_syntax::object::Name<'_>>(b"Subtype")
          .is_some_and(|n| n.as_str() == "Image");
        let size = |k: &[u8]| d.get::<f64>(k).unwrap_or(0.0);
        is_image && size(b"Width") * size(b"Height") >= PICTURE_PIXELS
      })
  })
}

fn cell(s: &str) -> String {
  s.replace('|', "\\|")
    .replace(['\r', '\n'], " ")
    .chars()
    .take(1000)
    .collect()
}

/// Rows as a Markdown table, the first as its header.
fn table(rows: &[Vec<String>]) -> String {
  let width = rows
    .iter()
    .map(Vec::len)
    .max()
    .unwrap_or(0)
    .clamp(1, MAX_COLS);
  let line = |r: &[String]| {
    let cells: Vec<String> = (0..width)
      .map(|i| r.get(i).map(|c| cell(c)).unwrap_or_default())
      .collect();
    format!("| {} |", cells.join(" | "))
  };
  let mut out = Vec::new();
  if let Some(head) = rows.first() {
    out.push(line(head));
    out.push(line(&vec!["---".to_owned(); width]));
    out.extend(rows[1..].iter().map(|r| line(r)));
  }
  out.join("\n")
}

fn sheets(bytes: &[u8]) -> Result<String, String> {
  use calamine::Reader;
  let mut book =
    calamine::open_workbook_auto_from_rs(Cursor::new(bytes.to_vec())).map_err(|e| e.to_string())?;
  let names = book.sheet_names();
  let mut out = String::new();
  let mut cut = names.len() > MAX_SHEETS;
  for name in names.iter().take(MAX_SHEETS) {
    let Ok(range) = book.worksheet_range(name) else {
      continue;
    };
    let (h, w) = range.get_size();
    cut |= h > MAX_ROWS || w > MAX_COLS;
    let rows: Vec<Vec<String>> = range
      .rows()
      .take(MAX_ROWS)
      .map(|r| r.iter().take(MAX_COLS).map(ToString::to_string).collect())
      .collect();
    out.push_str(&format!("## {name}\n\n{}\n\n", table(&rows)));
  }
  if cut {
    out.push_str(&format!(
      "(preview: at most {MAX_SHEETS} sheets, {MAX_ROWS} rows and {MAX_COLS} columns each; the whole file is saved)"
    ));
  }
  Ok(out.trim_end().to_owned())
}

fn delimited(bytes: &[u8], delimiter: u8) -> Result<String, String> {
  let mut reader = csv::ReaderBuilder::new()
    .delimiter(delimiter)
    .has_headers(false)
    .flexible(true)
    .from_reader(bytes);
  let mut rows = Vec::new();
  let mut cut = false;
  for record in reader.records() {
    if rows.len() == MAX_ROWS {
      cut = true;
      break;
    }
    let record = record.map_err(|e| e.to_string())?;
    rows.push(record.iter().take(MAX_COLS).map(str::to_owned).collect());
  }
  let mut out = table(&rows);
  if cut {
    out.push_str(&format!(
      "\n\n(preview: the first {MAX_ROWS} rows; the whole file is saved)"
    ));
  }
  Ok(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tables_are_markdown() {
    let md = delimited(b"Name,Count\nApples,7\nPipe|d,\"two\nlines\"\n", b',').unwrap();
    assert_eq!(
      md,
      "| Name | Count |\n| --- | --- |\n| Apples | 7 |\n| Pipe\\|d | two lines |"
    );
    assert_eq!(
      extension("https://x.org/a/report.PDF?x=1", "", b"%PDF-1.4"),
      "pdf"
    );
    assert_eq!(extension("https://x.org/data", "text/csv", b"a,b"), "csv");
    assert_eq!(
      extension("https://x.org/a.tar.gz", "application/gzip", b"\x1f\x8b"),
      "gz"
    );
    assert_eq!(
      extension("https://x.org/", "application/octet-stream", b""),
      "bin"
    );
  }
}
