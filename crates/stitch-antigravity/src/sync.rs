//! What of a thread's messages an agy session already holds, and what to
//! send it next: the pure part of the bridge.
//!
//! A session reflects a prefix of Stitch's messages, kept as one
//! fingerprint per message. A request whose messages start with that
//! prefix goes on in the session: the answer to the call agy is waiting
//! on, or a new prompt of the user messages after it. Anything else (a
//! rewind, a compaction, messages from another provider, a turn cut off)
//! starts a fresh session, its first prompt carrying the earlier
//! conversation as a transcript.

use base64::Engine as _;
use serde_json::{Value, json};
use stitch_providers::{Content, Message, Role};

/// The most of one tool result a transcript carries.
const RESULT_MAX: usize = 8_000;

/// A message's fingerprint: what stays the same when the node rebuilds
/// the message from its records. The model's turns by their text and the
/// IDs of their calls (the text is what this bridge returned); the user's
/// by the IDs of the results they carry, and their place in the sequence
/// (their text may gain notes as they are rebuilt: a pause's time, recalled
/// memories).
pub fn fingerprint(m: &Message) -> u64 {
  let mut h = blake3::Hasher::new();
  match m.role {
    Role::Assistant => {
      h.update(b"a\0");
      let text: String = m
        .content
        .iter()
        .filter_map(|c| match c {
          Content::Text(t) => Some(t.as_str()),
          _ => None,
        })
        .collect();
      h.update(text.trim().as_bytes());
      for c in &m.content {
        if let Content::ToolCall { id, .. } = c {
          h.update(b"\0c");
          h.update(id.as_bytes());
        }
      }
    }
    Role::User => {
      h.update(b"u\0");
      for c in &m.content {
        if let Content::ToolResult { id, .. } = c {
          h.update(b"\0r");
          h.update(id.as_bytes());
        }
      }
    }
  }
  let bytes = h.finalize();
  u64::from_le_bytes(bytes.as_bytes()[..8].try_into().expect("8 bytes"))
}

pub fn fingerprints(messages: &[Message]) -> Vec<u64> {
  messages.iter().map(fingerprint).collect()
}

/// A block of a prompt (ACP's `ContentBlock`).
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
  Text(String),
  Image { media_type: String, data: Vec<u8> },
}

impl Block {
  pub fn to_acp(&self) -> Value {
    match self {
      Block::Text(t) => json!({"type": "text", "text": t}),
      Block::Image { media_type, data } => json!({
        "type": "image",
        "mimeType": media_type,
        "data": base64::engine::general_purpose::STANDARD.encode(data),
      }),
    }
  }
}

/// What a request asks of the session.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
  /// The result of the call agy waits on: answer it and stream on.
  Answer { output: String, is_error: bool },
  /// A new prompt in the same session.
  Prompt(Vec<Block>),
  /// A fresh session, its first prompt these blocks; why, for the log.
  Fresh { blocks: Vec<Block>, why: String },
}

/// What to do with `messages`, given the fingerprints the session holds
/// (`None`: no session) and the call agy waits on, if any.
pub fn plan(synced: Option<&[u64]>, pending: Option<&str>, messages: &[Message]) -> Plan {
  let fresh = |why: &str| Plan::Fresh {
    blocks: fresh_prompt(messages),
    why: why.to_owned(),
  };
  let Some(synced) = synced else {
    return fresh("no session");
  };
  let fps = fingerprints(messages);
  if !fps.starts_with(synced) {
    return fresh("the conversation is not the session's");
  }
  let rest = &messages[synced.len()..];
  if rest.is_empty() {
    return fresh("nothing new to send");
  }
  if rest.iter().any(|m| m.role == Role::Assistant) {
    return fresh("a model's turn the session did not make");
  }
  match pending {
    Some(call) => {
      let mut answer = None;
      let mut extra = Vec::new();
      for (i, m) in rest.iter().enumerate() {
        for c in &m.content {
          match c {
            Content::ToolResult {
              id,
              output,
              is_error,
            } if i == 0 && id == call && answer.is_none() => {
              answer = Some((output.clone(), *is_error));
            }
            Content::ToolResult { .. } => return fresh("a result of a call agy did not make"),
            other => extra.push(other.clone()),
          }
        }
      }
      let Some((mut output, is_error)) = answer else {
        return fresh("the waiting call got no result");
      };
      let said = text_of(&extra);
      if !said.trim().is_empty() {
        output
          .push_str("\n\n[Not part of the tool's output: a message that arrived while it ran.]\n");
        output.push_str(said.trim());
      }
      Plan::Answer { output, is_error }
    }
    None => {
      if rest
        .iter()
        .flat_map(|m| &m.content)
        .any(|c| matches!(c, Content::ToolResult { .. }))
      {
        return fresh("results of calls agy is not waiting on");
      }
      Plan::Prompt(blocks_of(rest))
    }
  }
}

/// The first prompt of a fresh session: the conversation before the last
/// run of the user's messages as a transcript, then those messages.
pub fn fresh_prompt(messages: &[Message]) -> Vec<Block> {
  let mut start = messages.len();
  while start > 0 {
    let m = &messages[start - 1];
    if m.role == Role::User
      && !m
        .content
        .iter()
        .any(|c| matches!(c, Content::ToolResult { .. }))
    {
      start -= 1;
    } else {
      break;
    }
  }
  let (earlier, new) = messages.split_at(start);
  let mut blocks = Vec::new();
  if !earlier.is_empty() {
    blocks.push(Block::Text(transcript(earlier)));
  }
  if new.is_empty() {
    blocks.push(Block::Text("Go on from where the transcript ends.".into()));
  } else {
    blocks.extend(blocks_of(new));
  }
  blocks
}

/// Messages as a transcript in text: who said what, the calls made and
/// their results (long ones cut).
pub fn transcript(messages: &[Message]) -> String {
  let mut names = std::collections::HashMap::new();
  for m in messages {
    for c in &m.content {
      if let Content::ToolCall { id, name, .. } = c {
        names.insert(id.as_str(), name.as_str());
      }
    }
  }
  let mut out = String::from(
    "[Stitch: this conversation began before this session. What was said so far, as a \
     transcript; the conversation goes on after it.]\n",
  );
  for m in messages {
    let who = match m.role {
      Role::User => "User",
      Role::Assistant => "Assistant",
    };
    let mut text = Vec::new();
    for c in &m.content {
      match c {
        Content::ToolCall { name, input, .. } => {
          flush(&mut out, who, &mut text);
          out.push_str(&format!(
            "\nAssistant called the tool `{name}` with {input}\n"
          ));
        }
        Content::ToolResult {
          id,
          output,
          is_error,
        } => {
          flush(&mut out, who, &mut text);
          let name = names.get(id.as_str()).copied().unwrap_or("a tool");
          let failed = if *is_error { " (failed)" } else { "" };
          out.push_str(&format!(
            "\nResult of `{name}`{failed}:\n{}\n",
            cut(output, RESULT_MAX)
          ));
        }
        other => text.push(other.clone()),
      }
    }
    flush(&mut out, who, &mut text);
  }
  out.push_str("\n[End of the transcript.]");
  out
}

fn flush(out: &mut String, who: &str, text: &mut Vec<Content>) {
  let t = text_of(text);
  text.clear();
  if !t.trim().is_empty() {
    out.push_str(&format!("\n{who}:\n{}\n", t.trim()));
  }
}

/// `s` cut to at most `max` bytes (at a character boundary), saying so.
fn cut(s: &str, max: usize) -> String {
  if s.len() <= max {
    return s.to_owned();
  }
  let mut end = max;
  while !s.is_char_boundary(end) {
    end -= 1;
  }
  format!("{}\n[… {} more bytes cut]", &s[..end], s.len() - end)
}

/// Content as text: images and files named, not carried.
pub fn text_of(content: &[Content]) -> String {
  let mut parts = Vec::new();
  for c in content {
    match c {
      Content::Text(t) => parts.push(t.clone()),
      Content::Image { media_type, .. } => parts.push(format!("[an image ({media_type})]")),
      Content::Document {
        media_type, name, ..
      }
      | Content::File {
        media_type, name, ..
      } => parts.push(format!(
        "[attached `{}` ({media_type}): this model cannot read it]",
        name.as_deref().unwrap_or("a file")
      )),
      Content::ToolCall { name, input, .. } => {
        parts.push(format!("[called `{name}` with {input}]"))
      }
      Content::ToolResult { output, .. } => parts.push(output.clone()),
    }
  }
  parts.join("\n\n")
}

/// The user's messages as prompt blocks: text joined, images as images.
pub fn blocks_of(messages: &[Message]) -> Vec<Block> {
  let mut blocks: Vec<Block> = Vec::new();
  let mut text = Vec::new();
  for m in messages {
    for c in &m.content {
      match c {
        Content::Image { media_type, data }
          if matches!(
            media_type.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
          ) =>
        {
          if !text.is_empty() {
            blocks.push(Block::Text(text_of(&text)));
            text.clear();
          }
          blocks.push(Block::Image {
            media_type: media_type.clone(),
            data: data.clone(),
          });
        }
        other => text.push(other.clone()),
      }
    }
  }
  if !text.is_empty() {
    blocks.push(Block::Text(text_of(&text)));
  }
  if blocks.is_empty() {
    blocks.push(Block::Text("(an empty message)".into()));
  }
  blocks
}

#[cfg(test)]
mod tests {
  use super::*;

  fn user(t: &str) -> Message {
    Message::user(vec![Content::Text(t.into())])
  }
  fn assistant(content: Vec<Content>) -> Message {
    Message {
      role: Role::Assistant,
      content,
      native: None,
    }
  }
  fn call(id: &str) -> Content {
    Content::ToolCall {
      id: id.into(),
      name: "shell_run".into(),
      input: json!({"command": "ls"}),
    }
  }
  fn result(id: &str, out: &str) -> Content {
    Content::ToolResult {
      id: id.into(),
      output: out.into(),
      is_error: false,
    }
  }

  #[test]
  fn a_new_user_message_is_prompted_in_the_same_session() {
    let mut m = vec![user("hi"), assistant(vec![Content::Text("hello".into())])];
    let synced = fingerprints(&m);
    m.push(user("next"));
    assert_eq!(
      plan(Some(&synced), None, &m),
      Plan::Prompt(vec![Block::Text("next".into())])
    );
  }

  #[test]
  fn the_waiting_call_gets_its_result_and_the_steering_after_it() {
    let mut m = vec![
      user("list"),
      assistant(vec![Content::Text("Looking.".into()), call("c1")]),
    ];
    let synced = fingerprints(&m);
    m.push(Message::user(vec![result("c1", "a\nb")]));
    m.push(user("also check c"));
    match plan(Some(&synced), Some("c1"), &m) {
      Plan::Answer { output, is_error } => {
        assert!(!is_error);
        assert!(output.starts_with("a\nb\n\n[Not part of the tool's output"));
        assert!(output.ends_with("also check c"));
      }
      other => panic!("{other:?}"),
    }
    // Without steering, the result alone.
    let m2 = &m[..3];
    assert_eq!(
      plan(Some(&synced), Some("c1"), m2),
      Plan::Answer {
        output: "a\nb".into(),
        is_error: false
      }
    );
    // Another call's result: a fresh session.
    assert!(matches!(
      plan(Some(&synced), Some("c2"), m2),
      Plan::Fresh { .. }
    ));
  }

  #[test]
  fn a_rewind_or_a_compaction_starts_fresh_with_the_transcript() {
    let m = vec![
      user("hi"),
      assistant(vec![Content::Text("hello".into()), call("c1")]),
      Message::user(vec![result("c1", "files")]),
      assistant(vec![Content::Text("done".into())]),
    ];
    let synced = fingerprints(&m);
    // Rewound: the last reply gone, another message instead.
    let mut r = m[..3].to_vec();
    r.push(assistant(vec![Content::Text("other".into())]));
    r.push(user("again"));
    match plan(Some(&synced), None, &r) {
      Plan::Fresh { blocks, .. } => {
        let Block::Text(t) = &blocks[0] else { panic!() };
        assert!(t.contains("User:\nhi"));
        assert!(t.contains("Assistant called the tool `shell_run` with {\"command\":\"ls\"}"));
        assert!(t.contains("Result of `shell_run`:\nfiles"));
        assert!(t.contains("Assistant:\nother"));
        assert_eq!(blocks[1], Block::Text("again".into()));
      }
      other => panic!("{other:?}"),
    }
    // Compacted: a summary first.
    let c = vec![user("[summary] …"), user("go on")];
    assert!(matches!(plan(Some(&synced), None, &c), Plan::Fresh { .. }));
  }

  #[test]
  fn texts_of_user_messages_do_not_count_but_the_models_do() {
    // A user message rebuilt with a time note is still the same message.
    let a = vec![user("hi"), assistant(vec![Content::Text("hello".into())])];
    let b = vec![
      user("[2 hours later]\nhi"),
      assistant(vec![Content::Text("hello\n".into())]),
    ];
    assert_eq!(fingerprints(&a), fingerprints(&b));
    let c = vec![user("hi"), assistant(vec![Content::Text("bye".into())])];
    assert_ne!(fingerprints(&a), fingerprints(&c));
  }

  #[test]
  fn long_results_are_cut_and_images_named() {
    let long = "x".repeat(RESULT_MAX + 10);
    let t = transcript(&[
      assistant(vec![call("c")]),
      Message::user(vec![
        result("c", &long),
        Content::Image {
          media_type: "image/png".into(),
          data: vec![1],
        },
      ]),
    ]);
    assert!(t.contains("[… 10 more bytes cut]"));
    assert!(t.contains("[an image (image/png)]"));
    // A new message's images go as images.
    let b = blocks_of(&[Message::user(vec![
      Content::Text("see".into()),
      Content::Image {
        media_type: "image/png".into(),
        data: vec![1, 2],
      },
    ])]);
    assert_eq!(b.len(), 2);
    assert_eq!(b[1].to_acp()["type"], "image");
  }
}
