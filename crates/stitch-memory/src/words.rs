//! The token heuristics edge talk settled on in the field (2026-07-11 to
//! 07-22), ported as they are: one tokenizer for every gate, so the gates
//! agree on what a word is.
//!
//! - A **distinctive token** is longer than three bytes and not an English
//!   stopword. `len > 3` alone counted that/this/with/have as distinctive,
//!   so filler-padded turns cleared every gate.
//! - **Cue gate:** a message carries a topic when it has four distinctive
//!   tokens; short reactive turns sit next to their tone-twins in embedding
//!   space, so recalling on them surfaces tone, not topic.
//! - **Substance floor:** a memory with fewer than four distinctive tokens
//!   (unsegmented text such as Japanese: twelve codepoints) is never quoted
//!   unasked and never earns credit; it stays in the graph as a stepping
//!   stone.
//! - **Credit:** a shown memory helped when its unique distinctive tokens
//!   come back in the answer, at least max(3, min(⌈0.3 × distinct⌉, 8)):
//!   the fraction alone rewarded short questions and starved long answers.
//! - **Markers:** U+2063 (skip: "not memory-worthy") and U+2060 (dismiss:
//!   "the memories shown did not help", implies skip); zero natural
//!   occurrences in 160k field messages.

use std::collections::BTreeSet;

/// "This message is not memory-worthy": never observed.
pub const SKIP: char = '\u{2063}';
/// "The memories shown this exchange did not help": the shown memories that
/// earned no credit are penalized, and the message is not observed.
pub const DISMISS: char = '\u{2060}';
const SKIP_ASCII: &str = "<!--engram:skip-->";
const DISMISS_ASCII: &str = "<!--engram:dismiss-->";

/// Distinctive tokens a cue needs to recall on (field: reactive 0–3,
/// short genuine topicals 4–5).
pub const CUE_MIN_TOKENS: usize = 4;
/// The substance floor.
pub const SUBSTANCE_MIN_TOKENS: usize = 4;
pub const SUBSTANCE_MIN_CODEPOINTS: usize = 12;
/// Credit: the fraction of distinctive tokens, and its floor and cap.
pub const FEEDBACK_OVERLAP: f32 = 0.3;
pub const CREDIT_MIN_MATCHED: usize = 3;
pub const CREDIT_MATCHED_CAP: usize = 8;

const SEPARATORS: &[char] = &[
  '.', ',', ';', ':', '!', '?', '\'', '"', '(', ')', '[', ']', '{', '}', '*', '#', '`', '-',
];

/// edge talk's ~120 English stopwords (other languages skip the filter).
#[rustfmt::skip]
const STOPWORDS: &[&str] = &[
  "about", "above", "after", "again", "against", "almost", "along", "already", "also", "although",
  "always", "among", "another", "anyway", "around", "because", "been", "before", "being", "below",
  "between", "both", "cannot", "could", "does", "doing", "down", "during", "each", "either",
  "else", "even", "ever", "every", "from", "gonna", "gotta", "have", "haven", "having", "here",
  "hers", "herself", "himself", "however", "into", "itself", "just", "like", "little", "many",
  "maybe", "might", "more", "most", "much", "must", "myself", "need", "never", "okay", "only",
  "onto", "other", "others", "otherwise", "ought", "over", "perhaps", "please", "quite", "rather",
  "really", "same", "should", "since", "some", "something", "sometimes", "still", "such", "sure",
  "than", "that", "thats", "their", "theirs", "them", "themselves", "then", "there", "therefore",
  "these", "they", "thing", "things", "think", "this", "those", "through", "thus", "under",
  "until", "upon", "very", "wanna", "want", "well", "were", "what", "whatever", "when", "where",
  "whether", "which", "while", "whom", "whose", "will", "with", "within", "without", "would",
  "yeah", "your", "yours", "yourself",
];

/// Lowercased tokens, split at whitespace and punctuation.
fn tokens(text: &str) -> Vec<String> {
  text
    .to_lowercase()
    .split(|c: char| c.is_whitespace() || SEPARATORS.contains(&c))
    .filter(|t| !t.is_empty())
    .map(str::to_owned)
    .collect()
}

fn is_distinctive(t: &str) -> bool {
  t.len() > 3 && !STOPWORDS.contains(&t)
}

/// Artifact markup is reference, not experience: its attributes would
/// tokenize into words of their own.
fn strip_artifacts(text: &str) -> String {
  let mut out = String::with_capacity(text.len());
  let mut rest = text;
  while let Some(i) = rest.find("<artifact").or_else(|| rest.find("</artifact")) {
    out.push_str(&rest[..i]);
    match rest[i..].find('>') {
      Some(j) => rest = &rest[i + j + 1..],
      None => {
        rest = "";
      }
    }
  }
  out.push_str(rest);
  out
}

/// Distinctive tokens in a cue (artifact tags stripped).
pub fn distinctive_count(text: &str) -> usize {
  tokens(&strip_artifacts(text))
    .iter()
    .filter(|t| is_distinctive(t))
    .count()
}

/// Whether a cue carries a topic, so recalling on it means something.
pub fn substantive(text: &str) -> bool {
  distinctive_count(text) >= CUE_MIN_TOKENS
}

/// The substance floor: whether a memory may be quoted unasked or earn
/// credit. Segmented text counts distinctive tokens; only a single token
/// (unsegmented script) falls back to codepoints.
pub fn substantial(text: &str) -> bool {
  let text = strip_artifacts(text);
  let all = tokens(&text);
  if all.len() > 1 {
    all.iter().filter(|t| is_distinctive(t)).count() >= SUBSTANCE_MIN_TOKENS
  } else {
    text.trim().chars().count() >= SUBSTANCE_MIN_CODEPOINTS
  }
}

/// Codepoint trigram containment, for unsegmented text (CJK): the share of
/// the memory's non-overlapping trigrams found in the answer.
fn trigram_overlap(memory: &str, answer: &str) -> f32 {
  let cps: Vec<char> = memory.to_lowercase().chars().collect();
  if cps.len() <= 5 {
    return 0.0;
  }
  let answer = answer.to_lowercase();
  let (mut total, mut matched, mut i) = (0, 0, 0);
  while i < cps.len() - 2 {
    let gram: String = cps[i..(i + 3).min(cps.len())].iter().collect();
    total += 1;
    if answer.contains(&gram) {
      matched += 1;
    }
    i += 3;
  }
  matched as f32 / total as f32
}

/// The share of a memory's distinctive tokens found, word for word, in
/// `answer`.
pub fn overlap(memory: &str, answer: &str) -> f32 {
  let all = tokens(memory);
  if all.len() <= 1 {
    return trigram_overlap(memory, answer);
  }
  let words: Vec<&String> = all.iter().filter(|t| is_distinctive(t)).collect();
  if words.is_empty() {
    return 0.0;
  }
  let said: BTreeSet<String> = tokens(answer).into_iter().collect();
  words.iter().filter(|w| said.contains(w.as_str())).count() as f32 / words.len() as f32
}

/// Whether a shown memory helped the answer (see the module docs).
pub fn credit_earned(memory: &str, answer: &str) -> bool {
  let all = tokens(memory);
  if all.len() <= 1 {
    return trigram_overlap(memory, answer) >= FEEDBACK_OVERLAP;
  }
  let words: BTreeSet<&String> = all.iter().filter(|t| is_distinctive(t)).collect();
  if words.is_empty() {
    return false;
  }
  let said: BTreeSet<String> = tokens(answer).into_iter().collect();
  let matched = words.iter().filter(|w| said.contains(w.as_str())).count();
  let need = (3 * words.len())
    .div_ceil(10)
    .clamp(CREDIT_MIN_MATCHED, CREDIT_MATCHED_CAP);
  matched >= need
}

/// Keyword-search terms: tokens of three bytes or more, not stopwords.
pub fn terms(text: &str) -> Vec<String> {
  tokens(text)
    .into_iter()
    .filter(|t| t.len() >= 3 && !STOPWORDS.contains(&t.as_str()))
    .collect()
}

/// Whether the text dismisses the memories it was shown.
pub fn dismiss_marked(text: &str) -> bool {
  text.contains(DISMISS) || text.contains(DISMISS_ASCII)
}

/// Whether the text is not memory-worthy (dismissal implies it).
pub fn skip_marked(text: &str) -> bool {
  text.contains(SKIP) || text.contains(SKIP_ASCII) || dismiss_marked(text)
}

/// Whether the text is only markers (a declined follow-up): kept in the
/// record, never shown.
pub fn marker_only(text: &str) -> bool {
  let mut rest = text.to_owned();
  let mut found = false;
  for m in [
    SKIP.to_string(),
    DISMISS.to_string(),
    SKIP_ASCII.into(),
    DISMISS_ASCII.into(),
  ] {
    if rest.contains(&m) {
      found = true;
      rest = rest.replace(&m, "");
    }
  }
  found && rest.trim().is_empty()
}

/// The text without markers, for showing.
pub fn without_markers(text: &str) -> String {
  text
    .replace([SKIP, DISMISS], "")
    .replace(SKIP_ASCII, "")
    .replace(DISMISS_ASCII, "")
}

/// Whether the text carries a number (a dose, a price, a date): a newer
/// near-duplicate of it is a fresher version of a mutable fact.
pub fn mutable(text: &str) -> bool {
  text.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn cues_tell_topics_from_reactions() {
    assert!(!substantive("oh you're right, thanks!"));
    assert!(!substantive("ok do it"));
    assert!(!substantive("I think that would work with this"));
    // edge talk's calibration example: a short genuine topical passes.
    assert!(substantive("did we talk about my wasabi plants recently?"));
  }

  #[test]
  fn the_substance_floor() {
    assert!(!substantial("Any idea?"));
    assert!(!substantial("here btw,"));
    assert!(!substantial("Do you see it?"));
    assert!(!substantial("How about his suggestion btw?"));
    assert!(substantial("potted with lapillo mix, watered twice weekly"));
    assert!(substantial(
      "甲状腺の薬を毎朝五十マイクログラム飲んでいます"
    ));
    assert!(!substantial("<artifact path=\"a/b/c.md\" kind=\"file\">"));
  }

  #[test]
  fn credit_needs_real_reuse() {
    let memory = "The lead sheet export renders chords above lyrics, one page per song, in PDF.";
    assert!(credit_earned(
      memory,
      "Per what we settled: the export renders chords above the lyrics, one page per song, as a PDF."
    ));
    assert!(!credit_earned(memory, "I don't know about that."));
    // Two of five is not credit any more (it was, by the fraction alone).
    assert!(!credit_earned(
      "How often do the wasabi plants need misting?",
      "The wasabi likes shade."
    ));
    // Word boundaries: "art" is not in "start".
    assert_eq!(overlap("art gallery visit plans", "start"), 0.0);
    assert!(credit_earned(
      "甲状腺の薬を毎朝五十マイクログラム飲んでいます",
      "毎朝五十マイクログラムの甲状腺の薬ですね"
    ));
  }

  #[test]
  fn markers() {
    assert!(skip_marked("fine\u{2063}"));
    assert!(skip_marked("not useful\u{2060}"));
    assert!(dismiss_marked("x <!--engram:dismiss-->"));
    assert!(!dismiss_marked("x <!--engram:skip-->"));
    assert!(marker_only(" \u{2060}\n"));
    assert!(!marker_only("Sure.\u{2060}"));
    assert!(!marker_only("plain"));
    assert_eq!(without_markers("ok\u{2060}"), "ok");
    assert!(mutable("now 50 mcg"));
    assert!(!mutable("no numbers here"));
  }
}
