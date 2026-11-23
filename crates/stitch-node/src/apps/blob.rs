//! The web client's image blobs (design §2.24): `GET /_stitch/blob/<hash>`
//! shows an image of the user's threads (a message's, a tool's), and `POST
//! /_stitch/blob?name=<file name>` stores a file the user pasted, dropped or picked in the
//! composer, answering its hash, which `Request::Chat` takes as an
//! attachment (`blake3:…`).
//!
//! **Gate.** What every request here passes (the Host check, the cookie, in
//! `http`), and the origin must be the pin of the built-in client: another
//! app's origin or a version's (`show_app`) is refused, whatever its
//! cookie. A page of another origin (`Origin`, `Sec-Fetch-Site`) is refused
//! too, and a POST must carry its own `Origin`.
//!
//! **Images inline, anything else a download.** An image is decided by its
//! bytes (the magic numbers of PNG, JPEG, GIF, WebP and the HEIF family),
//! never by a label, and shown with that type, `X-Content-Type-Options:
//! nosniff` and `Cross-Origin-Resource-Policy: same-origin`. Every other
//! blob (an SVG, HTML, a PDF, any file the user attached; up to 50 MB) is
//! answered as `application/octet-stream` with `Content-Disposition:
//! attachment`, `nosniff` and `Content-Security-Policy: sandbox`, so the
//! page can never be made to run something the store holds.

use std::sync::Arc;

use stitch_records::BlobHash;

use super::serve::Response;
use crate::node::Shared;

const BASE: &str = "/_stitch/blob";
/// Largest file taken from the composer.
pub(crate) const MAX_UPLOAD: usize = 50 << 20;

pub(crate) fn is_blob_path(path: &str) -> bool {
  path == BASE || path.starts_with("/_stitch/blob/")
}

fn refused(status: u16, why: &str) -> Response {
  Response::text(status, why).header("X-Content-Type-Options", "nosniff")
}

fn image(bytes: Vec<u8>, kind: &str) -> Response {
  Response::new(200, kind, bytes)
    .header("X-Content-Type-Options", "nosniff")
    .header("Cache-Control", "private, max-age=31536000, immutable")
    .header("Cross-Origin-Resource-Policy", "same-origin")
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn serve(
  s: &Arc<Shared>,
  name: &str,
  host: &str,
  method: &str,
  path: &str,
  origin: Option<&str>,
  site: Option<&str>,
  body: Vec<u8>,
) -> Response {
  let own = format!("http://{host}");
  if !super::ws::is_client_host(s, name)
    || origin.is_some_and(|o| o != own)
    || site.is_some_and(|f| f != "same-origin")
  {
    return refused(403, "the node's images are for the web client alone");
  }
  let (path, query) = path.split_once('?').unwrap_or((path, ""));
  match (method, path.strip_prefix("/_stitch/blob/")) {
    ("GET" | "HEAD", Some(hash)) => get(s, hash, query).await,
    ("POST", None) if path == BASE => {
      if origin.is_none() {
        return refused(403, "not from this app");
      }
      post(s, body, query)
    }
    _ => refused(405, "GET /_stitch/blob/<hash>, or POST /_stitch/blob"),
  }
}

async fn get(s: &Arc<Shared>, hash: &str, query: &str) -> Response {
  let Ok(hash) = hash.parse::<BlobHash>() else {
    return refused(404, "no such file");
  };
  if !s.state().blobs.contains(&hash) {
    crate::node::fetch_blobs(s, &[hash]).await;
  }
  let Ok(bytes) = std::fs::read(s.state().blobs.path(&hash)) else {
    return refused(404, "no such file");
  };
  match stitch_providers::media::sniff_image(&bytes) {
    Some(kind) => image(bytes, kind),
    None => download(bytes, name_of(query).as_deref()),
  }
}

/// Anything else is only ever a download: never run by the page.
fn download(bytes: Vec<u8>, name: Option<&str>) -> Response {
  let name = name.map(clean_name).unwrap_or_else(|| "file".into());
  let ascii: String = name
    .chars()
    .map(|c| {
      if c.is_ascii_alphanumeric() || "._- ".contains(c) {
        c
      } else {
        '_'
      }
    })
    .collect();
  let encoded: String = url::form_urlencoded::byte_serialize(name.as_bytes())
    .collect::<String>()
    .replace('+', "%20")
    .replace('*', "%2A");
  Response::new(200, "application/octet-stream", bytes)
    .header(
      "Content-Disposition",
      format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"),
    )
    .header("X-Content-Type-Options", "nosniff")
    .header("Content-Security-Policy", "sandbox")
    .header("Cache-Control", "private, max-age=31536000, immutable")
    .header("Cross-Origin-Resource-Policy", "same-origin")
}

use super::clean_name;

fn name_of(query: &str) -> Option<String> {
  url::form_urlencoded::parse(query.as_bytes())
    .find(|(k, _)| k == "name")
    .map(|(_, v)| v.into_owned())
}

fn post(s: &Arc<Shared>, body: Vec<u8>, query: &str) -> Response {
  if body.len() > MAX_UPLOAD {
    return refused(413, "a file of at most 50 MB");
  }
  if body.is_empty() {
    return refused(400, "an empty file");
  }
  let kind = match stitch_providers::media::sniff_image(&body) {
    Some(k) => k.to_owned(),
    None => crate::node::file_type(name_of(query).as_deref(), &body),
  };
  match s.state().blobs.put_bytes(&body) {
    Ok(hash) => Response::json(&serde_json::json!({
      "hash": hash.to_string(),
      "type": kind,
      "bytes": body.len(),
    })),
    Err(e) => refused(500, &e.to_string()),
  }
}
