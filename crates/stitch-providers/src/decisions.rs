//! Closed-set judgements behind one shape (design §2.28, Decisions): an
//! input and named questions, each answered with a calibrated
//! probability, in one call, without generating text. Gemini's Decisions
//! API (`gemini-decision-1`) is the first backend; another (a chat model
//! forced to a JSON schema) is one more arm in [`Provider::decide`] and
//! [`Provider::decides`].
//!
//! Yes/no questions are what the node asks today (the hazard guard);
//! choices and scores are in the shape so a later use needs no new call.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};

use crate::{Error, Provider, Result, Usage};

/// A question asked of the input.
#[derive(Clone, Debug, PartialEq)]
pub enum Question {
  /// Yes or no: answered with the probability of yes.
  YesNo { instructions: String },
  /// One of named options, each with when it applies (may be empty).
  Choice {
    instructions: String,
    options: BTreeMap<String, String>,
  },
  /// A level on an ordered rubric, lowest first.
  Score {
    instructions: String,
    levels: Vec<String>,
  },
}

/// What to decide: one shared input (text), many questions by key.
#[derive(Clone, Debug, PartialEq)]
pub struct DecisionRequest {
  /// The provider's model, without the provider's name.
  pub model: String,
  pub input: String,
  pub questions: BTreeMap<String, Question>,
  /// The whole call's limit (decisions gate other work: short).
  pub timeout: Duration,
}

/// One question's answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
  /// The probability of yes (0.01 to 0.99 for Gemini's).
  YesNo { p: f64, confidence: f64 },
  Choice {
    choice: String,
    probabilities: BTreeMap<String, f64>,
    confidence: f64,
  },
  /// The probability-weighted level (it may fall between levels).
  Score {
    score: f64,
    probabilities: BTreeMap<String, f64>,
    confidence: f64,
  },
}

impl Answer {
  /// A yes/no answer's probability of yes.
  pub fn yes(&self) -> Option<f64> {
    match self {
      Answer::YesNo { p, .. } => Some(*p),
      _ => None,
    }
  }

  pub fn confidence(&self) -> f64 {
    match self {
      Answer::YesNo { confidence, .. }
      | Answer::Choice { confidence, .. }
      | Answer::Score { confidence, .. } => *confidence,
    }
  }
}

/// What came back.
#[derive(Clone, Debug, PartialEq)]
pub struct Decided {
  pub model: String,
  pub answers: BTreeMap<String, Answer>,
  /// Input tokens only: a decision generates none.
  pub usage: Usage,
}

impl Provider {
  /// Whether `model` of this provider answers decisions.
  pub fn decides(&self, model: &str) -> bool {
    match self {
      Provider::Gemini(_) => crate::gemini::decides(model),
      _ => false,
    }
  }

  /// Answers `r`'s questions over its input, where this kind of provider
  /// can.
  pub async fn decide(&self, r: &DecisionRequest) -> Result<Decided> {
    match &*self.current().await? {
      Provider::Gemini(p) => p.decide(r).await,
      other => Err(Error::Protocol {
        provider: other.kind(),
        message: "this kind of provider answers no decisions".into(),
      }),
    }
  }
}

/// The request's body as Gemini's Decisions API takes it.
pub(crate) fn body(r: &DecisionRequest) -> Value {
  let questions: serde_json::Map<String, Value> = r
    .questions
    .iter()
    .map(|(k, q)| {
      let v = match q {
        Question::YesNo { instructions } => json!({"type": "noul", "instructions": instructions}),
        Question::Choice {
          instructions,
          options,
        } => json!({"type": "choice", "instructions": instructions, "criteria": options}),
        Question::Score {
          instructions,
          levels,
        } => json!({"type": "score", "instructions": instructions, "criteria": levels}),
      };
      (k.clone(), v)
    })
    .collect();
  json!({"model": r.model, "input": r.input, "questions": questions})
}

/// Its answers from the API's reply; a question it did not answer is an
/// error, so a caller never takes a missing answer for "no".
pub(crate) fn parse(r: &DecisionRequest, reply: &Value, provider: &'static str) -> Result<Decided> {
  let f = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64);
  let probs = |v: &Value| -> BTreeMap<String, f64> {
    v.get("probabilities")
      .and_then(Value::as_object)
      .map(|m| {
        m.iter()
          .filter_map(|(k, p)| p.as_f64().map(|p| (k.clone(), p)))
          .collect()
      })
      .unwrap_or_default()
  };
  let mut answers = BTreeMap::new();
  for (key, q) in &r.questions {
    let a = reply
      .get("answers")
      .and_then(|a| a.get(key))
      .ok_or_else(|| Error::protocol(provider, format!("no answer to `{key}`")))?;
    let missing = || Error::protocol(provider, format!("the answer to `{key}` has no value"));
    let answer = match q {
      Question::YesNo { .. } => {
        let p = f(a, "noul").ok_or_else(missing)?;
        Answer::YesNo {
          p,
          confidence: f(a, "confidence").unwrap_or(p.max(1.0 - p)),
        }
      }
      Question::Choice { .. } => Answer::Choice {
        choice: a
          .get("choice")
          .and_then(Value::as_str)
          .ok_or_else(missing)?
          .to_owned(),
        probabilities: probs(a),
        confidence: f(a, "confidence").unwrap_or(0.0),
      },
      Question::Score { .. } => Answer::Score {
        score: f(a, "score").ok_or_else(missing)?,
        probabilities: probs(a),
        confidence: f(a, "confidence").unwrap_or(0.0),
      },
    };
    answers.insert(key.clone(), answer);
  }
  let input_tokens = reply
    .get("usage")
    .and_then(|u| u.get("total_input_tokens"))
    .and_then(Value::as_u64)
    .unwrap_or(0);
  Ok(Decided {
    model: r.model.clone(),
    answers,
    usage: Usage {
      input_tokens,
      ..Usage::default()
    },
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn request() -> DecisionRequest {
    let mut questions = BTreeMap::new();
    questions.insert(
      "bad".to_owned(),
      Question::YesNo {
        instructions: "Is it bad?".into(),
      },
    );
    questions.insert(
      "dept".to_owned(),
      Question::Choice {
        instructions: "Which?".into(),
        options: [("a".to_owned(), String::new())].into(),
      },
    );
    DecisionRequest {
      model: "gemini-decision-1".into(),
      input: "x".into(),
      questions,
      timeout: Duration::from_secs(3),
    }
  }

  #[test]
  fn the_body_names_each_question_by_its_type() {
    let b = body(&request());
    assert_eq!(b["questions"]["bad"]["type"], "noul");
    assert_eq!(b["questions"]["dept"]["type"], "choice");
    assert_eq!(b["questions"]["dept"]["criteria"]["a"], "");
    assert_eq!(b["model"], "gemini-decision-1");
  }

  #[test]
  fn answers_and_usage_are_read_and_a_missing_one_is_an_error() {
    let r = request();
    let reply = json!({
      "answers": {
        "bad": {"type": "noul", "noul": 0.93, "confidence": 0.93},
        "dept": {"type": "choice", "choice": "a", "probabilities": {"a": 1.0}, "confidence": 1.0}
      },
      "usage": {"total_input_tokens": 321, "total_output_tokens": 0}
    });
    let d = parse(&r, &reply, "gemini").unwrap();
    assert_eq!(d.answers["bad"].yes(), Some(0.93));
    assert_eq!(d.usage.input_tokens, 321);
    assert_eq!(d.usage.output_tokens, 0);
    let partial = json!({"answers": {"bad": {"noul": 0.2}}});
    assert!(parse(&r, &partial, "gemini").is_err());
  }
}
