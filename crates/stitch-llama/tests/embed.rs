//! EmbeddingGemma 2 through llama.cpp, against the real model. Set
//! `STITCH_EMBED_MODEL` to its GGUF (and `STITCH_EMBED_PROJECTOR` to the
//! mmproj GGUF for images); without them the tests say so and pass.

use std::path::PathBuf;

use stitch_llama::{Embedder, Options, cosine};

fn model() -> Option<PathBuf> {
  let p = PathBuf::from(std::env::var_os("STITCH_EMBED_MODEL")?);
  p.is_file().then_some(p)
}

fn projector() -> Option<PathBuf> {
  let p = PathBuf::from(std::env::var_os("STITCH_EMBED_PROJECTOR")?);
  p.is_file().then_some(p)
}

const QUERY: &str = "task: search result | query: ";
const DOC: &str = "title: none | text: ";

#[test]
fn texts_embed_by_meaning() {
  let Some(m) = model() else {
    eprintln!("STITCH_EMBED_MODEL not set: skipped");
    return;
  };
  let mut e = Embedder::load(&m, None, &Options::default()).unwrap();
  assert_eq!(e.dimensions(), 768);
  let q = e
    .embed(&format!("{QUERY}What causes the northern lights?"))
    .unwrap();
  let hit = e
    .embed(&format!(
      "{DOC}The northern lights are caused by charged particles from the sun hitting the atmosphere."
    ))
    .unwrap();
  let miss = e
    .embed(&format!(
      "{DOC}Boil the pasta in salted water for nine minutes."
    ))
    .unwrap();
  let norm: f32 = q.iter().map(|x| x * x).sum::<f32>().sqrt();
  assert!((norm - 1.0).abs() < 1e-4, "unit length: {norm}");
  let (a, b) = (cosine(&q, &hit), cosine(&q, &miss));
  assert!(a > b + 0.1, "related {a} vs unrelated {b}");

  // The same input gives the same vector, as memory needs.
  let again = e
    .embed(&format!("{QUERY}What causes the northern lights?"))
    .unwrap();
  assert_eq!(q, again);

  // The CPU agrees with the GPU closely.
  let mut cpu = Embedder::load(
    &m,
    None,
    &Options {
      gpu: false,
      ..Options::default()
    },
  )
  .unwrap();
  let on_cpu = cpu
    .embed(&format!("{QUERY}What causes the northern lights?"))
    .unwrap();
  assert!(cosine(&q, &on_cpu) > 0.99, "{}", cosine(&q, &on_cpu));

  // Long text is cut at the context, not refused.
  let long = "word ".repeat(20_000);
  assert_eq!(e.embed(&long).unwrap().len(), 768);
  assert!(e.embed("").is_err());
  assert!(e.embed_image(&[1, 2, 3], None).is_err(), "no projector");
}

fn square(rgb: [u8; 3]) -> Vec<u8> {
  let img = image::RgbImage::from_pixel(256, 256, image::Rgb(rgb));
  let mut out = std::io::Cursor::new(Vec::new());
  image::DynamicImage::ImageRgb8(img)
    .write_to(&mut out, image::ImageFormat::Png)
    .unwrap();
  out.into_inner()
}

#[test]
fn images_share_the_text_space() {
  let (Some(m), Some(p)) = (model(), projector()) else {
    eprintln!("STITCH_EMBED_MODEL or STITCH_EMBED_PROJECTOR not set: skipped");
    return;
  };
  let mut e = Embedder::load(&m, Some(&p), &Options::default()).unwrap();
  assert!(e.sees());
  let red = e.embed_image(&square([220, 20, 20]), None).unwrap();
  let blue = e.embed_image(&square([20, 40, 220]), None).unwrap();
  assert_eq!(red.len(), 768);
  let says_red = e.embed(&format!("{QUERY}a plain red square")).unwrap();
  let says_blue = e.embed(&format!("{QUERY}a plain blue square")).unwrap();
  assert!(
    cosine(&says_red, &red) > cosine(&says_red, &blue),
    "red text: red {} blue {}",
    cosine(&says_red, &red),
    cosine(&says_red, &blue)
  );
  assert!(
    cosine(&says_blue, &blue) > cosine(&says_blue, &red),
    "blue text: blue {} red {}",
    cosine(&says_blue, &blue),
    cosine(&says_blue, &red)
  );
  assert!(e.embed_image(b"not an image", None).is_err());

  // A phone screenshot's size (what crashed an iPhone at full size) is
  // scaled down and embeds like a small one.
  let tall = encode(image::RgbImage::from_pixel(
    1206,
    2622,
    image::Rgb([220, 20, 20]),
  ));
  let v = e.embed_image(&tall, None).unwrap();
  assert!(cosine(&v, &red) > 0.9, "{}", cosine(&v, &red));
}

fn encode(img: image::RgbImage) -> Vec<u8> {
  let mut out = std::io::Cursor::new(Vec::new());
  image::DynamicImage::ImageRgb8(img)
    .write_to(&mut out, image::ImageFormat::Png)
    .unwrap();
  out.into_inner()
}

#[test]
fn large_images_are_scaled_down_for_the_projector() {
  let tall = encode(image::RgbImage::from_pixel(
    1206,
    2622,
    image::Rgb([1, 2, 3]),
  ));
  let fitted = stitch_llama::fit_image(&tall).unwrap();
  assert_eq!(fitted.height(), stitch_llama::IMAGE_SIDE);
  assert_eq!(fitted.width(), 353);
  // At most the projector's token cap (48 px a token).
  assert!(fitted.width().div_ceil(48) * fitted.height().div_ceil(48) <= 256);
  // Smaller ones stay as they are.
  let small = stitch_llama::fit_image(&square([1, 2, 3])).unwrap();
  assert_eq!((small.width(), small.height()), (256, 256));
  assert!(stitch_llama::fit_image(b"not an image").is_none());
}
