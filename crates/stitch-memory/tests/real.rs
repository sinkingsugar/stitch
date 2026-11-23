//! Memory with the real EmbeddingGemma 2 (set `STITCH_EMBED_MODEL`, and
//! `STITCH_EMBED_PROJECTOR` for images; without them the test passes
//! saying so): what was said comes back to a paraphrased question, an
//! unrelated question brings nothing, and an image is recalled by words.

use std::path::PathBuf;

use stitch_memory::{
  At, Encoder, Fact, Fold, ModelFiles, PROPOSER_TOP, ambient_ceiling, handle, pack,
};

fn observe(place: &str, v: &[f32]) -> Fact {
  Fact::Observe {
    place: place.into(),
    embedding: pack(v),
    demand: 0.0,
    own: false,
    supersedes: false,
  }
}

const SAID: &[&str] = &[
  "We decided the lead sheet export renders chords above the lyrics, one page per song, as a PDF.",
  "The aeroponic misting pump runs fifteen seconds every fifteen minutes; the reservoir is changed weekly.",
  "Stitch syncs records between nodes over iroh, reconciling ID ranges by fingerprint.",
  "My sister's birthday dinner is at the Thai place on Via Roma, Saturday at eight.",
  "The invoice for the October consulting work went out on the third, net thirty days.",
  "To rotate the server's TLS certificate, run the renew script and restart nginx.",
];

#[test]
fn what_was_said_comes_back_to_a_question() {
  let Some(model) = std::env::var_os("STITCH_EMBED_MODEL").map(PathBuf::from) else {
    eprintln!("STITCH_EMBED_MODEL not set: skipped");
    return;
  };
  let projector = std::env::var_os("STITCH_EMBED_PROJECTOR").map(PathBuf::from);
  let enc = Encoder::new(
    ModelFiles {
      model,
      projector: projector.clone(),
    },
    true,
    true,
    8192,
  );
  let mut fold = Fold::new();
  for (i, text) in SAID.iter().enumerate() {
    let v = enc.document(text).unwrap();
    fold.apply(
      &At::new(format!("r{i:03}"), 0, 0),
      &observe(&format!("home:t#m{i}"), &v),
    );
  }
  let ask = |q: &str| {
    let cue = enc.query(q).unwrap();
    fold.recall(&cue, &|_| true)
  };
  for (q, want) in [
    ("how did we set up the chord sheet PDF export?", 0),
    ("how often does the mister spray the plants?", 1),
    ("when is the birthday dinner and where?", 3),
    ("how do I renew the HTTPS certificate?", 5),
  ] {
    let hits = ask(q);
    assert_eq!(
      hits.first().map(|h| h.handle),
      Some(handle(&format!("home:t#m{want}"))),
      "{q}: {hits:?}"
    );
  }
  let nothing = ask("what is the boiling point of tungsten?");
  assert!(nothing.is_empty(), "unrelated: {nothing:?}");

  // Unasked, the user's message (embedded as it will be stored) is the
  // cue, under the ceiling its novelty sets: a close match is volunteered,
  // an unrelated message volunteers nothing.
  let unasked = |text: &str| {
    let cue = enc.document(text).unwrap();
    let novelty = fold.peek("home:u#m1", &cue, 0.0).novelty;
    let (ceiling, _) = ambient_ceiling(novelty);
    fold.cortex(&cue, ceiling, PROPOSER_TOP, &|_| true)
  };
  let volunteered = unasked("remind me what we decided for the lead sheet chord export layout");
  eprintln!("volunteered: {volunteered:?}");
  assert_eq!(
    volunteered.first().map(|h| h.0),
    Some(handle("home:t#m0")),
    "{volunteered:?}"
  );
  let quiet = unasked("I'm thinking of repainting the kitchen a pale olive green next spring");
  assert!(quiet.is_empty(), "{quiet:?}");

  if projector.is_some() {
    let mut red = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
      256,
      256,
      image::Rgb([220, 20, 20]),
    ))
    .write_to(&mut red, image::ImageFormat::Png)
    .unwrap();
    let v = enc.image(red.get_ref()).unwrap();
    fold.apply(&At::new("r100", 0, 0), &observe("home:t#m9:0", &v));
    let cue = enc.query("a plain red square").unwrap();
    let hits = fold.recall(&cue, &|_| true);
    eprintln!("red square: {hits:?}");
    assert_eq!(hits.first().map(|h| h.handle), Some(handle("home:t#m9:0")));
  }
}

/// A phone's encoder (a 2048-token context, suspended in the background):
/// it refuses while suspended, works again once resumed, and encodes what
/// fits as a desktop's (8192) does, so the nodes' memories agree.
#[test]
fn a_phones_encoder_suspends_and_agrees() {
  let Some(model) = std::env::var_os("STITCH_EMBED_MODEL").map(PathBuf::from) else {
    eprintln!("STITCH_EMBED_MODEL not set: skipped");
    return;
  };
  let files = ModelFiles {
    model,
    projector: None,
  };
  let phone = Encoder::new(files.clone(), false, true, 2048);
  let desktop = Encoder::new(files, false, true, 8192);
  let before = phone.document(SAID[2]).unwrap();
  phone.suspend(true);
  assert!(phone.suspended());
  assert!(phone.document(SAID[2]).is_err());
  phone.suspend(false);
  let after = phone.document(SAID[2]).unwrap();
  assert_eq!(before, after);
  let wide = desktop.document(SAID[2]).unwrap();
  let cos: f32 = after.iter().zip(&wide).map(|(a, b)| a * b).sum();
  assert!(cos > 0.999, "phone and desktop differ: {cos}");
}
