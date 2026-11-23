//! Preset answers (design §2.30, "Inline controls", step 1): the
//! `suggest_replies` tool. The agent offers the user a few answers to its
//! reply; clients show them as buttons under the turn's end, and pressing
//! one sends its text as an ordinary user message.
//!
//! Nothing is stored apart: the tool call is in the transcript, and the
//! transcript builders (`chat::transcript_shown`, the web client's
//! `apps::threads::transcript`) give its replies, validated as here, as a
//! field of its entry (`replies`). Whether they are live is the clients'
//! to say from the transcript (`live`): the entry with replies is the
//! latest one, no user message after it, and no turn runs.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stitch_providers::ToolDef;

use crate::tools::ToolOutcome;

/// The tool's name.
pub const TOOL: &str = "suggest_replies";
/// At most this many replies.
pub const MAX: usize = 6;
/// A label's length at most, in characters.
pub const LABEL_MAX: usize = 60;
/// A reply's text at most, in characters.
pub const TEXT_MAX: usize = 2000;

/// One preset answer: the button's `label`, and the `text` it sends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggested {
  pub label: String,
  pub text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
  replies: Vec<InputReply>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputReply {
  label: String,
  #[serde(default)]
  text: Option<String>,
}

/// The replies a call offers, or why they are refused: 1 to `MAX`, each
/// label short, single-line and distinct, `text` defaulting to `label`.
pub fn validate(input: &Value) -> Result<Vec<Suggested>, String> {
  let i: Input = serde_json::from_value(input.clone())
    .map_err(|e| format!("{TOOL} takes {{replies: [{{label, text?}}]}}: {e}"))?;
  if i.replies.is_empty() || i.replies.len() > MAX {
    return Err(format!(
      "give 1 to {MAX} replies (got {}): only the few obvious next answers",
      i.replies.len()
    ));
  }
  let mut out: Vec<Suggested> = Vec::with_capacity(i.replies.len());
  for r in i.replies {
    let label = r.label.trim().to_owned();
    if label.is_empty() {
      return Err("a reply's label is empty".into());
    }
    if label.contains('\n') {
      return Err(format!(
        "label {label:?} spans lines: keep it to a few words"
      ));
    }
    let n = label.chars().count();
    if n > LABEL_MAX {
      return Err(format!(
        "label {label:?} is {n} characters: at most {LABEL_MAX} (a few words; put the longer \
         message in `text`)"
      ));
    }
    if out.iter().any(|o| o.label == label) {
      return Err(format!("label {label:?} is given twice"));
    }
    let text = match r.text.map(|t| t.trim().to_owned()) {
      Some(t) if !t.is_empty() => t,
      _ => label.clone(),
    };
    if text.chars().count() > TEXT_MAX {
      return Err(format!(
        "the text of {label:?} is longer than {TEXT_MAX} characters"
      ));
    }
    out.push(Suggested { label, text });
  }
  Ok(out)
}

/// A transcript's tool call: its replies, when it is a valid
/// `suggest_replies` (an invalid one was refused, and shows none).
pub fn of_call(name: &str, input: &Value) -> Option<Vec<Suggested>> {
  (name == TOOL).then(|| validate(input).ok()).flatten()
}

/// The tool: shown at once, nothing to wait for.
pub(crate) fn call(input: &Value) -> ToolOutcome {
  match validate(input) {
    Ok(r) => ToolOutcome::ok(format!(
      "Shown: {} under your reply, live until the user writes. Write your reply now if you \
       have not (it asks in words), then end the turn; do not call this again this turn.",
      r.iter()
        .map(|r| format!("[{}]", r.label))
        .collect::<Vec<_>>()
        .join(" ")
    )),
    Err(e) => ToolOutcome::err(e),
  }
}

/// The tool call as one line in a transcript: its labels.
pub(crate) fn summary(input: &Value) -> String {
  match validate(input) {
    Ok(r) => format!(
      "{TOOL}: {}",
      r.iter()
        .map(|r| r.label.as_str())
        .collect::<Vec<_>>()
        .join(" · ")
    ),
    Err(_) => format!("{TOOL} {input}"),
  }
}

/// Which entry's replies are live, by index: `entries` in order, each
/// `(is_user_message, has_replies)`. The last entry with replies, when no
/// user message comes after it and no turn runs (`running`); older ones
/// are inert. Clients in other languages (the web client, the app) apply
/// the same rule.
pub fn live(entries: impl IntoIterator<Item = (bool, bool)>, running: bool) -> Option<usize> {
  if running {
    return None;
  }
  let mut at = None;
  for (n, (user, replies)) in entries.into_iter().enumerate() {
    if user {
      at = None;
    }
    if replies {
      at = Some(n);
    }
  }
  at
}

pub(crate) fn definition() -> ToolDef {
  ToolDef {
    name: TOOL.into(),
    description: format!(
      "Offer the user preset answers: buttons under your reply (numbered in the terminal); \
       pressing one sends its text as their message. Use it rarely: only when their next \
       answer is very likely one of a few obvious choices you just put to them (A or B, yes \
       or no to an action you propose). Not as a routine follow-up, not after a report or an \
       answer that asks for no decision, not for open questions; at most once a turn. 1 to \
       {MAX} replies; a label is a few words (at most {LABEL_MAX} characters), `text` what is \
       sent (default: the label), worded as the user would say it. Your reply still asks in \
       words: write it, call this, then end the turn."
    ),
    input_schema: json!({
      "type": "object", "additionalProperties": false,
      "properties": {
        "replies": {
          "type": "array", "minItems": 1, "maxItems": MAX,
          "items": {
            "type": "object", "additionalProperties": false,
            "properties": {
              "label": {"type": "string", "description": "The button: a few words."},
              "text": {"type": "string", "description": "What pressing it sends (default: the label)."}
            },
            "required": ["label"]
          }
        }
      },
      "required": ["replies"]
    }),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn text_defaults_to_the_label() {
    let r = validate(&json!({"replies": [
      {"label": " Yes "},
      {"label": "No", "text": "No, stop here."},
      {"label": "Later", "text": "  "}
    ]}))
    .unwrap();
    assert_eq!(
      r,
      vec![
        Suggested {
          label: "Yes".into(),
          text: "Yes".into()
        },
        Suggested {
          label: "No".into(),
          text: "No, stop here.".into()
        },
        Suggested {
          label: "Later".into(),
          text: "Later".into()
        },
      ]
    );
  }

  #[test]
  fn what_is_refused_says_why() {
    let bad = |v: Value| validate(&v).unwrap_err();
    assert!(bad(json!({"replies": []})).contains("1 to 6"));
    let seven: Vec<Value> = (0..7).map(|i| json!({"label": i.to_string()})).collect();
    assert!(bad(json!({ "replies": seven })).contains("got 7"));
    assert!(bad(json!({"replies": [{"label": "x".repeat(61)}]})).contains("at most 60"));
    assert!(validate(&json!({"replies": [{"label": "é".repeat(60)}]})).is_ok());
    assert!(bad(json!({"replies": [{"label": "  "}]})).contains("empty"));
    assert!(bad(json!({"replies": [{"label": "a\nb"}]})).contains("spans lines"));
    assert!(bad(json!({"replies": [{"label": "A"}, {"label": "A "}]})).contains("twice"));
    assert!(bad(json!({"replies": [{"label": "A", "txt": "x"}]})).contains("takes"));
    assert!(bad(json!({"replies": [{"label": "A", "text": "x".repeat(2001)}]})).contains("2000"));
  }

  #[test]
  fn the_call_says_it_is_shown() {
    let ok = call(&json!({"replies": [{"label": "A"}, {"label": "B"}]}));
    assert!(
      ok.ok && ok.output.starts_with("Shown: [A] [B]"),
      "{}",
      ok.output
    );
    let err = call(&json!({"replies": []}));
    assert!(!err.ok);
    assert_eq!(
      summary(&json!({"replies": [{"label": "A"}, {"label": "B", "text": "b!"}]})),
      "suggest_replies: A · B"
    );
    assert!(of_call("notify", &json!({"replies": [{"label": "A"}]})).is_none());
    assert!(of_call(TOOL, &json!({"replies": []})).is_none());
    assert_eq!(
      of_call(TOOL, &json!({"replies": [{"label": "A"}]}))
        .unwrap()
        .len(),
      1
    );
  }

  #[test]
  fn only_the_latest_unanswered_is_live() {
    // (user message, has replies)
    let e = [(true, false), (false, false), (false, true), (false, false)];
    assert_eq!(live(e, false), Some(2));
    assert_eq!(live(e, true), None);
    // Answered (or anything written since): inert.
    let e = [(false, true), (true, false), (false, false)];
    assert_eq!(live(e, false), None);
    // Two in a turn: the later.
    let e = [(false, true), (false, true)];
    assert_eq!(live(e, false), Some(1));
    // An older one, a newer turn without: inert.
    let e = [(false, true), (true, false), (false, false)];
    assert_eq!(live(e, false), None);
    assert_eq!(live([], false), None);
  }
}
