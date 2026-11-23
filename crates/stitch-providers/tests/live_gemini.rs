//! Gemini against the real API, by hand only (`--ignored`): a key from
//! `GEMINI_API_KEY` or `GOOGLE_API_KEY`, or from a dotenv file named by
//! `STITCH_TEST_GEMINI_ENV` (its `GEMINI_API_KEY=` line). Nothing of the
//! key is printed.
//!
//! `cargo test -p stitch-providers --test live_gemini -- --ignored --nocapture`
//! (`STITCH_TEST_GEMINI_MODEL`, `STITCH_TEST_GEMINI_IMAGE_MODEL` pick the
//! models; the cheapest Flash and image models listed otherwise).

use serde_json::json;
use stitch_providers::{Content, Event, Gemini, Message, Provider, Request, Role, ToolDef};

fn key() -> Option<String> {
  for v in ["GEMINI_API_KEY", "GOOGLE_API_KEY"] {
    if let Ok(k) = std::env::var(v)
      && !k.is_empty()
    {
      return Some(k);
    }
  }
  let file = std::env::var("STITCH_TEST_GEMINI_ENV").ok()?;
  let text = std::fs::read_to_string(file).ok()?;
  text.lines().find_map(|l| {
    let v = l.trim().strip_prefix("GEMINI_API_KEY=")?;
    Some(v.trim().trim_matches('"').trim_matches('\'').to_owned())
  })
}

fn provider() -> Option<Gemini> {
  key().map(Gemini::new)
}

/// A second of a 440 Hz tone, 16 kHz mono WAV, padded with silence to
/// `bytes` at least.
fn tone(bytes: usize) -> Vec<u8> {
  let rate = 16_000u32;
  let samples = (rate as usize).max(bytes / 2);
  let mut pcm = Vec::with_capacity(samples * 2);
  for i in 0..samples {
    let v = if i < rate as usize {
      ((i as f64 * 440.0 * std::f64::consts::TAU / f64::from(rate)).sin() * 12_000.0) as i16
    } else {
      0
    };
    pcm.extend_from_slice(&v.to_le_bytes());
  }
  let mut w = Vec::new();
  w.extend_from_slice(b"RIFF");
  w.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
  w.extend_from_slice(b"WAVEfmt ");
  w.extend_from_slice(&16u32.to_le_bytes());
  w.extend_from_slice(&1u16.to_le_bytes());
  w.extend_from_slice(&1u16.to_le_bytes());
  w.extend_from_slice(&rate.to_le_bytes());
  w.extend_from_slice(&(rate * 2).to_le_bytes());
  w.extend_from_slice(&2u16.to_le_bytes());
  w.extend_from_slice(&16u16.to_le_bytes());
  w.extend_from_slice(b"data");
  w.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
  w.extend_from_slice(&pcm);
  w
}

/// A 2×2 red PNG.
fn tone_png() -> Vec<u8> {
  use base64::Engine;
  base64::engine::general_purpose::STANDARD
    .decode("iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEklEQVR4nGP4z8DAwMDAxAADAA4OAgFH8f0hAAAAAElFTkSuQmCC")
    .unwrap()
}

async fn pick(p: &Provider, var: &str, want: impl Fn(&str) -> bool) -> String {
  if let Ok(m) = std::env::var(var) {
    return m;
  }
  let models = p.models().await.expect("models");
  models
    .iter()
    .rev()
    .find(|m| want(m))
    .cloned()
    .unwrap_or_else(|| panic!("no model to test among {models:?}"))
}

#[tokio::test]
#[ignore = "calls the Gemini API"]
async fn live_gemini() {
  let Some(g) = provider() else {
    eprintln!("no Gemini key: skipped");
    return;
  };
  let p = Provider::Gemini(g.clone());
  let models = p.models().await.expect("models");
  eprintln!("{} models: {models:?}", models.len());
  let model = pick(&p, "STITCH_TEST_GEMINI_MODEL", |m| {
    m.contains("flash") && !m.contains("image") && !m.contains("lite") && !m.contains("preview")
  })
  .await;
  eprintln!("chat model: {model}");
  eprintln!("window: {:?}", p.context_window(&model).await);

  // A tool call, its result, and the answer: replayed natively (with
  // the call's signature), then from content alone (another model's).
  let tool = ToolDef {
    name: "get_weather".into(),
    description: "The weather in a city.".into(),
    input_schema: json!({"type": "object", "properties": {"city": {"type": "string"}},
      "required": ["city"], "additionalProperties": false}),
  };
  let mut req = Request::new(
    model.clone(),
    vec![Message::user(vec![Content::Text(
      "What's the weather in Paris? Use the tool.".into(),
    )])],
  );
  req.system = Some("Be brief.".into());
  req.tools = vec![tool];
  let mut events = Vec::new();
  let r = p.stream(&req, &mut |e| events.push(e)).await.expect("call");
  eprintln!("step 1: {:?} {:?} {:?}", r.stop, r.content, r.usage);
  let (id, call_seen) = r
    .content
    .iter()
    .find_map(|c| match c {
      Content::ToolCall { id, .. } => Some((id.clone(), true)),
      _ => None,
    })
    .expect("a tool call");
  assert!(call_seen);
  assert!(
    events
      .iter()
      .any(|e| matches!(e, Event::ToolCallStart { .. }))
  );
  req.messages.push(Message {
    role: Role::Assistant,
    content: r.content.clone(),
    native: r.native.clone(),
  });
  req.messages.push(Message::user(vec![Content::ToolResult {
    id,
    output: "Sunny, 21°C".into(),
    is_error: false,
  }]));
  let r2 = p.stream(&req, &mut |_| {}).await.expect("native replay");
  eprintln!("step 2 (native): {:?}", r2.text());
  assert!(r2.text().contains("21"));
  // The same turn with an image a tool made for it (`generate_image`),
  // carried after the function response.
  {
    let mut with_image = req.clone();
    let turn = &mut with_image.messages[1].content;
    turn.push(Content::Text(
      "[image img:000000000000 `generated.png`]".into(),
    ));
    turn.push(Content::Image {
      media_type: "image/png".into(),
      data: tone_png(),
    });
    with_image.messages.push(Message::user(vec![Content::Text(
      "What colour is the image made for your turn?".into(),
    )]));
    let r = p
      .stream(&with_image, &mut |_| {})
      .await
      .expect("a carried image");
    eprintln!("carried image: {:?}", r.text());
  }
  for m in &mut req.messages {
    m.native = None;
  }
  let r3 = p
    .stream(&req, &mut |_| {})
    .await
    .expect("replay from content");
  eprintln!("step 2 (content): {:?}", r3.text());

  // Audio, inline and through the Files API.
  for inline_max in [usize::MAX, 0] {
    let mut g = g.clone();
    g.inline_max = inline_max;
    let p = Provider::Gemini(g);
    let req = Request::new(
      model.clone(),
      vec![Message::user(vec![
        Content::Document {
          media_type: "audio/wav".into(),
          name: Some("tone.wav".into()),
          data: tone(0),
        },
        Content::Text("Describe this audio in five words.".into()),
      ])],
    );
    let r = p.stream(&req, &mut |_| {}).await.expect("audio");
    eprintln!(
      "audio ({}): {:?}",
      if inline_max == 0 { "file" } else { "inline" },
      r.text()
    );
    assert!(!r.text().is_empty());
  }

  // An image made, then edited in a second turn from the first's parts.
  let image_model = pick(&p, "STITCH_TEST_GEMINI_IMAGE_MODEL", |m| {
    m.contains("flash") && m.contains("image") && !m.contains("preview")
  })
  .await;
  eprintln!("image model: {image_model}");
  let mut req = Request::new(
    image_model.clone(),
    vec![Message::user(vec![Content::Text(
      "Draw a small red circle on a white background.".into(),
    )])],
  );
  req.tools = vec![];
  let r = p.stream(&req, &mut |_| {}).await.expect("image");
  let images = r
    .content
    .iter()
    .filter(|c| matches!(c, Content::Image { .. }))
    .count();
  eprintln!(
    "image: {images} image(s), {:?}, {:?}, {:?}",
    r.text(),
    r.stop,
    r.usage
  );
  assert!(images > 0);
  req.messages.push(Message {
    role: Role::Assistant,
    content: r.content.clone(),
    native: r.native.clone(),
  });
  req.messages.push(Message::user(vec![Content::Text(
    "Now make the circle blue.".into(),
  )]));
  let r = p.stream(&req, &mut |_| {}).await.expect("image edit");
  eprintln!(
    "edit: {} image(s)",
    r.content
      .iter()
      .filter(|c| matches!(c, Content::Image { .. }))
      .count()
  );

  // `generate_image`'s backend: made, then edited from the image made.
  let made = p
    .generate_image(&stitch_providers::images::ImageRequest {
      model: image_model.clone(),
      prompt: "A small green triangle on a white background.".into(),
      images: vec![],
      aspect_ratio: Some("16:9".into()),
    })
    .await
    .expect("generate_image");
  eprintln!(
    "generate_image: {} image(s), {:?}",
    made.images.len(),
    made.usage
  );
  if let Some(first) = made.images.first().cloned() {
    let edited = p
      .generate_image(&stitch_providers::images::ImageRequest {
        model: image_model.clone(),
        prompt: "Make the triangle purple.".into(),
        images: vec![first],
        aspect_ratio: None,
      })
      .await
      .expect("generate_image edit");
    eprintln!("generate_image edit: {} image(s)", edited.images.len());
  }

  // A thread that used tools on another model goes on on the image model,
  // which takes none.
  let req = Request::new(
    image_model,
    vec![
      Message::user(vec![Content::Text("What's the date?".into())]),
      Message {
        role: Role::Assistant,
        content: vec![Content::ToolCall {
          id: "toolu_1".into(),
          name: "shell_run".into(),
          input: json!({"command": "date"}),
        }],
        native: None,
      },
      Message::user(vec![Content::ToolResult {
        id: "toolu_1".into(),
        output: "Sun Oct 18 2026".into(),
        is_error: false,
      }]),
      Message {
        role: Role::Assistant,
        content: vec![Content::Text("It is October 18.".into())],
        native: None,
      },
      Message::user(vec![Content::Text(
        "Draw a calendar page for that date.".into(),
      )]),
    ],
  );
  let r = p
    .stream(&req, &mut |_| {})
    .await
    .expect("image after tools");
  eprintln!(
    "after tools: {} image(s)",
    r.content
      .iter()
      .filter(|c| matches!(c, Content::Image { .. }))
      .count()
  );
}
