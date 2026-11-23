//! The libraries every app can load from `/_lib/<file>`, offline: kept
//! gzipped in the binary (`applibs/`, listed in `libs_list.rs`), sent as
//! they are to a browser that takes gzip, inflated for the phone's web view.
//!
//! Three.js and the leadsheet stack (its loader `leadsheet.js` is Stitch's,
//! edge talk's `leadsheetLoader`) are ES modules imported by bare names
//! (`"three"`, `"leadsheet"`, `"spessasynth"`): every page without an import
//! map of its own gets one to `/_lib/`. On the phone too: edge
//! talk inlined the modules as `data:` URIs because WebKit's module loader
//! did not reach its scheme handler, but its pages had no origin then; an
//! app's page has its own, and its modules load through the handler.

use std::io::Read;

use super::serve::{Client, Response};

include!("libs_list.rs");

/// A library's bytes, inflated.
pub(super) fn inflate(name: &str) -> Option<Vec<u8>> {
  let (_, gz) = LIBS.iter().find(|(n, _)| *n == name)?;
  let mut out = Vec::new();
  flate2::read::GzDecoder::new(*gz)
    .read_to_end(&mut out)
    .ok()?;
  Some(out)
}

/// The leadsheet stack's loader: Stitch's own, not a vendored file.
const LEADSHEET: &str = include_str!("leadsheet.js");

pub(super) fn serve(name: &str, client: Client) -> Response {
  if name == "leadsheet.js" {
    return Response {
      status: 200,
      headers: vec![(
        "Content-Type".into(),
        "text/javascript; charset=utf-8".into(),
      )],
      body: LEADSHEET.as_bytes().to_vec(),
    }
    .header("Cache-Control", "no-cache");
  }
  let Some((_, gz)) = LIBS.iter().find(|(n, _)| *n == name) else {
    return Response {
      status: 404,
      headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
      body: format!(
        "no bundled library {name}; there are: {}",
        LIBS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
      )
      .into_bytes(),
    };
  };
  let media_type = super::serve::media_type(name);
  let r = match client {
    Client::Browser { gzip: true } => Response {
      status: 200,
      headers: vec![
        ("Content-Type".into(), media_type.into()),
        ("Content-Encoding".into(), "gzip".into()),
      ],
      body: gz.to_vec(),
    },
    _ => Response {
      status: 200,
      headers: vec![("Content-Type".into(), media_type.into())],
      body: inflate(name).unwrap_or_default(),
    },
  };
  r.header("Cache-Control", "public, max-age=86400")
}

/// The bare names a page may import, and the files they are.
const MODULES: &[(&str, &str)] = &[
  ("three", "three.module.min.js"),
  (
    "three/addons/controls/OrbitControls.js",
    "OrbitControls.min.js",
  ),
  (
    "three/addons/controls/OrbitControls",
    "OrbitControls.min.js",
  ),
  ("leadsheet", "leadsheet_wasm.js"),
  ("spessasynth", "spessasynth.min.js"),
];

/// The import map of the bundled modules, for every page that has none of
/// its own (a document takes one, and it must come before any module).
pub(super) fn import_map(page: &str) -> Option<String> {
  if page.contains("type=\"importmap\"") || page.contains("type='importmap'") {
    return None;
  }
  let imports: serde_json::Map<String, serde_json::Value> = MODULES
    .iter()
    .map(|(name, file)| ((*name).into(), format!("/_lib/{file}").into()))
    .collect();
  Some(format!(
    "<script type=\"importmap\">{}</script>",
    serde_json::json!({ "imports": imports })
  ))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn libraries_inflate_and_three_gets_a_map() {
    assert!(LIBS.len() > 20);
    let chart = inflate("chart.min.js").unwrap();
    assert!(chart.len() > 100_000);
    assert_eq!(serve("nope.js", Client::App).status, 404);
    let gz = serve("chart.min.js", Client::Browser { gzip: true });
    assert_eq!(gz.get("Content-Encoding"), Some("gzip"));
    assert_eq!(serve("chart.min.js", Client::App).body, chart);

    let page = "<script type=\"module\">import * as THREE from 'three';</script>";
    assert!(
      import_map(page)
        .unwrap()
        .contains("\"three\":\"/_lib/three.module.min.js\"")
    );
    assert!(
      import_map("<p>music</p>")
        .unwrap()
        .contains("\"leadsheet\":\"/_lib/leadsheet_wasm.js\"")
    );
    assert!(import_map(&format!("{page}<script type=\"importmap\">{{}}</script>")).is_none());
    let loader = serve("leadsheet.js", Client::App);
    assert!(String::from_utf8_lossy(&loader.body).contains("window.initLeadsheet"));
    // MeshRepair: our own (the meshrepair repository's build, MIT OR
    // Apache-2.0), the API apps use, its wasm from /_lib/.
    let mr = String::from_utf8(inflate("meshrepair.min.js").unwrap()).unwrap();
    assert!(
      mr.contains("'/_lib/meshrepair.wasm'"),
      "the wasm from /_lib/"
    );
    assert!(mr.contains("initMeshRepair"));
    let loader = String::from_utf8(inflate("meshrepair-loader.min.js").unwrap()).unwrap();
    assert!(loader.contains("loadMeshRepair"));
    for (name, text) in [
      ("meshrepair.min.js", &mr),
      ("meshrepair-loader.min.js", &loader),
    ] {
      assert!(text.contains("MIT OR Apache-2.0"), "{name}: our license");
      assert!(!text.contains("GPL"), "{name}: no GPL code");
    }
    let wasm = inflate("meshrepair.wasm").unwrap();
    assert_eq!(&wasm[..4], b"\0asm");
    assert!(
      !wasm.windows(7).any(|w| w == b"vcglib/" || w == b"VCGLIB_"),
      "not the VCGlib build"
    );
    assert!(inflate("GeneralUserGS.sf3").unwrap().len() > 8_000_000);
  }
}
