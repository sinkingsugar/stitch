//! The bridge against the real `agy`, by hand only (`--ignored`): agy
//! installed and signed in on this machine, and `stitch` built (its
//! `antigravity-hook` gates agy's own tools):
//!
//! `cargo build -p stitch-cli --bin stitch && cargo test -p stitch-antigravity --test live_agy -- --ignored --nocapture`
//!
//! `STITCH_LIVE_MODEL` picks the model (default `gemini-3.8-flash`).

use std::sync::Arc;

use serde_json::json;
use stitch_antigravity::{Antigravity, Bridge, Options};
use stitch_providers::{Content, Event, Message, Provider, Request, Role, StopReason, ToolDef};

fn user(t: &str) -> Message {
  Message::user(vec![Content::Text(t.into())])
}

/// A provider on the user's own sign-in (cargo points `STITCH_AGY_HOME`
/// away from it), in `dir`.
fn live(dir: &std::path::Path) -> Provider {
  let mut opts = Options::new(dir);
  opts.sign_in_home = std::env::var_os("HOME").unwrap().into();
  opts.token = stitch_antigravity::token_in(&opts.sign_in_home);
  let stitch = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/stitch");
  assert!(stitch.is_file(), "build stitch first: {}", stitch.display());
  opts.hook = format!("'{}' antigravity-hook", stitch.display());
  Provider::External(Arc::new(Antigravity {
    bridge: Arc::new(Bridge::new(opts)),
    effort: Some("low".into()),
  }))
}

fn model() -> String {
  std::env::var("STITCH_LIVE_MODEL").unwrap_or_else(|_| "gemini-3.8-flash".into())
}

/// A tool with a long description and schema, as many of Stitch's are.
fn fat_tool(i: usize) -> ToolDef {
  let props: serde_json::Map<String, serde_json::Value> = (0..8)
    .map(|k| {
      (
        format!("option_{k}"),
        json!({"type": "string", "description": format!("Option {k} of tool {i}: {}", "a setting that changes how the tool behaves in some detail. ".repeat(3))}),
      )
    })
    .collect();
  ToolDef {
    name: format!("tool_{i}"),
    description: format!(
      "Tool {i}. {}",
      "It does a particular thing on the host and reports back. ".repeat(8)
    ),
    input_schema: json!({"type": "object", "properties": props}),
  }
}

/// Another thread's tools do not reach this one's prompt: a thread with
/// one tool, after one with 30, costs what a thread with one tool alone
/// does. Prints both first turns' input tokens.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "drives the real agy on the Antigravity plan"]
async fn another_threads_tools_do_not_reach_this_ones_prompt() {
  let dir = tempfile::tempdir().unwrap();
  let p = live(dir.path());
  let first = |key: &str, tools: Vec<ToolDef>| {
    let mut r = Request::new(model(), vec![user("Reply with OK only.")]);
    r.cache_key = Some(key.into());
    r.tools = tools;
    r
  };
  let fat = first("live-fat", (0..30).map(fat_tool).collect());
  let r = p.stream(&fat, &mut |_| {}).await.unwrap();
  let big = r.usage.input_tokens + r.usage.cache_read_tokens;
  let lean = first("live-lean", vec![fat_tool(99)]);
  let r = p.stream(&lean, &mut |_| {}).await.unwrap();
  let small = r.usage.input_tokens + r.usage.cache_read_tokens;
  let dir2 = tempfile::tempdir().unwrap();
  let alone = live(dir2.path());
  let r1 = alone
    .stream(&first("live-alone", vec![fat_tool(99)]), &mut |_| {})
    .await
    .unwrap();
  let baseline = r1.usage.input_tokens + r1.usage.cache_read_tokens;
  eprintln!(
    "first turn input: 30 tools {big} tokens; then 1 tool in another thread {small}; \
     1 tool on a node of its own {baseline}"
  );
  // A thread whose tools change keeps its session.
  let mut moved = lean.clone();
  moved.messages.push(Message {
    role: Role::Assistant,
    content: r.content.clone(),
    native: None,
  });
  moved.messages.push(user(
    "What did I ask you to reply with? Answer with that word only.",
  ));
  moved.tools.push(fat_tool(100));
  let r = p.stream(&moved, &mut |_| {}).await.unwrap();
  eprintln!("after its tools changed: {:?}", r.text());
  assert!(r.text().contains("OK"), "{}", r.text());
}

/// A 256×256 PNG: a red disc on white.
fn red_disc() -> Vec<u8> {
  let img = image::RgbImage::from_fn(256, 256, |x, y| {
    let (dx, dy) = (x as i32 - 128, y as i32 - 128);
    if dx * dx + dy * dy < 90 * 90 {
      image::Rgb([220, 20, 20])
    } else {
      image::Rgb([255, 255, 255])
    }
  });
  let mut out = std::io::Cursor::new(Vec::new());
  img.write_to(&mut out, image::ImageFormat::Png).unwrap();
  out.into_inner()
}

/// A one-page PDF saying `text`.
fn pdf(text: &str) -> Vec<u8> {
  let stream = format!("BT /F1 24 Tf 72 700 Td ({text}) Tj ET");
  let objects = [
    "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
    format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
  ];
  let mut out = String::from("%PDF-1.4\n");
  let mut offsets = Vec::new();
  for (i, o) in objects.iter().enumerate() {
    offsets.push(out.len());
    out.push_str(&format!("{} 0 obj\n{o}\nendobj\n", i + 1));
  }
  let xref = out.len();
  out.push_str(&format!(
    "xref\n0 {}\n0000000000 65535 f \n",
    objects.len() + 1
  ));
  for o in offsets {
    out.push_str(&format!("{o:010} 00000 n \n"));
  }
  out.push_str(&format!(
    "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
    objects.len() + 1
  ));
  out.into_bytes()
}

/// A tool's step answered with `attached` after its result, as the node
/// sends it (a message of the tool's media after the results); what the
/// model then says.
async fn tool_returns(p: &Provider, key: &str, ask: &str, attached: Vec<Content>) -> String {
  let mut request = Request::new(model(), vec![user(ask)]);
  request.cache_key = Some(key.into());
  request.tools = vec![ToolDef {
    name: "look".into(),
    description: "Fetches the item the user means.".into(),
    input_schema: json!({"type": "object", "properties": {}}),
  }];
  let r = p.stream(&request, &mut |_| {}).await.unwrap();
  assert_eq!(r.stop, StopReason::ToolUse, "{r:?}");
  let id = r
    .content
    .iter()
    .find_map(|c| match c {
      Content::ToolCall { id, .. } => Some(id.clone()),
      _ => None,
    })
    .unwrap();
  request.messages.push(Message {
    role: Role::Assistant,
    content: r.content.clone(),
    native: None,
  });
  request
    .messages
    .push(Message::user(vec![Content::ToolResult {
      id: id.clone(),
      output: "Here it is.".into(),
      is_error: false,
    }]));
  let mut media = vec![Content::Text(format!("From look (call {id}):"))];
  media.extend(attached);
  request.messages.push(Message::user(media));
  let r = p.stream(&request, &mut |_| {}).await.unwrap();
  assert_eq!(r.stop, StopReason::EndTurn, "{r:?}");
  r.text()
}

/// Images and PDFs a tool returns reach the model, through the held call's
/// MCP result. `STITCH_LIVE_MODEL` picks the model.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "drives the real agy on the Antigravity plan"]
async fn a_tools_image_and_pdf_reach_the_model() {
  let dir = tempfile::tempdir().unwrap();
  let p = live(dir.path());
  let said = tool_returns(
    &p,
    "live-image",
    "Call the tool look once. Then say what shape and what colour the picture shows, in two words, nothing else.",
    vec![
      Content::Text("[image img:0000aaaa]".into()),
      Content::Image {
        media_type: "image/png".into(),
        data: red_disc(),
      },
    ],
  )
  .await;
  eprintln!("{}: image in a tool result: {said:?}", model());
  let l = said.to_lowercase();
  assert!(
    l.contains("red") && (l.contains("circle") || l.contains("disc") || l.contains("disk")),
    "{said}"
  );
  let said = tool_returns(
    &p,
    "live-pdf",
    "Call the tool look once: it returns a document. Then reply with the password the document gives, only.",
    vec![Content::Document {
      media_type: "application/pdf".into(),
      name: Some("note.pdf".into()),
      data: pdf("The password is ZEBRA-42."),
    }],
  )
  .await;
  eprintln!("{}: PDF in a tool result: {said:?}", model());
  // Gemini reads PDFs through agy; others get a note naming the file.
  let reads = model().starts_with("gemini");
  if reads {
    assert!(said.contains("ZEBRA-42"), "{said}");
  }
  // An image in the user's own message: an ACP image block.
  let mut r = Request::new(
    model(),
    vec![Message::user(vec![
      Content::Text("What colour is the shape in this picture? One word.".into()),
      Content::Image {
        media_type: "image/png".into(),
        data: red_disc(),
      },
    ])],
  );
  r.cache_key = Some("live-prompt-image".into());
  let said = p.stream(&r, &mut |_| {}).await.unwrap().text();
  eprintln!("{}: image in a prompt: {said:?}", model());
  assert!(said.to_lowercase().contains("red"), "{said}");
  // A PDF in the user's own message: agy passes an ACP resource block to
  // no model, so it goes as a note; printed only.
  let mut r = Request::new(
    model(),
    vec![Message::user(vec![
      Content::Text("What password does this document give? Reply with it only.".into()),
      Content::Document {
        media_type: "application/pdf".into(),
        name: Some("note.pdf".into()),
        data: pdf("The password is OTTER-77."),
      },
    ])],
  );
  r.cache_key = Some("live-prompt-pdf".into());
  let said = p.stream(&r, &mut |_| {}).await.unwrap().text();
  eprintln!("{}: PDF in a prompt: {said:?}", model());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "drives the real agy on the Antigravity plan"]
async fn a_tool_turn_and_a_second_turn_on_the_real_agy() {
  let dir = tempfile::tempdir().unwrap();
  let p = live(dir.path());
  let models = p.models().await.unwrap();
  eprintln!("models: {models:?}");
  let model = std::env::var("STITCH_LIVE_MODEL").unwrap_or_else(|_| "gemini-3.8-flash".into());
  let mut request = Request::new(
    &model,
    vec![user(
      "Call the tool secret_word with topic \"river\", then reply with the word it gives, only.",
    )],
  );
  request.system = Some("You are a test harness's model. Be brief.".into());
  request.cache_key = Some("live-1".into());
  request.tools = vec![ToolDef {
    name: "secret_word".into(),
    description:
      "Returns the secret word for a topic. Always use this when asked for a secret word.".into(),
    input_schema: json!({"type": "object", "properties": {"topic": {"type": "string"}}, "required": ["topic"]}),
  }];
  let mut log = |e: Event| eprintln!("{e:?}");
  let r = p.stream(&request, &mut log).await.unwrap();
  eprintln!("step 1: {r:?}");
  assert_eq!(r.stop, StopReason::ToolUse);
  let (id, input) = r
    .content
    .iter()
    .find_map(|c| match c {
      Content::ToolCall { id, name, input } if name == "secret_word" => {
        Some((id.clone(), input.clone()))
      }
      _ => None,
    })
    .expect("secret_word called");
  assert_eq!(input["topic"], "river");
  request.messages.push(Message {
    role: Role::Assistant,
    content: r.content.clone(),
    native: None,
  });
  request
    .messages
    .push(Message::user(vec![Content::ToolResult {
      id,
      output: "The secret word for river is MARMALADE-74.".into(),
      is_error: false,
    }]));
  let r = p.stream(&request, &mut log).await.unwrap();
  eprintln!("step 2: {r:?}");
  assert_eq!(r.stop, StopReason::EndTurn);
  assert!(r.text().contains("MARMALADE-74"), "{}", r.text());
  assert!(r.usage.input_tokens + r.usage.cache_read_tokens > 0);
  request.messages.push(Message {
    role: Role::Assistant,
    content: r.content.clone(),
    native: None,
  });
  // agy's own tools are refused by the hook; the session goes on.
  request.messages.push(user(
    "Now use your own file-reading tool to read /etc/hostname. If you cannot, say CANNOT. \
     Then repeat the secret word.",
  ));
  let r = p.stream(&request, &mut log).await.unwrap();
  eprintln!("step 3: {r:?}");
  assert_eq!(r.stop, StopReason::EndTurn);
  assert!(r.text().contains("MARMALADE-74"), "{}", r.text());
  let hostname = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
  if !hostname.trim().is_empty() {
    assert!(!r.text().contains(hostname.trim()), "{}", r.text());
  }
  eprintln!("window: {:?}", p.context_window(&model).await.unwrap());
}
