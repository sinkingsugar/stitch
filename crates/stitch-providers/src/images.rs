//! Image generation behind one shape (design §2.8, Images), for the
//! node's `generate_image` tool: a prompt, the images it works from (to
//! edit or to follow), an aspect ratio. Each provider kind that makes
//! images answers it its own way; Gemini's image models are the first.
//! Another backend (OpenAI's Images API, xAI's) is one more arm in
//! [`Provider::generate_image`] and [`Provider::makes_images`]; the node's
//! tool, its result and the app's image views stay as they are.

use crate::{Content, Error, Provider, Result, Usage};

/// What to make.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageRequest {
  /// The provider's model, without the provider's name.
  pub model: String,
  pub prompt: String,
  /// Images to work from, in order: `(media type, bytes)`.
  pub images: Vec<(String, Vec<u8>)>,
  /// `1:1`, `16:9`, …; `None` leaves the model's.
  pub aspect_ratio: Option<String>,
}

/// What came back.
#[derive(Clone, Debug, PartialEq)]
pub struct Generated {
  pub model: String,
  /// `(media type, bytes)`, in the order made.
  pub images: Vec<(String, Vec<u8>)>,
  /// What the model said with them.
  pub text: String,
  pub usage: Usage,
}

/// The aspect ratios the tool offers (Gemini's).
pub const ASPECT_RATIOS: &[&str] = &[
  "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9",
];

impl Provider {
  /// Whether `model` of this provider makes images.
  pub fn makes_images(&self, model: &str) -> bool {
    match self {
      Provider::Gemini(_) => crate::gemini::makes_images(model),
      _ => false,
    }
  }

  /// Makes images, where this kind of provider can.
  pub async fn generate_image(&self, r: &ImageRequest) -> Result<Generated> {
    let response = match self {
      Provider::Gemini(p) => p.generate_image(r).await?,
      other => {
        return Err(Error::Protocol {
          provider: other.kind(),
          message: "this kind of provider makes no images".into(),
        });
      }
    };
    let text = response.text();
    let images = response
      .content
      .into_iter()
      .filter_map(|c| match c {
        Content::Image { media_type, data } => Some((media_type, data)),
        _ => None,
      })
      .collect();
    Ok(Generated {
      model: response.model,
      images,
      text,
      usage: response.usage,
    })
  }
}
