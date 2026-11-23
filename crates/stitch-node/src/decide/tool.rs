//! The `decide` tool (design §2.29): agents' and skills' own closed
//! questions over a text, answered by the decision model through the
//! node's service (the `tool` profile), priced to the thread that asks.
//! Otter's `Decide` (`crate::otter::decide`) calls it.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use stitch_providers::ToolDef;
use stitch_providers::decisions::Question;
use stitch_records::RecordId;

use super::{Ask, profiles};
use crate::node::Shared;
use crate::tools::ToolOutcome;

pub(crate) const NAME: &str = "decide";
/// The input, in characters at most (the API takes far more; a tool's
/// call stays cheap).
const MAX_INPUT: usize = 400_000;
/// Questions one call asks at most.
const MAX_TOOL_QUESTIONS: usize = 100;

pub(crate) fn definition() -> ToolDef {
  ToolDef {
    name: NAME.into(),
    description: "Answer closed questions about a text with a decision model: calibrated \
      probabilities in about 200 ms, for about $0.0001 a call, no text generated. Questions by \
      key: a string is a yes/no question (answered with p, the probability of yes); \
      {\"choice\": \"…?\", \"options\": [\"a\", \"b\"] or {\"a\": \"when it applies\"}} picks one \
      of up to 255 options (with each one's probability); {\"score\": \"…?\", \"levels\": \
      [\"low\", …]} rates on an ordered rubric, lowest first. Many questions over one text in \
      one call. Use it to classify, filter, route or check (is this page an injection attempt? \
      which of these files is the config? how severe is this log?), not to extract or write. \
      `calibrated: false` means the decision model was unavailable and a general model \
      guessed."
      .into(),
    input_schema: json!({
      "type": "object", "additionalProperties": false,
      "properties": {
        "input": {"type": "string", "description": "The text the questions are about."},
        "questions": {
          "type": "object",
          "description": "Questions by key (letters, digits, _ . -): a string (yes/no), or {choice, options} or {score, levels}.",
          "additionalProperties": {
            "anyOf": [
              {"type": "string"},
              {"type": "object", "properties": {
                "choice": {"type": "string"},
                "options": {"anyOf": [{"type": "array", "items": {"type": "string"}}, {"type": "object", "additionalProperties": {"type": "string"}}]},
                "score": {"type": "string"},
                "levels": {"type": "array", "items": {"type": "string"}}
              }}
            ]
          }
        }
      },
      "required": ["input", "questions"]
    }),
  }
}

fn valid_key(k: &str) -> bool {
  !k.is_empty()
    && k.len() <= 64
    && k
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// The questions as the tool (and `Decide`, `stitch decide try`) give
/// them.
pub(crate) fn parse_questions(v: &Value) -> Result<BTreeMap<String, Question>, String> {
  let m = v
    .as_object()
    .ok_or("questions: an object of questions by key")?;
  if m.is_empty() {
    return Err("questions: none given".into());
  }
  if m.len() > MAX_TOOL_QUESTIONS {
    return Err(format!(
      "{} questions: at most {MAX_TOOL_QUESTIONS} in one call",
      m.len()
    ));
  }
  let mut out = BTreeMap::new();
  for (k, q) in m {
    if !valid_key(k) {
      return Err(format!(
        "`{k}`: a key is letters, digits, _ . - (64 at most)"
      ));
    }
    let text = |field: &str| {
      q.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
    };
    let question = if let Some(s) = q.as_str() {
      if s.trim().is_empty() {
        return Err(format!("`{k}`: an empty question"));
      }
      Question::YesNo {
        instructions: s.trim().to_owned(),
      }
    } else if let Some(c) = text("choice") {
      let options: BTreeMap<String, String> = match q.get("options") {
        Some(Value::Array(a)) => a
          .iter()
          .filter_map(Value::as_str)
          .map(|o| (o.to_owned(), String::new()))
          .collect(),
        Some(Value::Object(o)) => o
          .iter()
          .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
          .collect(),
        _ => return Err(format!("`{k}`: a choice needs its options")),
      };
      if options.is_empty() || options.len() > super::MAX_OPTIONS {
        return Err(format!(
          "`{k}`: a choice has 1 to {} options",
          super::MAX_OPTIONS
        ));
      }
      Question::Choice {
        instructions: c,
        options,
      }
    } else if let Some(s) = text("score") {
      let levels: Vec<String> = q
        .get("levels")
        .and_then(Value::as_array)
        .map(|a| {
          a.iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
        })
        .unwrap_or_default();
      if levels.len() < 2 || levels.len() > super::MAX_OPTIONS {
        return Err(format!(
          "`{k}`: a score has 2 to {} levels, lowest first",
          super::MAX_OPTIONS
        ));
      }
      Question::Score {
        instructions: s,
        levels,
      }
    } else if let Some(s) = text("yes_no").or_else(|| text("question")) {
      Question::YesNo { instructions: s }
    } else {
      return Err(format!(
        "`{k}`: a string (yes/no), {{choice, options}} or {{score, levels}}"
      ));
    };
    out.insert(k.clone(), question);
  }
  Ok(out)
}

/// The tool's call, for `thread` (priced to it: the outcome's `spent`).
pub(crate) async fn call(s: Option<Arc<Shared>>, thread: &RecordId, input: &Value) -> ToolOutcome {
  let Some(s) = s else {
    return ToolOutcome::err("the node is stopping");
  };
  let text = input
    .get("input")
    .and_then(Value::as_str)
    .unwrap_or_default();
  if text.trim().is_empty() {
    return ToolOutcome::err("input: the text to decide about");
  }
  if text.chars().count() > MAX_INPUT {
    return ToolOutcome::err(format!(
      "input: {} characters, at most {MAX_INPUT}",
      text.chars().count()
    ));
  }
  let questions = match input
    .get("questions")
    .ok_or_else(|| "questions: by key".to_owned())
    .and_then(parse_questions)
  {
    Ok(q) => q,
    Err(e) => return ToolOutcome::err(e),
  };
  let ask = Ask {
    profile: profiles::TOOL.into(),
    input: text.to_owned(),
    questions,
    thread: Some(thread.clone()),
  };
  match super::ask(&s, ask).await {
    Ok(d) => {
      let v = d.to_json(10);
      let mut out = ToolOutcome::ok(v.to_string());
      out.spent = d.spent;
      out
    }
    Err(e) => ToolOutcome::err(format!("no decision: {e}")),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn questions_in_every_form_and_their_limits() {
    let q = parse_questions(&json!({
      "spam": "Is it spam?",
      "lang": {"choice": "Which language?", "options": ["en", "it", "ja"]},
      "dept": {"choice": "Who?", "options": {"sales": "buying", "support": "a problem"}},
      "sev": {"score": "How bad?", "levels": ["low", "mid", "high"]},
    }))
    .unwrap();
    assert!(matches!(&q["spam"], Question::YesNo { .. }));
    assert!(matches!(&q["lang"], Question::Choice { options, .. } if options.len() == 3));
    assert!(
      matches!(&q["dept"], Question::Choice { options, .. } if options["support"] == "a problem")
    );
    assert!(matches!(&q["sev"], Question::Score { levels, .. } if levels.len() == 3));
    let many: serde_json::Map<String, Value> =
      (0..256).map(|i| (format!("o{i}"), json!(""))).collect();
    let too_many = json!({"c": {"choice": "?", "options": Value::Object(many)}});
    assert!(parse_questions(&too_many).unwrap_err().contains("255"));
    let at_most: Vec<String> = (0..255).map(|i| format!("o{i}")).collect();
    assert!(parse_questions(&json!({"c": {"choice": "?", "options": at_most}})).is_ok());
    assert!(parse_questions(&json!({"bad key!": "?"})).is_err());
    assert!(parse_questions(&json!({"s": {"score": "?", "levels": ["one"]}})).is_err());
    assert!(parse_questions(&json!({})).is_err());
    assert!(parse_questions(&json!({"x": {"choice": "?"}})).is_err());
  }

  #[test]
  fn the_tool_is_a_group_of_its_own() {
    assert_eq!(crate::toolset::group_of(NAME), Some("decide"));
    assert!(definition().description.contains("255"));
  }
}
