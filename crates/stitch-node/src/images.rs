//! Images for the model (design §2.3, "Seeing and asking"): scaled down
//! where they enter a thread (a tool's image is stored at most `MAX_SIDE`
//! on its long side, its original kept beside it for `zoom_image`), fitted
//! again when sent (a few MB, `MAX_SIDE`; older records hold larger ones),
//! fitted to a provider's stricter limit when it names one, and cropped
//! from the original when the model zooms in.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Mutex;

use image::{DynamicImage, GenericImageView, ImageFormat};
use stitch_providers::{Content, Message};

/// Larger images are scaled down before they are sent.
const MAX_BYTES: usize = 3_500_000;
/// The long side of any image a model gets, and of a zoomed-in region at
/// most: what every provider takes. Anthropic's general limit is 8000 px,
/// but above 20 images and documents in one request every image must fit a
/// stricter one (2000 px in its documentation, 3000 seen in its errors);
/// OpenAI's models take larger ones and scale them down themselves.
pub const MAX_SIDE: u32 = 2000;

/// An encoded image and its media type.
type Encoded = (Vec<u8>, &'static str);

/// An image as fitted for a model, and the sizes it went from and to when
/// it was scaled.
#[derive(Clone, Debug, PartialEq)]
pub struct Fitted {
  pub data: Vec<u8>,
  pub media_type: String,
  pub scaled: Option<Scaled>,
}

/// A scaling: from the original's width and height to the new ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scaled {
  pub from: (u32, u32),
  pub to: (u32, u32),
}

impl Scaled {
  /// The factor, new over original.
  pub fn factor(&self) -> f64 {
    f64::from(self.to.0.max(self.to.1)) / f64::from(self.from.0.max(self.from.1).max(1))
  }

  /// How it reads to the model: `3000×1000 scaled to 2000×667 (×0.67)`.
  pub fn describe(&self) -> String {
    format!(
      "{}×{} scaled to {}×{} (×{:.2})",
      self.from.0,
      self.from.1,
      self.to.0,
      self.to.1,
      self.factor()
    )
  }
}

/// Fitted images by blob hash: a thread sends its images again each turn.
static FITTED: Mutex<Option<HashMap<String, Fitted>>> = Mutex::new(None);

/// The image as providers take it: unchanged when small enough, else
/// scaled down to `MAX_SIDE` (PNG when it has transparency, else JPEG).
/// `key` caches the result for a stored image.
/// An image sent unchanged goes as the type its bytes say, whatever it
/// was labelled.
pub fn fit(bytes: Vec<u8>, media_type: &str, key: Option<&str>) -> Fitted {
  let media_type = stitch_providers::media::image_type(media_type, &bytes).to_owned();
  let unchanged = |data: Vec<u8>| Fitted {
    data,
    media_type: media_type.clone(),
    scaled: None,
  };
  if bytes.len() <= MAX_BYTES
    && let Ok(size) = imagesize(&bytes)
    && size.0.max(size.1) <= MAX_SIDE
  {
    return unchanged(bytes);
  }
  if let Some(k) = key
    && let Some(f) = FITTED
      .lock()
      .expect("fitted lock")
      .as_ref()
      .and_then(|m| m.get(k))
  {
    return f.clone();
  }
  let Ok(img) = image::load_from_memory(&bytes) else {
    return unchanged(bytes);
  };
  let from = img.dimensions();
  let img = scale(img, MAX_SIDE);
  let Ok(((data, t), to)) = encode_within(img) else {
    return unchanged(bytes);
  };
  let fitted = Fitted {
    data,
    media_type: t.to_owned(),
    scaled: (from != to).then_some(Scaled { from, to }),
  };
  if let Some(k) = key {
    let mut cache = FITTED.lock().expect("fitted lock");
    let map = cache.get_or_insert_with(HashMap::new);
    if map.len() >= 32 {
      map.clear();
    }
    map.insert(k.to_owned(), fitted.clone());
  }
  fitted
}

/// The image scaled down so its long side is at most `side`, when it is
/// larger and readable; `None` leaves it as it is.
pub fn shrink(bytes: &[u8], side: u32) -> Option<Fitted> {
  let size = imagesize(bytes).ok()?;
  if size.0.max(size.1) <= side {
    return None;
  }
  let img = image::load_from_memory(bytes).ok()?;
  let from = img.dimensions();
  let img = scale(img, side);
  let ((data, t), to) = encode_within(img).ok()?;
  Some(Fitted {
    data,
    media_type: t.to_owned(),
    scaled: Some(Scaled { from, to }),
  })
}

/// The long side a provider's refusal asks images to fit: Anthropic's
/// "… exceed max allowed size for many-image requests: 2000 pixels".
pub fn many_image_limit(message: &str) -> Option<u32> {
  let rest = &message[message.find("many-image requests")?..];
  let digits: String = rest
    .chars()
    .skip_while(|c| !c.is_ascii_digit())
    .take_while(char::is_ascii_digit)
    .collect();
  digits.parse().ok().filter(|n| *n > 0)
}

/// Every image of `messages` scaled down to `side` on its long side at
/// most; how many were.
pub fn clamp(messages: &mut [Message], side: u32) -> usize {
  let mut n = 0;
  for m in messages {
    for c in &mut m.content {
      if let Content::Image { media_type, data } = c
        && let Some(f) = shrink(data, side)
      {
        *data = f.data;
        *media_type = f.media_type;
        n += 1;
      }
    }
  }
  n
}

fn imagesize(bytes: &[u8]) -> Result<(u32, u32), image::ImageError> {
  image::ImageReader::new(Cursor::new(bytes))
    .with_guessed_format()?
    .into_dimensions()
}

fn scale(img: DynamicImage, side: u32) -> DynamicImage {
  let (w, h) = img.dimensions();
  if w.max(h) <= side {
    img
  } else {
    img.resize(side, side, image::imageops::FilterType::Lanczos3)
  }
}

/// The image encoded to at most `MAX_BYTES`: an opaque image with an alpha
/// channel goes as JPEG, else the long side steps down (×0.75, a few
/// times) and the last try drops alpha. Also the size it ended at.
fn encode_within(mut img: DynamicImage) -> Result<(Encoded, (u32, u32)), String> {
  if img.color().has_alpha() && img.to_rgba8().pixels().all(|p| p.0[3] == 255) {
    img = DynamicImage::ImageRgb8(img.into_rgb8());
  }
  for step in 0..8 {
    let last = step == 7;
    let size = img.dimensions();
    let encoded = if last {
      encode(DynamicImage::ImageRgb8(img.to_rgb8()))?
    } else {
      encode(img.clone())?
    };
    if encoded.0.len() <= MAX_BYTES || last {
      return Ok((encoded, size));
    }
    let side = (f64::from(size.0.max(size.1)) * 0.75) as u32;
    img = img.resize(side, side, image::imageops::FilterType::Triangle);
  }
  unreachable!("the last step returns")
}

fn encode(img: DynamicImage) -> Result<Encoded, String> {
  let mut out = Cursor::new(Vec::new());
  if img.color().has_alpha() {
    img
      .write_to(&mut out, ImageFormat::Png)
      .map_err(|e| e.to_string())?;
    Ok((out.into_inner(), "image/png"))
  } else {
    img
      .into_rgb8()
      .write_to(&mut out, ImageFormat::Jpeg)
      .map_err(|e| e.to_string())?;
    Ok((out.into_inner(), "image/jpeg"))
  }
}

/// The region of an image between `left`, `top`, `right` and `bottom`
/// (fractions of its width and height), at the original's resolution up to
/// `MAX_SIDE`; its pixel size too.
pub fn crop(bytes: &[u8], region: [f64; 4]) -> Result<(Encoded, (u32, u32)), String> {
  let [l, t, r, b] = region;
  if !(0.0..1.0).contains(&l) || !(0.0..1.0).contains(&t) || r <= l || b <= t || r > 1.0 || b > 1.0
  {
    return Err("the region needs 0 ≤ left < right ≤ 1 and 0 ≤ top < bottom ≤ 1".into());
  }
  let img = image::load_from_memory(bytes).map_err(|e| format!("not an image I can read: {e}"))?;
  let (w, h) = img.dimensions();
  let x = (l * w as f64).floor() as u32;
  let y = (t * h as f64).floor() as u32;
  let cw = (((r - l) * w as f64).ceil() as u32).clamp(1, w - x);
  let ch = (((b - t) * h as f64).ceil() as u32).clamp(1, h - y);
  let region = scale(img.crop_imm(x, y, cw, ch), MAX_SIDE);
  let size = region.dimensions();
  Ok((encode(region)?, size))
}

/// The short ID images go by for the model: `img:` and the start of the
/// blob hash.
pub fn short_id(hash: &stitch_records::BlobHash) -> String {
  format!("img:{}", &hash.hex()[..12])
}

/// The stored image a short ID (or a full hash) names.
pub fn resolve(blobs: &stitch_store::BlobStore, id: &str) -> Option<stitch_records::BlobHash> {
  let id = id.trim();
  if let Ok(full) = id.parse::<stitch_records::BlobHash>() {
    return blobs.contains(&full).then_some(full);
  }
  let hex = id.strip_prefix("img:").unwrap_or(id).to_ascii_lowercase();
  if hex.len() < 8 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
    return None;
  }
  // Blobs live in `blake3/<2 hex>/<2 hex>/<hex>`; any stored hash shows
  // where that is.
  let probe: stitch_records::BlobHash = format!("blake3:{hex:0<64}").parse().ok()?;
  let dir = blobs.path(&probe).parent()?.to_owned();
  let mut found = std::fs::read_dir(dir)
    .ok()?
    .flatten()
    .filter_map(|e| e.file_name().into_string().ok())
    .filter(|n| n.starts_with(&hex) && n.len() == 64);
  let one = found.next()?;
  // An ambiguous prefix names nothing.
  if found.next().is_some() {
    return None;
  }
  format!("blake3:{one}").parse().ok()
}

/// An image's width and height.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
  imagesize(bytes).ok()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn png(w: u32, h: u32) -> Vec<u8> {
    let img = DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
      image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
    }));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
  }

  #[test]
  fn a_noisy_transparent_image_is_fitted_under_the_size_limit() {
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    let img = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(2000, 1500, |_, _| {
      x ^= x << 13;
      x ^= x >> 7;
      x ^= x << 17;
      let b = x.to_le_bytes();
      image::Rgba([b[0], b[1], b[2], 128 | b[3]])
    }));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    let big = out.into_inner();
    assert!(big.len() > MAX_BYTES);
    let fitted = fit(big, "image/png", None);
    assert!(fitted.data.len() <= MAX_BYTES, "{}", fitted.data.len());
    let (w, h) = dimensions(&fitted.data).unwrap();
    assert!(image::load_from_memory(&fitted.data).is_ok());
    if let Some(s) = fitted.scaled {
      assert_eq!(s.to, (w, h));
    }
  }

  #[test]
  fn a_png_labelled_jpeg_is_sent_as_png() {
    let small = png(40, 30);
    assert_eq!(
      fit(small.clone(), "image/jpeg", None).media_type,
      "image/png"
    );
    let big = png(3000, 10);
    assert_eq!(fit(big, "image/png", None).media_type, "image/jpeg");
  }

  #[test]
  fn large_images_are_fitted_and_regions_cropped() {
    let small = png(40, 30);
    let same = fit(small.clone(), "image/png", None);
    assert_eq!((same.data, same.media_type), (small, "image/png".into()));
    assert_eq!(same.scaled, None);
    let big = png(5000, 1000);
    let fitted = fit(big.clone(), "image/png", Some("k"));
    assert_eq!(fitted.media_type, "image/jpeg");
    assert_eq!(dimensions(&fitted.data), Some((2000, 400)));
    let scaled = fitted.scaled.unwrap();
    assert_eq!((scaled.from, scaled.to), ((5000, 1000), (2000, 400)));
    assert_eq!(scaled.describe(), "5000×1000 scaled to 2000×400 (×0.40)");
    // Cached: the same bytes come back.
    assert_eq!(fit(big.clone(), "image/png", Some("k")), fitted);
    // Above the stricter many-image limit (3000 seen from Anthropic), under
    // the old 4000: fitted too.
    let wide = fit(png(3200, 800), "image/png", None);
    assert_eq!(dimensions(&wide.data), Some((2000, 500)));

    let ((region, _), size) = crop(&big, [0.5, 0.0, 0.6, 0.5]).unwrap();
    assert_eq!(size, (500, 500), "full resolution");
    assert_eq!(dimensions(&region), Some((500, 500)));
    assert!(crop(&big, [0.6, 0.0, 0.5, 1.0]).is_err());
    assert!(crop(b"nope", [0.0, 0.0, 1.0, 1.0]).is_err());
  }

  #[test]
  fn a_providers_many_image_limit_is_read_and_applied() {
    let said = "messages.2.content.2.image.source.base64.data: At least one of the image \
                dimensions exceed max allowed size for many-image requests: 3000 pixels";
    assert_eq!(many_image_limit(said), Some(3000));
    assert_eq!(many_image_limit("prompt is too long: 3000 tokens"), None);
    let mut messages = vec![Message::user(vec![
      Content::Text("look".into()),
      Content::Image {
        media_type: "image/png".into(),
        data: png(1800, 900),
      },
      Content::Image {
        media_type: "image/png".into(),
        data: png(600, 300),
      },
    ])];
    assert_eq!(clamp(&mut messages, 1000), 1);
    let Content::Image { data, media_type } = &messages[0].content[1] else {
      panic!()
    };
    assert_eq!(dimensions(data), Some((1000, 500)));
    assert_eq!(media_type, "image/jpeg");
    let Content::Image { data, .. } = &messages[0].content[2] else {
      panic!()
    };
    assert_eq!(dimensions(data), Some((600, 300)), "small ones stay");
    assert!(shrink(b"not an image", 10).is_none());
  }
}
