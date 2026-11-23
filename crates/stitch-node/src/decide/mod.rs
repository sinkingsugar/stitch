//! Decisions as a node service (design §2.29): one entry point,
//! [`ask`], for every closed-set judgement the harness makes. A use names a
//! profile (`profiles`: its questions, thresholds and fallback policy) and
//! gives a text input; the decision model (`Provider::decide`, Gemini's
//! `gemini-decision-1`) answers each question with a calibrated
//! probability in one short call.
//!
//! - **Typed answers:** yes/no (the probability of yes), a choice (the
//!   option, each option's probability), a score (the probability-weighted
//!   level), each with a confidence; [`Decision::passes`] reads one against
//!   its threshold.
//! - **Metered and priced:** what a call spent comes back with the
//!   decision, for the calling thread to count as a tool's model use; a
//!   call no thread made counts in the node's own day (`decide-day`, as
//!   digests keep theirs).
//! - **Recorded** for calibration: each call appends a line to
//!   `<data>/decide/<YYYY-MM>.jsonl` on the node (profile, thread, the
//!   input's hash, length and first words with secrets' values taken out,
//!   the answers, model, milliseconds, cost, cached or not, calibrated or
//!   not, an error): local, never synced, never the input whole.
//! - **Cached:** an identical call (profile, model, questions, input)
//!   within ten minutes is answered again without a call.
//! - **Fallback:** none by default: when the decision model cannot answer
//!   (no key, a timeout, an error), the call fails. The user may set a
//!   fallback model (`stitch decide set --fallback <model>`, for every
//!   profile or one), asked forced to a JSON schema (`fallback`), its
//!   answers marked uncalibrated; never for what gates authority
//!   (`guard.fixer`, `land`, which ask the user instead, §2.28) nor loose
//!   ends (their own quick-model path).

pub(crate) mod fallback;
pub(crate) mod profiles;
pub(crate) mod tool;
pub(crate) mod triage;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use serde_json::{Value, json};
use stitch_providers::decisions::{Answer, Question};
use stitch_records::{Author, Draft, RecordId};

use crate::node::Shared;
use crate::peer::Caller;
use crate::tools::{BoxFuture, Spent};
use crate::{Error, Result};

pub(crate) use profiles::Profile;

/// The settings' records (`stitch decide set`).
pub(crate) const KIND: &str = "decide-settings";
/// The decision model when none is set.
pub(crate) const DEFAULT_MODEL: &str = crate::hazard::DEFAULT_MODEL;
/// Where the node keeps its own day's decisions' cost (local).
const DAY_KEY: &str = "decide-day";
/// Identical calls answered from the cache within this.
const CACHE_TTL: Duration = Duration::from_secs(600);
const CACHE_SIZE: usize = 256;
/// The most questions one call asks (the API's limit).
pub(crate) const MAX_QUESTIONS: usize = 1_000;
/// The most options a choice has.
pub(crate) const MAX_OPTIONS: usize = 255;
/// What the record keeps of the input, in characters at most.
const SUMMARY: usize = 160;

/// One call.
#[derive(Clone, Debug)]
pub(crate) struct Ask {
  /// The profile's name (`profiles::named`).
  pub profile: String,
  pub input: String,
  /// The questions, by key: the profile's (`Profile::questions`), with
  /// what the use adds, or ad-hoc ones (the `tool` profile).
  pub questions: BTreeMap<String, Question>,
  /// The thread that pays, when one asks; else the node's own day.
  pub thread: Option<RecordId>,
}

impl Ask {
  pub(crate) fn new(profile: &str, input: String) -> Ask {
    Ask {
      profile: profile.to_owned(),
      input,
      questions: profiles::named(profile)
        .map(Profile::questions)
        .unwrap_or_default(),
      thread: None,
    }
  }
}

/// A profile as it stands: its defaults under the user's settings.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Resolved {
  pub profile: String,
  pub thresholds: BTreeMap<String, f64>,
  /// The fallback model (`provider/model`) when the decision model cannot
  /// answer; none: the call fails.
  pub fallback: Option<String>,
  /// `provider/model`.
  pub model: String,
  /// The node's own decisions' cap a day, in dollars.
  pub daily_usd: f64,
  pub timeout: Duration,
  pub cache: bool,
  /// Where the thresholds came from.
  pub from: &'static str,
}

/// What came back.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Decision {
  pub profile: String,
  pub answers: BTreeMap<String, Answer>,
  pub thresholds: BTreeMap<String, f64>,
  /// The decision model's (calibrated); `false` for the fallback's.
  pub calibrated: bool,
  /// Who answered, `provider/model`.
  pub model: String,
  pub ms: u64,
  /// From the cache: nothing spent.
  pub cached: bool,
  /// Why the decision model did not answer, when the fallback did.
  pub fell_back: Option<String>,
  pub spent: Option<Spent>,
}

impl Decision {
  /// A yes/no answer's probability of yes.
  pub(crate) fn yes(&self, key: &str) -> Option<f64> {
    self.answers.get(key).and_then(Answer::yes)
  }

  pub(crate) fn threshold(&self, key: &str) -> f64 {
    profiles::threshold_of(&self.thresholds, key)
  }

  /// A choice's option and its probability.
  pub(crate) fn choice(&self, key: &str) -> Option<(&str, f64)> {
    match self.answers.get(key)? {
      Answer::Choice {
        choice,
        probabilities,
        confidence,
      } => Some((
        choice.as_str(),
        probabilities.get(choice).copied().unwrap_or(*confidence),
      )),
      _ => None,
    }
  }

  /// Whether an answer reaches its threshold: yes/no, its probability of
  /// yes; a choice, its option's probability; a score, its confidence.
  pub(crate) fn passes(&self, key: &str) -> bool {
    let t = self.threshold(key);
    match self.answers.get(key) {
      Some(Answer::YesNo { p, .. }) => *p >= t,
      // A choice's option may have its own (`verdict:done`).
      Some(Answer::Choice { .. }) => self.choice(key).is_some_and(|(c, p)| {
        p >= self
          .thresholds
          .get(&format!("{key}:{c}"))
          .copied()
          .unwrap_or(t)
      }),
      Some(Answer::Score { confidence, .. }) => *confidence >= t,
      None => false,
    }
  }

  /// The answers as JSON (the tool's, the records').
  pub(crate) fn answers_json(&self, top: usize) -> Value {
    let mut out = serde_json::Map::new();
    for (k, a) in &self.answers {
      out.insert(k.clone(), answer_json(a, top));
    }
    Value::Object(out)
  }

  /// As the tool gives it: each choice's or score's `top` likeliest.
  pub(crate) fn to_json(&self, top: usize) -> Value {
    let mut v = json!({
      "profile": self.profile,
      "answers": self.answers_json(top),
      "calibrated": self.calibrated,
      "model": self.model,
      "ms": self.ms,
    });
    if self.cached {
      v["cached"] = json!(true);
    }
    if let Some(e) = &self.fell_back {
      v["fell_back"] = json!(e);
    }
    if let Some(u) = self.spent.as_ref().and_then(|s| s.usd) {
      v["usd"] = json!(u);
    }
    v
  }
}

fn round(p: f64) -> f64 {
  (p * 1000.0).round() / 1000.0
}

/// An answer as JSON: the probabilities of a choice or score, the `top`
/// likeliest.
pub(crate) fn answer_json(a: &Answer, top: usize) -> Value {
  let probs = |m: &BTreeMap<String, f64>| {
    let mut v: Vec<(&String, &f64)> = m.iter().collect();
    v.sort_by(|a, b| b.1.total_cmp(a.1));
    let o: serde_json::Map<String, Value> = v
      .into_iter()
      .take(top)
      .map(|(k, p)| (k.clone(), json!(round(*p))))
      .collect();
    Value::Object(o)
  };
  match a {
    Answer::YesNo { p, confidence } => {
      json!({"type": "yes_no", "p": round(*p), "confidence": round(*confidence)})
    }
    Answer::Choice {
      choice,
      probabilities,
      confidence,
    } => json!({
      "type": "choice", "choice": choice,
      "p": round(probabilities.get(choice).copied().unwrap_or(*confidence)),
      "probabilities": probs(probabilities), "confidence": round(*confidence),
    }),
    Answer::Score {
      score,
      probabilities,
      confidence,
    } => json!({
      "type": "score", "score": round(*score),
      "probabilities": probs(probabilities), "confidence": round(*confidence),
    }),
  }
}

/// Answers by key and what they cost, or why there are none (with what
/// the call cost all the same, when a model answered something unusable).
pub(crate) type Answered = std::result::Result<(BTreeMap<String, Answer>, Spent), Failed>;

/// A call that gave no answers, and what it spent anyway.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Failed {
  pub why: String,
  pub spent: Option<Spent>,
}

impl From<String> for Failed {
  fn from(why: String) -> Failed {
    Failed { why, spent: None }
  }
}

impl From<&str> for Failed {
  fn from(why: &str) -> Failed {
    why.to_owned().into()
  }
}

/// Who answers (a fake in tests).
pub(crate) trait Backend: Send + Sync {
  /// The decision model `model` (`provider/model`).
  fn decide<'a>(
    &'a self,
    model: &'a str,
    input: &'a str,
    questions: &'a BTreeMap<String, Question>,
    timeout: Duration,
  ) -> BoxFuture<'a, Answered>;
  /// The fallback model `model` forced to a JSON schema (uncalibrated).
  fn fallback<'a>(
    &'a self,
    model: &'a str,
    input: &'a str,
    questions: &'a BTreeMap<String, Question>,
    timeout: Duration,
  ) -> BoxFuture<'a, Answered>;
}

/// Where records and the node's spending go (the node's, a list in
/// tests).
pub(crate) trait Sink: Send + Sync {
  fn record(&self, line: &Value);
  /// A call no thread made: the node's own day.
  fn node_spent(&self, usd: f64);
  /// What the node's own decisions spent today.
  fn node_today(&self) -> f64;
  /// What `thread` has left of its budget (none: it has none).
  fn budget_left(&self, thread: &RecordId) -> Option<f64> {
    let _ = thread;
    None
  }
}

/// Paid calls at once, for the whole node.
const CONCURRENT: usize = 4;
/// Paid decisions one tool call makes at most (a script's `Decide`s).
pub(crate) const PER_CALL: usize = 500;
/// What one tool call's decisions spend at most, in dollars.
pub(crate) const PER_CALL_USD: f64 = 0.50;
/// What the node's own decisions (the triage's pre-pass, loose ends, the
/// landing gate) spend in a day at most, unless the user sets another
/// (`stitch decide set --daily <usd>`).
pub(crate) const NODE_DAILY_USD: f64 = 0.50;

/// The node's service: its cache, and the paid calls it lets run at once.
pub(crate) struct Service {
  cache: Mutex<Cache>,
  paid: tokio::sync::Semaphore,
  /// Appends to the records, one at a time.
  pub(crate) appending: Mutex<()>,
}

impl Default for Service {
  fn default() -> Service {
    Service {
      cache: Mutex::default(),
      paid: tokio::sync::Semaphore::new(CONCURRENT),
      appending: Mutex::default(),
    }
  }
}

/// What a call may cost at most, priced as `model`'s input and some
/// output (a fallback writes), before it is made.
fn estimate(model: &str, chars: usize) -> f64 {
  let usage = stitch_providers::Usage {
    input_tokens: (chars / 3 + 200) as u64,
    output_tokens: 1_000,
    ..Default::default()
  };
  crate::pricing::priced(model, &usage, None).unwrap_or(0.01)
}

/// `v` with every string (keys too) redacted.
fn redacted(v: &Value, redact: &(dyn Fn(&str) -> String + Send + Sync)) -> Value {
  match v {
    Value::String(s) => Value::String(redact(s)),
    Value::Array(a) => Value::Array(a.iter().map(|x| redacted(x, redact)).collect()),
    Value::Object(o) => Value::Object(
      o.iter()
        .map(|(k, x)| (redact(k), redacted(x, redact)))
        .collect(),
    ),
    other => other.clone(),
  }
}

/// The questions with secrets' values taken out of every text a model
/// reads (instructions, options' descriptions, levels); a key or an
/// option's name holding one is refused (it is what the answer names).
fn redact_questions(
  questions: &BTreeMap<String, Question>,
  redact: &(dyn Fn(&str) -> String + Send + Sync),
) -> std::result::Result<BTreeMap<String, Question>, String> {
  let mut out = BTreeMap::new();
  for (k, q) in questions {
    if redact(k) != *k {
      return Err("a question's key holds a secret's value".into());
    }
    let q = match q {
      Question::YesNo { instructions } => Question::YesNo {
        instructions: redact(instructions),
      },
      Question::Choice {
        instructions,
        options,
      } => {
        let mut o = BTreeMap::new();
        for (name, about) in options {
          if redact(name) != *name {
            return Err(format!("an option of `{k}` holds a secret's value"));
          }
          o.insert(name.clone(), redact(about));
        }
        Question::Choice {
          instructions: redact(instructions),
          options: o,
        }
      }
      Question::Score {
        instructions,
        levels,
      } => Question::Score {
        instructions: redact(instructions),
        levels: levels.iter().map(|l| redact(l)).collect(),
      },
    };
    out.insert(k.clone(), q);
  }
  Ok(out)
}

#[derive(Default)]
struct Cache {
  entries: HashMap<String, (Instant, Decision)>,
  order: VecDeque<String>,
}

impl Cache {
  fn get(&mut self, key: &str) -> Option<Decision> {
    match self.entries.get(key) {
      Some((at, d)) if at.elapsed() < CACHE_TTL => Some(d.clone()),
      Some(_) => {
        self.entries.remove(key);
        None
      }
      None => None,
    }
  }

  fn put(&mut self, key: String, d: Decision) {
    if self
      .entries
      .insert(key.clone(), (Instant::now(), d))
      .is_none()
    {
      self.order.push_back(key);
    }
    while self.order.len() > CACHE_SIZE {
      if let Some(old) = self.order.pop_front() {
        self.entries.remove(&old);
      }
    }
  }
}

/// What a call runs with.
pub(crate) struct Env<'a> {
  pub backend: &'a dyn Backend,
  pub sink: &'a dyn Sink,
  /// Takes secrets' values out of the input before it is sent or kept.
  pub redact: &'a (dyn Fn(&str) -> String + Send + Sync),
  pub resolved: Resolved,
}

/// Checks a call's questions: some, not too many, choices with options.
pub(crate) fn check(questions: &BTreeMap<String, Question>) -> std::result::Result<(), String> {
  if questions.is_empty() {
    return Err("no questions".into());
  }
  if questions.len() > MAX_QUESTIONS {
    return Err(format!(
      "{} questions: at most {MAX_QUESTIONS}",
      questions.len()
    ));
  }
  for (k, q) in questions {
    match q {
      Question::Choice { options, .. } if options.is_empty() || options.len() > MAX_OPTIONS => {
        return Err(format!("`{k}`: a choice has 1 to {MAX_OPTIONS} options"));
      }
      Question::Score { levels, .. } if levels.len() < 2 || levels.len() > MAX_OPTIONS => {
        return Err(format!("`{k}`: a score has 2 to {MAX_OPTIONS} levels"));
      }
      _ => {}
    }
  }
  Ok(())
}

impl Service {
  /// Whether a paid call may be made now: the node's own decisions within
  /// its day's cap; a thread's within what its tool call may still spend
  /// (`PER_CALL` decisions, `PER_CALL_USD`, what is left of the thread's
  /// budget as the call began).
  fn admit(&self, env: &Env<'_>, ask: &Ask, input: &str) -> std::result::Result<(), String> {
    let r = &env.resolved;
    let chars = input.len() + format!("{:?}", ask.questions).len();
    let est =
      estimate(&r.model, chars).max(r.fallback.as_deref().map_or(0.0, |m| estimate(m, chars)));
    match &ask.thread {
      None => {
        let today = env.sink.node_today();
        if today + est > r.daily_usd {
          return Err(format!(
            "the node's decisions spent ${today:.4} today, its cap ${:.2} (stitch decide set \
             --daily <usd>)",
            r.daily_usd
          ));
        }
      }
      Some(t) => {
        if let Some(acc) = crate::tools::also_spent_cell() {
          let mut a = acc.lock().expect("spent lock");
          a.decisions += 1;
          if a.decisions > PER_CALL {
            return Err(format!("{PER_CALL} decisions in one call at most"));
          }
          let so_far = a.usd();
          if so_far + est > PER_CALL_USD {
            return Err(format!(
              "this call's decisions spent ${so_far:.4}: ${PER_CALL_USD:.2} a call at most"
            ));
          }
          let left = *a.left.get_or_insert_with(|| env.sink.budget_left(t));
          if left.is_some_and(|l| so_far + est > l) {
            return Err("the thread's budget would be passed".into());
          }
        } else if env.sink.budget_left(t).is_some_and(|l| est > l) {
          return Err("the thread's budget would be passed".into());
        }
      }
    }
    Ok(())
  }

  /// Asks `ask`'s questions as `env` says: the cache, the decision model,
  /// the fallback model when one is set; kept and metered. The whole
  /// request is redacted before it is hashed, sent or kept; money is
  /// admitted before each paid call (`admit`).
  pub(crate) async fn ask(
    &self,
    env: &Env<'_>,
    mut ask: Ask,
  ) -> std::result::Result<Decision, String> {
    check(&ask.questions)?;
    let r = &env.resolved;
    let input = (env.redact)(&ask.input);
    ask.questions = redact_questions(&ask.questions, env.redact)?;
    let rec = |v: Value| env.sink.record(&redacted(&v, env.redact));
    let key = {
      let mut h = blake3::Hasher::new();
      for part in [
        ask.profile.as_str(),
        r.model.as_str(),
        &format!("{:?}", ask.questions),
        input.as_str(),
      ] {
        h.update(part.as_bytes());
        h.update(&[0]);
      }
      h.finalize().to_hex().to_string()
    };
    let input_hash = blake3::hash(input.as_bytes()).to_hex()[..16].to_owned();
    let base = |d: Option<&Decision>, error: Option<&str>| {
      let mut v = json!({
        "at": Timestamp::now().to_string(),
        "profile": ask.profile,
        "input_hash": input_hash,
        "input_chars": input.chars().count(),
        "summary": summary(&input),
        "questions": ask.questions.len(),
      });
      if let Some(t) = &ask.thread {
        v["thread"] = json!(t.to_string());
      }
      if let Some(d) = d {
        v["answers"] = d.answers_json(3);
        v["calibrated"] = json!(d.calibrated);
        v["model"] = json!(d.model);
        v["ms"] = json!(d.ms);
        if d.cached {
          v["cached"] = json!(true);
        }
        if let Some(e) = &d.fell_back {
          v["fell_back"] = json!(e);
        }
        if let Some(s) = &d.spent {
          v["usd"] = json!(s.usd);
          v["input_tokens"] = json!(s.usage.input_tokens);
          v["output_tokens"] = json!(s.usage.output_tokens);
        }
      } else {
        v["model"] = json!(r.model);
      }
      if let Some(e) = error {
        v["error"] = json!(e);
      }
      v
    };
    let cached = if r.cache {
      self.cache.lock().expect("cache lock").get(&key)
    } else {
      None
    };
    if let Some(mut d) = cached {
      d.cached = true;
      d.spent = None;
      d.ms = 0;
      d.thresholds = r.thresholds.clone();
      rec(base(Some(&d), None));
      return Ok(d);
    }
    // Money first: the node's day, or what the calling tool call may
    // still spend (its thread's budget, a per-call cap).
    self.admit(env, &ask, &input)?;
    let _slot = self
      .paid
      .acquire()
      .await
      .map_err(|_| "the decision service is stopping".to_owned())?;
    let started = Instant::now();
    let grace = Duration::from_millis(500);
    let first = tokio::time::timeout(
      r.timeout + grace,
      env
        .backend
        .decide(&r.model, &input, &ask.questions, r.timeout),
    )
    .await
    .unwrap_or_else(|_| Err("timed out".into()));
    // What a failed call spent is the caller's all the same.
    let charge = |spent: &Option<Spent>| {
      if let Some(sp) = spent {
        match &ask.thread {
          Some(_) => crate::tools::also_spent(sp.clone()),
          None => {
            if let Some(u) = sp.usd {
              env.sink.node_spent(u);
            }
          }
        }
      }
    };
    let first = first.map_err(|f| {
      charge(&f.spent);
      f.why
    });
    let (answers, spent, calibrated, fell_back) = match first {
      Ok((a, s)) => (a, s, true, None),
      Err(e) if r.fallback.is_some() => {
        let model = r.fallback.as_deref().unwrap_or_default();
        let fb = tokio::time::timeout(
          fallback::TIMEOUT + grace,
          env
            .backend
            .fallback(model, &input, &ask.questions, fallback::TIMEOUT),
        )
        .await
        .unwrap_or_else(|_| Err("timed out".into()))
        .map_err(|f| {
          charge(&f.spent);
          f.why
        });
        match fb {
          Ok((a, s)) => (a, s, false, Some(cut(&e, 300))),
          Err(e2) => {
            let why = cut(&format!("{e}; the fallback: {e2}"), 400);
            rec(base(None, Some(&why)));
            return Err(why);
          }
        }
      }
      Err(e) => {
        let why = cut(&e, 300);
        rec(base(None, Some(&why)));
        return Err(why);
      }
    };
    // Every question answered, or none taken: a caller never reads a
    // missing answer as "no".
    if let Some(k) = ask.questions.keys().find(|k| !answers.contains_key(*k)) {
      charge(&Some(spent.clone()));
      let why = format!("no answer to `{k}`");
      rec(base(None, Some(&why)));
      return Err(why);
    }
    let d = Decision {
      profile: ask.profile.clone(),
      answers,
      thresholds: r.thresholds.clone(),
      calibrated,
      model: spent.model.clone(),
      ms: started.elapsed().as_millis() as u64,
      cached: false,
      fell_back,
      spent: Some(spent),
    };
    if ask.thread.is_none()
      && let Some(u) = d.spent.as_ref().and_then(|s| s.usd)
    {
      env.sink.node_spent(u);
    }
    rec(base(Some(&d), None));
    if d.calibrated && r.cache {
      self.cache.lock().expect("cache lock").put(key, d.clone());
    }
    Ok(d)
  }
}

fn summary(input: &str) -> String {
  let one: String = input.split_whitespace().collect::<Vec<_>>().join(" ");
  let s: String = one.chars().take(SUMMARY).collect();
  if s.len() < one.len() {
    format!("{s}…")
  } else {
    s
  }
}

pub(crate) fn cut(s: &str, n: usize) -> String {
  if s.len() <= n {
    return s.to_owned();
  }
  let mut i = n;
  while !s.is_char_boundary(i) {
    i -= 1;
  }
  format!("{}…", &s[..i])
}

/// Asks the decision model `model` of `provider` (named `name`): the
/// answers and what they cost. One request, never retried.
pub(crate) async fn call_provider(
  provider: &stitch_providers::Provider,
  name: &str,
  model: &str,
  input: &str,
  questions: &BTreeMap<String, Question>,
  timeout: Duration,
) -> std::result::Result<(BTreeMap<String, Answer>, Spent), String> {
  use stitch_providers::decisions::DecisionRequest;
  if !provider.has_key() {
    return Err(format!("no key for {name}"));
  }
  let request = DecisionRequest {
    model: model.to_owned(),
    input: input.to_owned(),
    questions: questions.clone(),
    timeout,
  };
  let d = provider.decide(&request).await.map_err(|e| e.to_string())?;
  let full = format!("{name}/{}", d.model);
  let usd = crate::pricing::priced(&full, &d.usage, None);
  Ok((
    d.answers,
    Spent {
      model: full,
      usage: d.usage,
      usd,
    },
  ))
}

/// The node's providers.
pub(crate) struct ProviderBackend {
  pub shared: Weak<Shared>,
}

impl Backend for ProviderBackend {
  fn decide<'a>(
    &'a self,
    model: &'a str,
    input: &'a str,
    questions: &'a BTreeMap<String, Question>,
    timeout: Duration,
  ) -> BoxFuture<'a, Answered> {
    Box::pin(async move {
      let s = self
        .shared
        .upgrade()
        .ok_or(Failed::from("the node is stopping"))?;
      let (provider, name, m) = s
        .config
        .resolve_authenticated(&s.data, Some(model))
        .await
        .map_err(|e| e.to_string())?;
      if !provider.decides(&m) {
        return Err(format!("{model} answers no decisions").into());
      }
      call_provider(&provider, &name, &m, input, questions, timeout)
        .await
        .map_err(Failed::from)
    })
  }

  fn fallback<'a>(
    &'a self,
    model: &'a str,
    input: &'a str,
    questions: &'a BTreeMap<String, Question>,
    timeout: Duration,
  ) -> BoxFuture<'a, Answered> {
    Box::pin(async move {
      let s = self
        .shared
        .upgrade()
        .ok_or(Failed::from("the node is stopping"))?;
      // A role name (`quick`) or `provider/model`.
      let model = crate::modelset::named(&s, None, Some(model)).await;
      let (provider, _, name) = s
        .config
        .resolve_authenticated(&s.data, Some(&model))
        .await
        .map_err(|e| e.to_string())?;
      fallback::ask(&provider, &model, &name, input, questions, timeout).await
    })
  }
}

/// The node's records and day.
pub(crate) struct NodeSink {
  pub shared: Weak<Shared>,
}

impl Sink for NodeSink {
  fn record(&self, line: &Value) {
    let Some(s) = self.shared.upgrade() else {
      return;
    };
    let dir = s.data.join("decide");
    // One line at a time: written whole, never interleaved.
    let _one = s.decide.appending.lock().expect("append lock");
    if let Err(e) = append(&dir, line) {
      eprintln!("decide: a record not kept: {e}");
    }
  }

  fn node_spent(&self, usd: f64) {
    if let Some(s) = self.shared.upgrade() {
      let index = s.state().index.lock().expect("index lock");
      let today = today();
      let (n, total) = day_of(&index, &today);
      let _ = index.set_local(DAY_KEY, Some(&format!("{today} {} {}", total + usd, n + 1)));
    }
  }

  fn node_today(&self) -> f64 {
    self.shared.upgrade().map_or(0.0, |s| {
      day_of(&s.state().index.lock().expect("index lock"), &today()).1
    })
  }

  fn budget_left(&self, thread: &RecordId) -> Option<f64> {
    let s = self.shared.upgrade()?;
    let budget = {
      let index = s.state().index.lock().expect("index lock");
      let r = index.record(thread).ok().flatten()?;
      crate::chat::Limits::from_json(r.fields.get("budget")).usd?
    };
    let spent = crate::detached::spent_by_chain(&s, thread, None)
      .and_then(|(_, u)| u)
      .unwrap_or(0.0);
    Some(budget - spent)
  }
}

fn today() -> String {
  jiff::Zoned::now().date().to_string()
}

/// The node's own decisions today: how many, what they cost.
fn day_of(index: &stitch_store::Index, day: &str) -> (u64, f64) {
  let v = index.local(DAY_KEY).ok().flatten().unwrap_or_default();
  let mut it = v.split_whitespace();
  if it.next() != Some(day) {
    return (0, 0.0);
  }
  let usd = it.next().and_then(|u| u.parse().ok()).unwrap_or(0.0);
  let n = it.next().and_then(|u| u.parse().ok()).unwrap_or(0);
  (n, usd)
}

/// Appends a record line to this month's file.
fn append(dir: &std::path::Path, line: &Value) -> std::io::Result<()> {
  use std::io::Write;
  std::fs::create_dir_all(dir)?;
  let month = Timestamp::now().strftime("%Y-%m").to_string();
  let mut f = std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(dir.join(format!("{month}.jsonl")))?;
  // The whole line in one write.
  f.write_all(format!("{line}\n").as_bytes())
}

/// The latest decision records, newest last.
fn read_log(s: &Shared, profile: Option<&str>, n: usize) -> Vec<Value> {
  let dir = s.data.join("decide");
  let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
    .map(|d| {
      d.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect()
    })
    .unwrap_or_default();
  files.sort();
  let mut out: Vec<Value> = Vec::new();
  for f in files.iter().rev() {
    let text = std::fs::read_to_string(f).unwrap_or_default();
    let mut these: Vec<Value> = text
      .lines()
      .filter_map(|l| serde_json::from_str::<Value>(l).ok())
      .filter(|v| profile.is_none_or(|p| v["profile"] == p))
      .collect();
    these.append(&mut out);
    out = these;
    if out.len() >= n {
      break;
    }
  }
  let skip = out.len().saturating_sub(n);
  out.split_off(skip)
}

/// The user's settings of decisions: per profile, thresholds and the
/// fallback model; for every profile, the decision model and the fallback
/// model (latest `decide-settings` record of the user's for each profile;
/// `*` for every profile's).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Settings {
  pub model: Option<String>,
  /// Every profile's fallback model (`provider/model` or a role such as
  /// `quick`); none: a call the decision model cannot answer fails.
  pub fallback: Option<String>,
  /// The node's own decisions' cap a day (`--daily`).
  pub daily_usd: Option<f64>,
  pub profiles: BTreeMap<String, Override>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Override {
  pub thresholds: BTreeMap<String, f64>,
  /// This profile's fallback model over every profile's: `Some(None)`,
  /// none for it.
  pub fallback: Option<Option<String>>,
}

/// A fallback field as written: a model, or none (`none`, empty).
fn fallback_of(v: &str) -> Option<String> {
  let v = v.trim();
  (!v.is_empty() && v != "none" && v != "off").then(|| v.to_owned())
}

pub(crate) fn settings(s: &Shared) -> Settings {
  let mut recs = {
    let index = s.state().index.lock().expect("index lock");
    index
      .records(Some(KIND))
      .unwrap_or_default()
      .into_iter()
      .filter(|r| r.by_user)
      .collect::<Vec<_>>()
  };
  recs.sort_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
  let mut out = Settings::default();
  for r in recs {
    let Some(p) = r.fields.get("profile").and_then(Value::as_str) else {
      continue;
    };
    if p == "*" {
      if let Some(m) = r.fields.get("model").and_then(Value::as_str) {
        out.model = (!m.is_empty()).then(|| m.to_owned());
      }
      if let Some(f) = r.fields.get("fallback").and_then(Value::as_str) {
        out.fallback = fallback_of(f);
      }
      if let Some(d) = r.fields.get("daily_usd").and_then(Value::as_f64) {
        out.daily_usd = Some(d).filter(|d| d.is_finite() && *d >= 0.0);
      }
      continue;
    }
    let o = Override {
      thresholds: r
        .fields
        .get("thresholds")
        .and_then(Value::as_object)
        .map(|m| {
          m.iter()
            .filter_map(|(k, v)| v.as_f64().filter(|t| valid(*t)).map(|t| (k.clone(), t)))
            .collect()
        })
        .unwrap_or_default(),
      fallback: r
        .fields
        .get("fallback")
        .and_then(Value::as_str)
        .map(fallback_of),
    };
    out.profiles.insert(p.to_owned(), o);
  }
  out
}

fn valid(t: f64) -> bool {
  t.is_finite() && (0.0..1.0).contains(&t)
}

/// A profile as it stands on this node: its defaults under the user's
/// settings.
pub(crate) fn resolve(s: &Shared, name: &str) -> std::result::Result<Resolved, String> {
  let p = profiles::named(name).ok_or_else(|| {
    format!(
      "no decision profile `{name}` ({})",
      profiles::builtin()
        .iter()
        .map(|p| p.name)
        .collect::<Vec<_>>()
        .join(", ")
    )
  })?;
  Ok(resolve_with(p, &settings(s)))
}

pub(crate) fn resolve_with(p: &Profile, user: &Settings) -> Resolved {
  let mut thresholds: BTreeMap<String, f64> = p
    .thresholds
    .iter()
    .map(|(k, t)| ((*k).to_owned(), *t))
    .collect();
  let mut fallback = user.fallback.clone();
  let mut from = "defaults";
  let model = user.model.clone().unwrap_or_else(|| DEFAULT_MODEL.into());
  if let Some(o) = user.profiles.get(p.name) {
    for (k, t) in &o.thresholds {
      thresholds.insert(k.clone(), *t);
    }
    if let Some(f) = &o.fallback {
      fallback = f.clone();
    }
    from = "synced settings";
  }
  if !p.may_fall_back {
    fallback = None;
  }
  Resolved {
    profile: p.name.to_owned(),
    thresholds,
    fallback,
    model,
    daily_usd: user.daily_usd.unwrap_or(NODE_DAILY_USD),
    timeout: p.timeout,
    cache: p.cache,
    from,
  }
}

/// The one entry point: asks `ask` as its profile stands on this node.
pub(crate) async fn ask(s: &Arc<Shared>, ask: Ask) -> std::result::Result<Decision, String> {
  let resolved = resolve(s, &ask.profile)?;
  ask_resolved(s, resolved, ask).await
}

/// `ask` with the profile already resolved (the guard's model and
/// thresholds as its check read them).
pub(crate) async fn ask_resolved(
  s: &Arc<Shared>,
  resolved: Resolved,
  ask: Ask,
) -> std::result::Result<Decision, String> {
  let weak = Arc::downgrade(s);
  let backend = ProviderBackend {
    shared: weak.clone(),
  };
  let sink = NodeSink { shared: weak };
  let vault = s.vault.clone();
  let redact = move |t: &str| vault.redact(t);
  let env = Env {
    backend: &backend,
    sink: &sink,
    redact: &redact,
    resolved,
  };
  s.decide.ask(&env, ask).await
}

/// `stitch decide`: `action` `profiles` (the default), `set`, `try`,
/// `log`.
pub(crate) async fn request(s: &Arc<Shared>, caller: &Caller, input: Value) -> Result<Vec<String>> {
  let action = input
    .get("action")
    .and_then(Value::as_str)
    .unwrap_or("profiles");
  match action {
    "profiles" => Ok(profile_lines(s)),
    "log" => {
      let n = input.get("n").and_then(Value::as_u64).unwrap_or(20) as usize;
      let p = input.get("profile").and_then(Value::as_str);
      let lines: Vec<String> = read_log(s, p, n)
        .iter()
        .map(|v| {
          let mut v = v.clone();
          if let Some(o) = v.as_object_mut() {
            o.remove("summary");
          }
          v.to_string()
        })
        .collect();
      Ok(if lines.is_empty() {
        vec!["no decisions recorded on this node yet".into()]
      } else {
        lines
      })
    }
    "try" => {
      let profile = input
        .get("profile")
        .and_then(Value::as_str)
        .unwrap_or(profiles::TOOL);
      let text = input
        .get("input")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
      let mut a = Ask::new(profile, text);
      if let Some(qs) = input.get("questions").filter(|q| !q.is_null()) {
        a.questions = tool::parse_questions(qs).map_err(Error::Config)?;
      }
      if a.questions.is_empty() {
        return Err(Error::Config(format!(
          "`{profile}` has no questions of its own: give some (--yes-no \"…?\")"
        )));
      }
      let d = ask(s, a).await.map_err(Error::Config)?;
      let mut out = vec![format!(
        "{} ({}, {} ms{}{}{}):",
        d.profile,
        d.model,
        d.ms,
        if d.calibrated { "" } else { ", uncalibrated" },
        if d.cached { ", cached" } else { "" },
        d.spent
          .as_ref()
          .and_then(|s| s.usd)
          .map(|u| format!(", ${u:.6}"))
          .unwrap_or_default()
      )];
      for k in d.answers.keys() {
        let mark = if d.passes(k) { "≥" } else { "<" };
        out.push(format!(
          "  {k}: {} ({mark} {:.2})",
          answer_json(&d.answers[k], 5),
          d.threshold(k)
        ));
      }
      Ok(out)
    }
    "set" => set(s, caller, &input).await,
    other => Err(Error::Config(format!(
      "stitch decide {other}: profiles, set, try or log"
    ))),
  }
}

fn profile_lines(s: &Shared) -> Vec<String> {
  let user = settings(s);
  let mut out = vec![
    format!(
      "decision model: {}",
      user.model.as_deref().unwrap_or(DEFAULT_MODEL)
    ),
    format!(
      "fallback model: {}",
      user.fallback.as_deref().unwrap_or(
        "none (a decision the decision model cannot answer fails; the guard in auto mode then \
         stops what needs one)"
      )
    ),
  ];
  for p in profiles::builtin() {
    let r = resolve_with(p, &user);
    let t: Vec<String> = r
      .thresholds
      .iter()
      .map(|(k, t)| format!("{k} {t:.2}"))
      .collect();
    let questions: Vec<String> = p
      .questions()
      .iter()
      .map(|(k, q)| {
        format!(
          "{k} ({})",
          match q {
            Question::YesNo { .. } => "yes/no".to_owned(),
            Question::Choice { options, .. } => format!("choice of {}", options.len()),
            Question::Score { levels, .. } => format!("score of {}", levels.len()),
          }
        )
      })
      .collect();
    out.push(format!(
      "{}: {}\n  questions: {}\n  thresholds: {} ({})\n  fallback: {}; model {}; timeout {} s",
      p.name,
      p.about,
      if questions.is_empty() {
        "the caller's own".to_owned()
      } else {
        questions.join(", ")
      },
      t.join(", "),
      r.from,
      match (&r.fallback, p.may_fall_back) {
        (Some(m), _) => format!("{m}, uncalibrated"),
        (None, false) if p.name == profiles::LOOSE_ENDS =>
          "none (loose ends' own quick-model path judges instead)".to_owned(),
        (None, false) => "never (the user is asked instead)".to_owned(),
        (None, true) => "none (the call fails)".to_owned(),
      },
      r.model,
      r.timeout.as_secs_f64()
    ));
  }
  let (n, usd) = day_of(&s.state().index.lock().expect("index lock"), &today());
  out.push(format!(
    "the node's own decisions today: {n}, ${usd:.4} of ${:.2} (threads' are counted with them)",
    user.daily_usd.unwrap_or(NODE_DAILY_USD)
  ));
  out
}

/// `stitch decide set [<profile>] [--threshold [key=]p]… [--fallback
/// <model|none>] [--model m]`: a profile's thresholds and fallback, or
/// (no profile, or `*`) every profile's decision model and fallback.
async fn set(s: &Arc<Shared>, caller: &Caller, input: &Value) -> Result<Vec<String>> {
  let name = input
    .get("profile")
    .and_then(Value::as_str)
    .filter(|p| !p.is_empty())
    .unwrap_or("*");
  let model = input.get("model").and_then(Value::as_str);
  let fallback = input.get("fallback").and_then(Value::as_str);
  let daily = input.get("daily_usd").and_then(Value::as_f64);
  if daily.is_some_and(|d| !(d.is_finite() && d >= 0.0)) {
    return Err(Error::Config("--daily: a sum of money, 0 or more".into()));
  }
  // `0.6` (the profile's one question, or every question of a profile
  // with one threshold for all) or `key=0.6`.
  let mut given: Vec<(Option<String>, f64)> = Vec::new();
  for t in input
    .get("thresholds")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
  {
    let (k, p) = match t.split_once('=') {
      Some((k, p)) => (Some(k.trim().to_owned()), p),
      None => (None, t),
    };
    let p: f64 = p
      .trim()
      .parse()
      .map_err(|_| Error::Config(format!("--threshold {t}: a probability (0.6, or key=0.6)")))?;
    if !valid(p) || p == 0.0 && k.is_none() {
      return Err(Error::Config(format!(
        "--threshold {t}: a probability between 0 and 1"
      )));
    }
    given.push((k, p));
  }
  if name == "*" {
    if !given.is_empty() {
      return Err(Error::Config(
        "thresholds are a profile's: `stitch decide set <profile> --threshold …`".into(),
      ));
    }
    if model.is_none() && fallback.is_none() && daily.is_none() {
      return Err(Error::Config(
        "`stitch decide set` without a profile sets the decision model (--model) or the \
         fallback model (--fallback <model|none>) for every profile"
          .into(),
      ));
    }
    let now = settings(s);
    let model = model.map(str::to_owned).or(now.model);
    let fallback = match fallback {
      Some(f) => fallback_of(f),
      None => now.fallback,
    };
    let daily = daily.or(now.daily_usd);
    let mut draft = Draft::new(KIND, Author::user(s.state().node.clone()))?
      .field("profile", "*")?
      .field("fallback", fallback.as_deref().unwrap_or("none"))?;
    if let Some(m) = &model {
      draft = draft.field("model", m.as_str())?;
    }
    if let Some(d) = daily {
      draft = draft.field("daily_usd", d)?;
    }
    let what = format!(
      "set decisions for every node: model {}, fallback {}, the node's own ${:.2} a day",
      model.as_deref().unwrap_or(DEFAULT_MODEL),
      fallback.as_deref().unwrap_or("none"),
      daily.unwrap_or(NODE_DAILY_USD)
    );
    let draft = draft.body(format!("# Decisions\n\n{what}\n"));
    publish(s, caller, &what, draft).await?;
    return Ok(profile_lines(s));
  }
  let p = profiles::named(name).ok_or_else(|| {
    Error::Config(format!(
      "no decision profile `{name}` (stitch decide profiles)"
    ))
  })?;
  if model.is_some() {
    return Err(Error::Config(
      "the decision model is every profile's: `stitch decide set --model m`".into(),
    ));
  }
  if fallback.is_some_and(|f| fallback_of(f).is_some()) && !p.may_fall_back {
    return Err(Error::Config(format!(
      "{name} takes no fallback model: {}",
      if p.name == profiles::LOOSE_ENDS {
        "loose ends have their own quick-model path"
      } else {
        "without the decision model it asks the user (design §2.28)"
      }
    )));
  }
  let now = settings(s).profiles.get(name).cloned().unwrap_or_default();
  let mut o = now;
  let questions = p.questions();
  let mut keys: Vec<String> = questions
    .keys()
    .cloned()
    .chain(p.thresholds.iter().map(|(k, _)| (*k).to_owned()))
    .collect();
  keys.sort();
  keys.dedup();
  for (k, t) in given {
    let k = match k {
      Some(k) if keys.contains(&k) || k == "*" => k,
      // A choice's option: `verdict:done`.
      Some(k)
        if k.split_once(':').is_some_and(|(q, o)| {
          matches!(questions.get(q), Some(Question::Choice { options, .. }) if options.contains_key(o))
        }) =>
      {
        k
      }
      Some(k) => {
        return Err(Error::Config(format!(
          "{name} has no question `{k}` ({})",
          keys.join(", ")
        )));
      }
      // The profile's one question (its options' own thresholds stand).
      None if questions.len() == 1 => questions.keys().next().cloned().unwrap_or_default(),
      // One threshold for every question.
      None if p.thresholds.len() == 1 => p.thresholds[0].0.to_owned(),
      // Several: a bare one would change nothing the named ones decide.
      None => {
        return Err(Error::Config(format!(
          "{name} has several questions: name one (--threshold key=p: {})",
          keys.join(", ")
        )));
      }
    };
    o.thresholds.insert(k, t);
  }
  if let Some(f) = fallback {
    o.fallback = Some(fallback_of(f));
  }
  let mut table = toml::map::Map::new();
  for (k, t) in &o.thresholds {
    table.insert(k.clone(), toml::Value::Float(*t));
  }
  let mut draft = Draft::new(KIND, Author::user(s.state().node.clone()))?
    .field("profile", name)?
    .field("thresholds", toml::Value::Table(table))?;
  if let Some(f) = &o.fallback {
    draft = draft.field("fallback", f.as_deref().unwrap_or("none"))?;
  }
  let said: Vec<String> = o
    .thresholds
    .iter()
    .map(|(k, t)| format!("{k} {t:.2}"))
    .collect();
  let what = format!(
    "set the decision profile {name} for every node: thresholds {}{}",
    said.join(", "),
    o.fallback
      .as_ref()
      .map(|f| format!(", fallback {}", f.as_deref().unwrap_or("none")))
      .unwrap_or_default()
  );
  let draft = draft.body(format!("# Decision profile {name}\n\n{what}\n"));
  publish(s, caller, &what, draft).await?;
  Ok(profile_lines(s))
}

/// Writes a setting for every node: the user's own, asked first when a
/// thread's shell set it, never the node's.
async fn publish(s: &Arc<Shared>, caller: &Caller, what: &str, draft: Draft) -> Result<()> {
  match caller {
    Caller::User => {}
    Caller::Thread(t) => crate::hazard::ask_user(s, t, what).await?,
    Caller::Node => {
      return Err(Error::Config(
        "decision settings are the user's to set: from their terminal or app".into(),
      ));
    }
  }
  let state = s.state();
  let home = state
    .home()
    .ok_or_else(|| Error::Config("no home workspace here".into()))?;
  home.publish(&state.node, draft)?;
  state.index.lock().expect("index lock").refresh(&home)?;
  Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
  use super::*;

  type Rule =
    Box<dyn Fn(&str, &str, &Question) -> std::result::Result<Answer, String> + Send + Sync>;

  /// Answers by a rule over (input, key, question); counts calls; its
  /// fallback answers by another rule, or fails.
  pub(crate) struct FakeBackend {
    pub rule: Rule,
    pub fallback: Option<Rule>,
    pub calls: Mutex<Vec<String>>,
    pub fallbacks: Mutex<usize>,
  }

  impl FakeBackend {
    pub(crate) fn new(
      rule: impl Fn(&str, &str, &Question) -> std::result::Result<Answer, String>
      + Send
      + Sync
      + 'static,
    ) -> FakeBackend {
      FakeBackend {
        rule: Box::new(rule),
        fallback: None,
        calls: Mutex::new(Vec::new()),
        fallbacks: Mutex::new(0),
      }
    }
  }

  fn spent(model: &str, usd: f64) -> Spent {
    Spent {
      model: model.into(),
      usage: stitch_providers::Usage {
        input_tokens: 100,
        ..Default::default()
      },
      usd: Some(usd),
    }
  }

  fn answer_all(
    rule: &Rule,
    input: &str,
    qs: &BTreeMap<String, Question>,
  ) -> std::result::Result<BTreeMap<String, Answer>, String> {
    // `Err("skip")` leaves that answer out (a partial answer).
    let mut out = BTreeMap::new();
    for (k, q) in qs {
      match rule(input, k, q) {
        Ok(a) => {
          out.insert(k.clone(), a);
        }
        Err(e) if e == "skip" => {}
        Err(e) => return Err(e),
      }
    }
    Ok(out)
  }

  impl Backend for FakeBackend {
    fn decide<'a>(
      &'a self,
      _model: &'a str,
      input: &'a str,
      questions: &'a BTreeMap<String, Question>,
      _timeout: Duration,
    ) -> BoxFuture<'a, Answered> {
      self.calls.lock().unwrap().push(input.to_owned());
      let r = answer_all(&self.rule, input, questions);
      Box::pin(async move {
        r.map(|a| (a, spent("gemini/gemini-decision-1", 0.00001)))
          .map_err(Failed::from)
      })
    }

    fn fallback<'a>(
      &'a self,
      model: &'a str,
      input: &'a str,
      questions: &'a BTreeMap<String, Question>,
      _timeout: Duration,
    ) -> BoxFuture<'a, Answered> {
      *self.fallbacks.lock().unwrap() += 1;
      let r = match &self.fallback {
        Some(f) => answer_all(f, input, questions),
        None => Err("no fallback".into()),
      };
      Box::pin(async move { r.map(|a| (a, spent(model, 0.001))).map_err(Failed::from) })
    }
  }

  #[derive(Default)]
  pub(crate) struct ListSink {
    pub lines: Mutex<Vec<Value>>,
    pub node: Mutex<f64>,
  }

  impl Sink for ListSink {
    fn record(&self, line: &Value) {
      self.lines.lock().unwrap().push(line.clone());
    }
    fn node_spent(&self, usd: f64) {
      *self.node.lock().unwrap() += usd;
    }
    fn node_today(&self) -> f64 {
      *self.node.lock().unwrap()
    }
  }

  pub(crate) fn yes(p: f64) -> Answer {
    Answer::YesNo {
      p,
      confidence: p.max(1.0 - p),
    }
  }

  pub(crate) fn env<'a>(
    backend: &'a FakeBackend,
    sink: &'a ListSink,
    profile: &str,
    user: &Settings,
  ) -> Env<'a> {
    static REDACT: fn(&str) -> String = |t: &str| t.replace("hunter2", "«secret:pw»");
    Env {
      backend,
      sink,
      redact: &REDACT,
      resolved: resolve_with(profiles::named(profile).unwrap(), user),
    }
  }

  #[tokio::test]
  async fn profiles_thresholds_and_user_overrides() {
    let b = FakeBackend::new(|input, _, _| Ok(yes(if input.contains("bad") { 0.8 } else { 0.1 })));
    let sink = ListSink::default();
    let svc = Service::default();
    let mut a = Ask::new(profiles::TRIAGE, "a bad issue".into());
    a.thread = Some("2026-11-14T000000Z-gamer-aaaa".parse().unwrap());
    let d = svc
      .ask(
        &env(&b, &sink, profiles::TRIAGE, &Settings::default()),
        a.clone(),
      )
      .await
      .unwrap();
    assert!(d.calibrated && !d.cached);
    assert_eq!(d.yes("not_a_bug"), Some(0.8));
    assert!(d.passes("not_a_bug"), "0.8 ≥ 0.7");
    // The user's threshold over the default.
    let mut user = Settings::default();
    user.profiles.insert(
      profiles::TRIAGE.into(),
      Override {
        thresholds: [("not_a_bug".into(), 0.9)].into(),
        fallback: None,
      },
    );
    let e = env(&b, &sink, profiles::TRIAGE, &user);
    assert_eq!(e.resolved.from, "synced settings");
    let mut a2 = a.clone();
    a2.input = "another bad issue".into();
    let d = svc.ask(&e, a2).await.unwrap();
    assert!(!d.passes("not_a_bug"), "0.8 < 0.9");
    assert!(d.passes("not_actionable"), "its default stands");
    // A thread paid: nothing to the node's day.
    assert_eq!(*sink.node.lock().unwrap(), 0.0);
    // The guard's profiles are set as any other (`stitch decide set
    // guard.fixer --threshold authority=0.35`), the model is every one's.
    user.profiles.insert(
      profiles::GUARD_FIXER.into(),
      Override {
        thresholds: [("authority".into(), 0.35)].into(),
        fallback: None,
      },
    );
    user.model = Some("gemini/other-decision".into());
    let r = resolve_with(profiles::named(profiles::GUARD_FIXER).unwrap(), &user);
    assert_eq!(r.thresholds["authority"], 0.35);
    assert_eq!(r.thresholds["catastrophic"], 0.5);
    assert_eq!(r.model, "gemini/other-decision");
    assert_eq!(r.fallback, None);
  }

  #[tokio::test]
  async fn identical_calls_are_cached_and_records_keep_no_secrets() {
    let b = FakeBackend::new(|_, _, _| Ok(yes(0.2)));
    let sink = ListSink::default();
    let svc = Service::default();
    let e = env(&b, &sink, profiles::TRIAGE, &Settings::default());
    let a = Ask::new(profiles::TRIAGE, "the password is hunter2".into());
    let d1 = svc.ask(&e, a.clone()).await.unwrap();
    let d2 = svc.ask(&e, a.clone()).await.unwrap();
    assert_eq!(
      b.calls.lock().unwrap().len(),
      1,
      "the second from the cache"
    );
    assert!(d2.cached && d2.spent.is_none() && !d1.cached);
    assert_eq!(d1.answers, d2.answers);
    assert!(
      !b.calls.lock().unwrap()[0].contains("hunter2"),
      "redacted before it is sent"
    );
    let lines = sink.lines.lock().unwrap().clone();
    assert_eq!(lines.len(), 2);
    let text = serde_json::to_string(&lines).unwrap();
    assert!(!text.contains("hunter2"), "{text}");
    assert_eq!(lines[0]["profile"], "triage");
    assert_eq!(lines[0]["calibrated"], true);
    assert!(lines[0]["input_hash"].is_string());
    assert!(lines[0]["usd"].is_number());
    assert_eq!(lines[1]["cached"], true);
    // No thread: the node's day pays, once.
    assert!((*sink.node.lock().unwrap() - 0.00001).abs() < 1e-12);
    // Another profile, the same input: asked again.
    let mut a3 = Ask::new(profiles::TOOL, a.input.clone());
    a3.questions = [(
      "x".into(),
      Question::YesNo {
        instructions: "x?".into(),
      },
    )]
    .into();
    svc
      .ask(&env(&b, &sink, profiles::TOOL, &Settings::default()), a3)
      .await
      .unwrap();
    assert_eq!(b.calls.lock().unwrap().len(), 2);
    // The landing gate judges each landing afresh.
    let e = env(&b, &sink, profiles::LAND, &Settings::default());
    let l = Ask::new(profiles::LAND, "+ allow.push(\"x\")".into());
    svc.ask(&e, l.clone()).await.unwrap();
    let again = svc.ask(&e, l).await.unwrap();
    assert!(!again.cached);
    assert_eq!(b.calls.lock().unwrap().len(), 4);
  }

  /// No fallback by default: the call fails. A fallback model the user set
  /// answers, uncalibrated and uncached; never for what gates authority;
  /// a profile may opt out of the global one.
  #[tokio::test]
  async fn the_fallback_is_the_users_setting_none_by_default() {
    let mut b = FakeBackend::new(|_, _, _| Err("503".into()));
    b.fallback = Some(Box::new(|_, _, _| Ok(yes(0.9))));
    let sink = ListSink::default();
    let svc = Service::default();
    let mut a = Ask::new(profiles::TOOL, "x".into());
    a.questions = [(
      "q".into(),
      Question::YesNo {
        instructions: "q?".into(),
      },
    )]
    .into();
    let e = |user: &Settings, p: &str| env(&b, &sink, p, user);
    // None set: every profile fails, the fallback never asked.
    for p in [profiles::TOOL, profiles::GUARD_USER, profiles::TRIAGE] {
      let mut a = a.clone();
      a.profile = p.into();
      let err = svc.ask(&e(&Settings::default(), p), a).await.unwrap_err();
      assert_eq!(err, "503", "{p}");
    }
    assert_eq!(*b.fallbacks.lock().unwrap(), 0);
    assert_eq!(sink.lines.lock().unwrap().last().unwrap()["error"], "503");
    // Set for every profile: it answers, uncalibrated, not cached.
    let user = Settings {
      fallback: Some("openai/gpt-quick".into()),
      ..Settings::default()
    };
    let d = svc.ask(&e(&user, profiles::TOOL), a.clone()).await.unwrap();
    assert!(!d.calibrated);
    assert_eq!(d.fell_back.as_deref(), Some("503"));
    assert_eq!(d.model, "openai/gpt-quick");
    assert_eq!(
      sink.lines.lock().unwrap().last().unwrap()["calibrated"],
      false
    );
    svc.ask(&e(&user, profiles::TOOL), a.clone()).await.unwrap();
    assert_eq!(*b.fallbacks.lock().unwrap(), 2, "not cached");
    let mut g = Ask::new(profiles::GUARD_USER, "rm -rf ~".into());
    g.thread = None;
    assert!(
      !svc
        .ask(&e(&user, profiles::GUARD_USER), g)
        .await
        .unwrap()
        .calibrated
    );
    // What gates authority, and loose ends, never take it.
    for p in [profiles::GUARD_FIXER, profiles::LAND, profiles::LOOSE_ENDS] {
      assert_eq!(e(&user, p).resolved.fallback, None, "{p}");
    }
    // A profile opts out of every profile's, or names its own.
    let mut user2 = user.clone();
    user2.profiles.insert(
      profiles::TOOL.into(),
      Override {
        thresholds: BTreeMap::new(),
        fallback: Some(None),
      },
    );
    user2.profiles.insert(
      profiles::TRIAGE.into(),
      Override {
        thresholds: BTreeMap::new(),
        fallback: Some(Some("anthropic/claude-haiku-4-5".into())),
      },
    );
    assert_eq!(e(&user2, profiles::TOOL).resolved.fallback, None);
    assert_eq!(
      e(&user2, profiles::TRIAGE).resolved.fallback.as_deref(),
      Some("anthropic/claude-haiku-4-5")
    );
    assert_eq!(fallback_of("none"), None);
    assert_eq!(fallback_of(" quick "), Some("quick".into()));
  }

  #[tokio::test]
  async fn choices_scores_and_bad_questions() {
    let b = FakeBackend::new(|_, _, q| {
      Ok(match q {
        Question::Choice { .. } => Answer::Choice {
          choice: "done".into(),
          probabilities: [("done".into(), 0.55), ("continue".into(), 0.45)].into(),
          confidence: 0.55,
        },
        Question::Score { .. } => Answer::Score {
          score: 1.4,
          probabilities: BTreeMap::new(),
          confidence: 0.7,
        },
        _ => yes(0.5),
      })
    });
    let sink = ListSink::default();
    let svc = Service::default();
    let a = Ask::new(profiles::LOOSE_ENDS, "x".into());
    let d = svc
      .ask(
        &env(&b, &sink, profiles::LOOSE_ENDS, &Settings::default()),
        a,
      )
      .await
      .unwrap();
    assert_eq!(d.choice("verdict"), Some(("done", 0.55)));
    assert!(!d.passes("verdict"), "0.55 < 0.6");
    let mut a = Ask::new(profiles::TOOL, "x".into());
    a.questions = [(
      "s".into(),
      Question::Score {
        instructions: "How?".into(),
        levels: vec!["a".into(), "b".into(), "c".into()],
      },
    )]
    .into();
    let e = env(&b, &sink, profiles::TOOL, &Settings::default());
    let d = svc.ask(&e, a.clone()).await.unwrap();
    assert!(d.passes("s"));
    assert_eq!(d.to_json(5)["answers"]["s"]["score"], 1.4);
    a.questions.clear();
    assert!(svc.ask(&e, a.clone()).await.is_err());
    a.questions = [(
      "c".into(),
      Question::Choice {
        instructions: "?".into(),
        options: (0..300).map(|i| (format!("o{i}"), String::new())).collect(),
      },
    )]
    .into();
    assert!(svc.ask(&e, a).await.unwrap_err().contains("255"));
  }

  #[tokio::test]
  async fn a_missing_answer_is_an_error_not_a_no() {
    let b = FakeBackend::new(|_, k, _| {
      if k == "not_actionable" {
        Err("skip".into())
      } else {
        Ok(yes(0.1))
      }
    });
    let sink = ListSink::default();
    let r = Service::default()
      .ask(
        &env(&b, &sink, profiles::TRIAGE, &Settings::default()),
        Ask::new(profiles::TRIAGE, "x".into()),
      )
      .await;
    // The model answered, but not every question: an error, never "no",
    // and what it cost is counted (the node's day here) and recorded.
    assert_eq!(r.unwrap_err(), "no answer to `not_actionable`");
    assert!(
      *sink.node.lock().unwrap() > 0.0,
      "the failed call is paid for"
    );
    assert_eq!(
      sink.lines.lock().unwrap().last().unwrap()["error"],
      "no answer to `not_actionable`"
    );
  }

  /// Secrets come out of the whole request (questions, options' texts,
  /// levels), of what is kept and of errors; a secret in an option's name
  /// is refused.
  #[tokio::test]
  async fn the_whole_request_and_its_record_are_redacted() {
    let b = FakeBackend::new(|_, _, q| {
      if format!("{q:?}").contains("hunter2") {
        Err("echo hunter2".into())
      } else {
        Ok(match q {
          Question::Choice { .. } => Answer::Choice {
            choice: "a".into(),
            probabilities: [("a".into(), 0.9)].into(),
            confidence: 0.9,
          },
          _ => yes(0.2),
        })
      }
    });
    let sink = ListSink::default();
    let svc = Service::default();
    let mut a = Ask::new(profiles::TOOL, "text".into());
    a.questions = [
      (
        "q".to_owned(),
        Question::YesNo {
          instructions: "Is the password hunter2?".into(),
        },
      ),
      (
        "c".to_owned(),
        Question::Choice {
          instructions: "Which?".into(),
          options: [("a".to_owned(), "the one with hunter2".to_owned())].into(),
        },
      ),
    ]
    .into();
    svc
      .ask(
        &env(&b, &sink, profiles::TOOL, &Settings::default()),
        a.clone(),
      )
      .await
      .unwrap();
    let all = serde_json::to_string(&*sink.lines.lock().unwrap()).unwrap();
    assert!(!all.contains("hunter2"), "{all}");
    let mut bad = a.clone();
    bad.questions = [(
      "c".to_owned(),
      Question::Choice {
        instructions: "Which?".into(),
        options: [("hunter2".to_owned(), String::new())].into(),
      },
    )]
    .into();
    let e = svc
      .ask(&env(&b, &sink, profiles::TOOL, &Settings::default()), bad)
      .await
      .unwrap_err();
    assert!(e.contains("secret") && !e.contains("hunter2"), "{e}");
    // An error the backend gave, redacted where it is kept.
    let failing = FakeBackend::new(|_, _, _| Err("the key hunter2 is wrong".into()));
    let mut a2 = a.clone();
    a2.input = "other".into();
    let _ = svc
      .ask(
        &env(&failing, &sink, profiles::TOOL, &Settings::default()),
        a2,
      )
      .await;
    let all = serde_json::to_string(&*sink.lines.lock().unwrap()).unwrap();
    assert!(!all.contains("hunter2"), "{all}");
  }

  /// Money is admitted before each paid call: the node's own day has a
  /// cap; a tool call's decisions (a script's) a count, a sum and what is
  /// left of the thread's budget; what a call spent is counted even when
  /// the call stops midway.
  #[tokio::test]
  async fn money_is_admitted_before_each_paid_call() {
    let b = FakeBackend::new(|_, _, _| Ok(yes(0.1)));
    let sink = ListSink::default();
    *sink.node.lock().unwrap() = 1.0;
    let svc = Service::default();
    let e = svc
      .ask(
        &env(&b, &sink, profiles::TRIAGE, &Settings::default()),
        Ask::new(profiles::TRIAGE, "x".into()),
      )
      .await
      .unwrap_err();
    assert!(e.contains("cap"), "{e}");
    assert!(b.calls.lock().unwrap().is_empty(), "not asked");
    // The user's cap.
    let user = Settings {
      daily_usd: Some(5.0),
      ..Settings::default()
    };
    svc
      .ask(
        &env(&b, &sink, profiles::TRIAGE, &user),
        Ask::new(profiles::TRIAGE, "x".into()),
      )
      .await
      .unwrap();
    // A thread's tool call: past what it may spend, refused.
    let thread: RecordId = "2026-11-14T000000Z-gamer-aaaa".parse().unwrap();
    let mut a = Ask::new(profiles::TOOL, String::new());
    a.questions = [(
      "q".to_owned(),
      Question::YesNo {
        instructions: "q?".into(),
      },
    )]
    .into();
    a.thread = Some(thread);
    let acc = crate::tools::AlsoSpentCell::default();
    acc.lock().unwrap().spent.push(spent("x", PER_CALL_USD));
    let e = crate::tools::spending_into(acc.clone(), async {
      let mut a = a.clone();
      a.input = "y".into();
      svc
        .ask(&env(&b, &sink, profiles::TOOL, &Settings::default()), a)
        .await
    })
    .await
    .unwrap_err();
    assert!(e.contains("a call at most"), "{e}");
    // Within it: asked, and a failed call's cost lands in the
    // accumulator, which outlives the call's future.
    let acc = crate::tools::AlsoSpentCell::default();
    let failing = FakeBackend::new(|_, k, _| {
      if k == "q2" {
        Err("skip".into())
      } else {
        Ok(yes(0.1))
      }
    });
    let mut two = a.clone();
    two.input = "z".into();
    two.questions.insert(
      "q2".into(),
      Question::YesNo {
        instructions: "q2?".into(),
      },
    );
    let fut = crate::tools::spending_into(acc.clone(), async {
      svc
        .ask(
          &env(&failing, &sink, profiles::TOOL, &Settings::default()),
          two,
        )
        .await
    });
    assert!(fut.await.is_err());
    assert_eq!(acc.lock().unwrap().spent.len(), 1);
    assert_eq!(acc.lock().unwrap().decisions, 1);
  }

  /// Records are appended one whole line at a time, whoever writes.
  #[test]
  fn concurrent_records_stay_whole_lines() {
    let d = tempfile::tempdir().unwrap();
    let svc = Arc::new(Service::default());
    let threads: Vec<_> = (0..8)
      .map(|t| {
        let (svc, dir) = (svc.clone(), d.path().to_owned());
        std::thread::spawn(move || {
          for i in 0..200 {
            let _one = svc.appending.lock().unwrap();
            append(&dir, &json!({"t": t, "i": i, "pad": "x".repeat(3000)})).unwrap();
          }
        })
      })
      .collect();
    for t in threads {
      t.join().unwrap();
    }
    let file = std::fs::read_dir(d.path())
      .unwrap()
      .next()
      .unwrap()
      .unwrap()
      .path();
    let text = std::fs::read_to_string(file).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1600);
    assert!(
      lines
        .iter()
        .all(|l| serde_json::from_str::<Value>(l).is_ok())
    );
  }
}
