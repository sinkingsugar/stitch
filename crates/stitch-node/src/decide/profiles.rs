//! Named profiles (design §2.29): a profile is a set of questions, the
//! thresholds its users read the answers against, and whether it may take
//! the fallback model the user sets for when the decision model is
//! unavailable (none by default: the call fails). The defaults are here;
//! the user's synced settings override thresholds and the fallback
//! (`stitch decide set`), the guard's profiles included.

use std::collections::BTreeMap;
use std::time::Duration;

use stitch_providers::decisions::Question;

/// One profile, as built in.
#[derive(Clone, Debug)]
pub(crate) struct Profile {
  pub name: &'static str,
  /// What it decides, for `stitch decide profiles`.
  pub about: &'static str,
  /// Its fixed questions, by key. A use may add questions of its own
  /// (the triage's per issue): their keys' first part (`duplicate` of
  /// `duplicate.i3`) names their threshold.
  pub questions: fn() -> Vec<(String, Question)>,
  /// Default thresholds by question key (`*`: every other question).
  pub thresholds: &'static [(&'static str, f64)],
  /// Whether the fallback model the user sets (`stitch decide set
  /// --fallback`), forced to a JSON schema, may answer when the decision
  /// model cannot (its answers marked uncalibrated). Never for what gates
  /// authority (`guard.fixer`, `land`: they ask the user instead) nor for
  /// loose ends, which have their own quick-model path.
  pub may_fall_back: bool,
  /// The call's limit.
  pub timeout: Duration,
  /// Whether an identical call within minutes is answered from the cache
  /// (not the landing gate's: rare, and each landing is judged afresh).
  pub cache: bool,
}

impl Profile {
  pub(crate) fn questions(&self) -> BTreeMap<String, Question> {
    (self.questions)().into_iter().collect()
  }
}

pub(crate) const GUARD_USER: &str = "guard.user";
pub(crate) const GUARD_FIXER: &str = "guard.fixer";
pub(crate) const LAND: &str = "land";
pub(crate) const TRIAGE: &str = "triage";
pub(crate) const LOOSE_ENDS: &str = "loose-ends";
/// Ad-hoc questions: the `decide` tool and Otter's `Decide`.
pub(crate) const TOOL: &str = "tool";

fn yes_no(q: &str) -> Question {
  Question::YesNo {
    instructions: q.to_owned(),
  }
}

fn guard_user() -> Vec<(String, Question)> {
  crate::hazard::questions(crate::hazard::Role::User)
    .into_iter()
    .map(|(k, q)| (k, yes_no(&q)))
    .collect()
}

fn guard_fixer() -> Vec<(String, Question)> {
  crate::hazard::questions(crate::hazard::Role::Fixers)
    .into_iter()
    .map(|(k, q)| (k, yes_no(&q)))
    .collect()
}

fn land() -> Vec<(String, Question)> {
  crate::hazard::questions(crate::hazard::Role::Landing)
    .into_iter()
    .map(|(k, q)| (k, yes_no(&q)))
    .collect()
}

/// The triage's questions asked of one issue (the pre-pass asks them of
/// each, keyed `<key>.<issue>`, with `duplicate` and `next` as choices
/// among the open issues).
pub(crate) const NOT_A_BUG: &str = "Is this issue, as filed, not a bug in Stitch itself: a request about the user's own work or another program, a misunderstanding of how Stitch works, expected behaviour, or a problem of the environment (a missing tool, a network or provider outage, a full disk) rather than of Stitch's code?";
pub(crate) const NOT_ACTIONABLE: &str = "Is this issue not actionable as filed: too vague to reproduce or fix, with no concrete symptom, place or expected behaviour, or already fixed or superseded by what it describes?";

fn triage() -> Vec<(String, Question)> {
  vec![
    ("not_a_bug".into(), yes_no(NOT_A_BUG)),
    ("not_actionable".into(), yes_no(NOT_ACTIONABLE)),
  ]
}

/// Loose ends' question: what a conversation that stopped is, for the
/// user (`loose::Verdict`), the options as the quick model's prompt
/// describes them.
pub(crate) const LOOSE_VERDICT: &str = "This is an excerpt of a conversation between a user and their AI agent that has stopped. What is it, for the user?";

pub(crate) fn loose_options() -> BTreeMap<String, String> {
  [
    (
      "waiting-on-you",
      "The agent asked the user a question, offered options, or needs a decision, an answer or an action from them to go on; also when it proposed a plan or next steps, or left uncommitted or unapplied work, for the user to approve or decide.",
    ),
    (
      "continue",
      "The work stopped partway or failed, with a next step the user could ask for.",
    ),
    (
      "done",
      "Finished, with nothing proposed or awaiting the user.",
    ),
    ("stale", "Abandoned or no longer worth doing."),
  ]
  .into_iter()
  .map(|(k, v)| (k.to_owned(), v.to_owned()))
  .collect()
}

fn loose_ends() -> Vec<(String, Question)> {
  vec![(
    "verdict".into(),
    Question::Choice {
      instructions: LOOSE_VERDICT.into(),
      options: loose_options(),
    },
  )]
}

fn none() -> Vec<(String, Question)> {
  Vec::new()
}

/// Every profile built in.
pub(crate) fn builtin() -> &'static [Profile] {
  static ALL: std::sync::OnceLock<Vec<Profile>> = std::sync::OnceLock::new();
  ALL.get_or_init(|| {
    vec![
      Profile {
        name: GUARD_USER,
        about: "the hazard guard over the user's own threads' actions (asks at or above)",
        questions: guard_user,
        thresholds: &[("catastrophic", crate::hazard::THRESHOLD)],
        // Auto mode: with no verdict the call fails; a fallback model the
        // user set answers instead, uncalibrated.
        may_fall_back: true,
        timeout: crate::hazard::TIMEOUT,
        cache: true,
      },
      Profile {
        name: GUARD_FIXER,
        about: "the hazard guard over what would ask fixers and the triage (both under: runs)",
        questions: guard_fixer,
        thresholds: &[
          ("catastrophic", crate::hazard::FIXERS_THRESHOLD),
          ("authority", crate::hazard::AUTHORITY_THRESHOLD),
        ],
        may_fall_back: false,
        timeout: crate::hazard::TIMEOUT,
        cache: true,
      },
      Profile {
        name: LAND,
        about: "a fixer's protected diff before landing (any at or above: the user is asked)",
        questions: land,
        thresholds: &[("*", crate::hazard::LANDING_THRESHOLD)],
        may_fall_back: false,
        timeout: crate::hazard::TIMEOUT.saturating_mul(2),
        cache: false,
      },
      Profile {
        name: TRIAGE,
        about: "the harness-issues triage's pre-pass: duplicates, not a bug, not actionable, \
                which next (advisory)",
        questions: triage,
        thresholds: &[
          ("duplicate", 0.6),
          ("not_a_bug", 0.7),
          ("not_actionable", 0.7),
          ("next", 0.0),
        ],
        // Advisory: the triage is a model reading the issues itself.
        may_fall_back: true,
        timeout: Duration::from_secs(10),
        cache: true,
      },
      Profile {
        name: LOOSE_ENDS,
        about: "whether a conversation that stopped waits on the user (the choice's probability \
                at or above: taken; else the quick model judges)",
        questions: loose_ends,
        // Measured on 400 of the user's conversations (design §2.29):
        // "waiting" is taken at 0.6, the rest only at 0.8, so a loose end
        // is rarely missed.
        thresholds: &[("verdict", 0.8), ("verdict:waiting-on-you", 0.6)],
        // Loose ends' own quick-model path judges what this does not.
        may_fall_back: false,
        timeout: Duration::from_secs(8),
        cache: true,
      },
      Profile {
        name: TOOL,
        about: "agents' and skills' own questions (the decide tool, Otter's Decide)",
        questions: none,
        thresholds: &[("*", 0.5)],
        may_fall_back: true,
        timeout: Duration::from_secs(8),
        cache: true,
      },
    ]
  })
}

pub(crate) fn named(name: &str) -> Option<&'static Profile> {
  builtin().iter().find(|p| p.name == name)
}

/// A question key's threshold among `thresholds`: the key, its first part,
/// `*`, else 0.5.
pub(crate) fn threshold_of(thresholds: &BTreeMap<String, f64>, key: &str) -> f64 {
  let head = key.split('.').next().unwrap_or(key);
  thresholds
    .get(key)
    .or_else(|| thresholds.get(head))
    .or_else(|| thresholds.get("*"))
    .copied()
    .unwrap_or(0.5)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_guards_profiles_ask_the_guards_questions_verbatim() {
    let u = named(GUARD_USER).unwrap().questions();
    assert_eq!(
      u["catastrophic"],
      Question::YesNo {
        instructions: crate::hazard::CATASTROPHIC.into()
      }
    );
    assert_eq!(named(GUARD_FIXER).unwrap().questions().len(), 2);
    assert_eq!(
      named(LAND).unwrap().questions().len(),
      crate::hazard::LANDING.len()
    );
    // What gates authority never takes the fallback model: it asks.
    for p in [GUARD_FIXER, LAND, LOOSE_ENDS] {
      assert!(!named(p).unwrap().may_fall_back, "{p}");
    }
    assert!(named(GUARD_USER).unwrap().may_fall_back);
  }

  #[test]
  fn thresholds_by_key_then_prefix_then_star() {
    let t: BTreeMap<String, f64> = [("duplicate".into(), 0.6), ("*".into(), 0.4)].into();
    assert_eq!(threshold_of(&t, "duplicate.i3"), 0.6);
    assert_eq!(threshold_of(&t, "next"), 0.4);
    assert_eq!(threshold_of(&BTreeMap::new(), "x"), 0.5);
  }
}
