//! An image's media type from its bytes (design §2.11, "Media types from
//! the bytes"): a file's name, its extension or a picker's word can be
//! wrong (a `Photo.jpg` that is PNG), and providers refuse an image whose
//! label disagrees with its bytes. So where the bytes say what they are,
//! that wins: when an attachment is stored, and again when a request is
//! built, which repairs records stored with a wrong label.

/// The image types every adapter that reads images takes.
pub const PROVIDER_IMAGES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// The media type an image's bytes declare by their signature: PNG, JPEG,
/// GIF, WebP, and HEIC, HEIF and AVIF (ISO base media files, by the brand
/// of their `ftyp` box); `None` when they match none of them.
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
  match bytes {
    [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => Some("image/png"),
    [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
    [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some("image/gif"),
    [
      b'R',
      b'I',
      b'F',
      b'F',
      _,
      _,
      _,
      _,
      b'W',
      b'E',
      b'B',
      b'P',
      ..,
    ] => Some("image/webp"),
    [_, _, _, _, b'f', b't', b'y', b'p', ..] => ftyp(bytes),
    _ => None,
  }
}

/// An ISO base media file's image type, from its major brand, else its
/// compatible brands (`mif1` alone, a HEIF that names no codec, comes last).
fn ftyp(bytes: &[u8]) -> Option<&'static str> {
  let size = u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
  let end = size.clamp(16, 256).min(bytes.len());
  let brand = |b: &[u8]| match b {
    b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"hevm" | b"hevs" => {
      Some("image/heic")
    }
    b"avif" | b"avis" => Some("image/avif"),
    b"heif" | b"mif1" | b"msf1" => Some("image/heif"),
    _ => None,
  };
  let major = bytes.get(8..12).and_then(brand);
  let compatible = || {
    let all: Vec<_> = bytes
      .get(16..end)?
      .as_chunks::<4>()
      .0
      .iter()
      .filter_map(|b| brand(b))
      .collect();
    let specific = all.iter().find(|t| **t != "image/heif");
    specific.or(all.first()).copied()
  };
  match major {
    Some("image/heif") | None => compatible().or(major),
    some => some,
  }
}

/// The type to send an image as: what its bytes say, else its label.
pub fn image_type<'a>(declared: &'a str, bytes: &[u8]) -> &'a str {
  sniff_image(bytes).unwrap_or(declared)
}

/// A note in place of an image no adapter but Gemini reads (HEIC, HEIF,
/// AVIF), so the request goes out and the model knows what it missed.
pub(crate) fn unsendable(media_type: &str, bytes: usize) -> String {
  format!(
    "[Attached image ({media_type}, {bytes} bytes): this model cannot read {media_type} \
     images; attach it as JPEG or PNG.]"
  )
}

/// A PNG's bytes labelled JPEG, as a `Photo.jpg` that is PNG is stored.
#[cfg(test)]
pub(crate) fn png_labelled_jpeg() -> crate::Content {
  crate::Content::Image {
    media_type: "image/jpeg".into(),
    data: vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13],
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn boxed(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
    let size = 16 + 4 * compatible.len() as u32;
    let mut b = size.to_be_bytes().to_vec();
    b.extend_from_slice(b"ftyp");
    b.extend_from_slice(major);
    b.extend_from_slice(&[0, 0, 0, 0]);
    for c in compatible {
      b.extend_from_slice(*c);
    }
    b.extend_from_slice(&[0; 8]);
    b
  }

  #[test]
  fn each_signature() {
    let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
    assert_eq!(sniff_image(&png), Some("image/png"));
    assert_eq!(
      sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0, 0]),
      Some("image/jpeg")
    );
    assert_eq!(sniff_image(b"GIF87a\x01\x00"), Some("image/gif"));
    assert_eq!(sniff_image(b"GIF89a\x01\x00"), Some("image/gif"));
    assert_eq!(sniff_image(b"RIFF\x24\0\0\0WEBPVP8 "), Some("image/webp"));
    assert_eq!(
      sniff_image(&boxed(b"heic", &[b"mif1", b"heic"])),
      Some("image/heic")
    );
    assert_eq!(
      sniff_image(&boxed(b"mif1", &[b"mif1", b"heic"])),
      Some("image/heic")
    );
    assert_eq!(sniff_image(&boxed(b"mif1", &[b"mif1"])), Some("image/heif"));
    assert_eq!(
      sniff_image(&boxed(b"avif", &[b"mif1", b"avif"])),
      Some("image/avif")
    );
  }

  #[test]
  fn not_images() {
    assert_eq!(sniff_image(b""), None);
    assert_eq!(sniff_image(b"\x89PNG"), None);
    assert_eq!(sniff_image(b"GIF88a"), None);
    assert_eq!(sniff_image(b"RIFF\x24\0\0\0WAVEfmt "), None);
    assert_eq!(sniff_image(&boxed(b"isom", &[b"mp41"])), None);
    assert_eq!(sniff_image(b"%PDF-1.7"), None);
  }

  #[test]
  fn the_bytes_win_over_the_label() {
    let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    assert_eq!(image_type("image/jpeg", &png), "image/png");
    assert_eq!(image_type("image/jpeg", b"unknown"), "image/jpeg");
  }
}
