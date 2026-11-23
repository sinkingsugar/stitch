//! `generate_image` (design §2.8, Images): any thread's model asks for an
//! image, made by an image backend of the node's providers (Gemini's image
//! models for now, `stitch_providers::images`). What it makes becomes the
//! calling turn's reply's attachments, as an image model's own images do
//! (the loop puts them there, `ToolOutcome::generated`), so the user gets
//! them in the reply and they replay to whatever model goes on, for edits.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use stitch_providers::ToolDef;
use stitch_providers::images::{ASPECT_RATIOS, ImageRequest};
use stitch_records::Attachment;
use stitch_store::BlobStore;

use crate::config::Config;
use crate::tools::{Spent, ToolOutcome};

/// The image model when `[images] model` names none.
pub const DEFAULT_MODEL: &str = "gemini/gemini-3.1-flash-image";

/// Images given to the backend are fitted to this, as every image sent.
const INPUT_SIDE: u32 = crate::images::MAX_SIDE;

/// The image model `generate_image` uses on this node (`provider/model`),
/// or `None` when none can be asked (no key for its provider, or a model
/// that makes no images): then the tool is not offered.
pub(crate) fn model(config: &Config, data: &std::path::Path) -> Option<String> {
  let model = config
    .images
    .as_ref()
    .and_then(|i| i.model.clone())
    .unwrap_or_else(|| DEFAULT_MODEL.into());
  let (provider, _, m) = config.resolve(data, Some(&model)).ok()?;
  (provider.has_key() && provider.makes_images(&m)).then_some(model)
}

pub(crate) fn definition(model: &str) -> ToolDef {
  ToolDef {
    name: "generate_image".into(),
    description: format!(
      "Make an image from a prompt, or change images: name them by their IDs (`img:…`: the \
       user's, a tool's, or ones this tool made) in `images` and say what to change. The \
       images it makes are shown to the user in your reply, as files they keep, and come \
       back to you with their IDs, so a later call can edit them again; the result gives each image's file on disk. Uses {model} unless \
       `model` names another image model (`provider/model`)."
    ),
    input_schema: json!({
      "type": "object", "additionalProperties": false,
      "properties": {
        "prompt": {"type": "string", "description": "What to make, or what to change in `images`."},
        "images": {"type": "array", "items": {"type": "string"},
          "description": "IDs (`img:…`) of images to edit or to work from, in order."},
        "aspect_ratio": {"type": "string", "enum": ASPECT_RATIOS},
        "model": {"type": "string", "description": "Another image model, `provider/model`."}
      },
      "required": ["prompt"]
    }),
  }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
  prompt: String,
  #[serde(default)]
  images: Vec<String>,
  #[serde(default)]
  aspect_ratio: Option<String>,
  #[serde(default)]
  model: Option<String>,
}

/// The media type of image bytes, by their signature.
pub(crate) fn sniff(bytes: &[u8]) -> Option<&'static str> {
  if bytes.starts_with(b"\x89PNG") {
    Some("image/png")
  } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
    Some("image/jpeg")
  } else if bytes.starts_with(b"GIF8") {
    Some("image/gif")
  } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
    Some("image/webp")
  } else {
    None
  }
}

/// The extension a file of `media_type` takes.
pub(crate) fn extension(media_type: &str) -> &'static str {
  match media_type {
    "image/jpeg" => "jpg",
    "image/webp" => "webp",
    "image/gif" => "gif",
    _ => "png",
  }
}

/// Runs a call: the images named read from the blob store and fitted, the
/// backend asked, what it made stored as blobs.
pub(crate) async fn generate(
  shared: Option<Arc<crate::node::Shared>>,
  blobs: BlobStore,
  default: String,
  input: Value,
) -> ToolOutcome {
  let Some(s) = shared else {
    return ToolOutcome::err("the node is stopping");
  };
  let req: Input = match serde_json::from_value(input) {
    Ok(r) => r,
    Err(e) => return ToolOutcome::err(e.to_string()),
  };
  if let Some(r) = &req.aspect_ratio
    && !ASPECT_RATIOS.contains(&r.as_str())
  {
    return ToolOutcome::err(format!(
      "aspect_ratio is one of {}",
      ASPECT_RATIOS.join(", ")
    ));
  }
  let full = req.model.clone().unwrap_or(default);
  let (provider, name, model) = match s.config.resolve_authenticated(&s.data, Some(&full)).await {
    Ok(r) => r,
    Err(e) => return ToolOutcome::err(e.to_string()),
  };
  if !provider.makes_images(&model) {
    return ToolOutcome::err(format!("{full} makes no images: name an image model"));
  }
  let mut images = Vec::new();
  for id in &req.images {
    let Some(hash) = crate::images::resolve(&blobs, id) else {
      return ToolOutcome::err(format!(
        "no image {id} here: use the `img:…` ID shown with it"
      ));
    };
    let bytes = match std::fs::read(blobs.path(&hash)) {
      Ok(b) => b,
      Err(e) => return ToolOutcome::err(format!("{id}: {e}")),
    };
    let Some(media_type) = sniff(&bytes) else {
      return ToolOutcome::err(format!("{id} is not an image (PNG, JPEG, GIF or WebP)"));
    };
    let fitted = match crate::images::shrink(&bytes, INPUT_SIDE) {
      Some(f) => (f.media_type, f.data),
      None => (media_type.to_owned(), bytes),
    };
    images.push(fitted);
  }
  let request = ImageRequest {
    model: model.clone(),
    prompt: req.prompt,
    images,
    aspect_ratio: req.aspect_ratio,
  };
  let made = match provider.generate_image(&request).await {
    Ok(m) => m,
    Err(e) => return ToolOutcome::err(format!("{full}: {e}")),
  };
  let spent = Some(Spent {
    model: format!("{name}/{}", made.model),
    usage: made.usage,
    usd: None,
  });
  if made.images.is_empty() {
    let mut out = ToolOutcome::err(format!(
      "{full} made no image{}",
      if made.text.trim().is_empty() {
        String::new()
      } else {
        format!("; it said: {}", made.text.trim())
      }
    ));
    out.spent = spent;
    return out;
  }
  let mut generated = Vec::new();
  let mut lines = Vec::new();
  for (media_type, data) in made.images {
    let media_type = stitch_providers::media::image_type(&media_type, &data).to_owned();
    let hash = match blobs.put_bytes(&data) {
      Ok(h) => h,
      Err(e) => return ToolOutcome::err(e.to_string()),
    };
    let id = crate::images::short_id(&hash);
    let size = crate::images::dimensions(&data)
      .map(|(w, h)| format!(", {w}×{h}"))
      .unwrap_or_default();
    lines.push(format!(
      "{id}{size}, file {} ({media_type})",
      blobs.path(&hash).display()
    ));
    generated.push(Attachment {
      name: Some(format!(
        "generated-{}.{}",
        &hash.hex()[..8],
        extension(&media_type)
      )),
      hash,
      media_type,
      bytes: data.len() as u64,
    });
  }
  let mut output = format!(
    "Made with {full}: {}. Shown to the user in your reply; the image follows. To change it, \
     call generate_image again with its ID in `images`. Each file is the node's stored copy: \
     to use one with shell tools, copy it (`cp <path> ./name.jpg`), never change it in place.",
    lines.join("; ")
  );
  if !made.text.trim().is_empty() {
    output.push_str(&format!("\nThe image model said: {}", made.text.trim()));
  }
  ToolOutcome {
    ok: true,
    output,
    generated,
    spent,
    ..ToolOutcome::default()
  }
}
