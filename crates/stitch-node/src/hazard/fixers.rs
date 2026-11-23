//! The fixers' role's one grant of authority (design §2.28): which of a
//! fixer's commands that would ask the user runs unasked instead. Here,
//! behind `tools/protected-paths` with the rest of the guard, so a fixer
//! cannot widen it without the landing gate; `ShellTools::approve` only
//! calls it.

use super::{Role, Verdict};
use crate::rules::Rules;

/// Whether a fixer's (or the triage's) command that its rules would ask
/// about runs unasked: in self-improvement's auto mode, in its sandbox on
/// this machine (not a host), decidable (it only discards local work,
/// nothing `fixer_guarded` names: `Rules::fixer_decidable`, signals only
/// where the sandbox confines them), and the model cleared both questions
/// with a verdict of its own (no verdict, no fallback: it asks).
pub(crate) fn clears(
  verdict: Option<&Verdict>,
  sandboxed: bool,
  on_host: bool,
  rules: &Rules,
  line: &str,
) -> bool {
  sandboxed
    && !on_host
    && verdict.is_some_and(|v| v.role == Role::Fixers && v.clears() && !v.uncalibrated)
    && rules.fixer_decidable(line)
}

#[cfg(test)]
mod tests {
  use super::super::Mode;
  use super::*;

  #[test]
  fn only_a_cleared_decidable_command_in_the_sandbox_runs() {
    let rules = Rules {
      mode: "sandbox".into(),
      fixer: true,
      signals_confined: true,
      ..Rules::default()
    };
    let v = Verdict {
      role: Role::Fixers,
      mode: Mode::On,
      threshold: 0.5,
      authority_threshold: 0.5,
      score: Some(0.05),
      authority: Some(0.05),
      ..Verdict::default()
    };
    let line = "git reset --hard HEAD~1";
    assert!(clears(Some(&v), true, false, &rules, line));
    assert!(!clears(Some(&v), false, false, &rules, line), "unsandboxed");
    assert!(!clears(Some(&v), true, true, &rules, line), "on a host");
    assert!(!clears(None, true, false, &rules, line), "no verdict");
    assert!(!clears(
      Some(&v),
      true,
      false,
      &rules,
      "git push origin main"
    ));
    let flagged = Verdict {
      authority: Some(0.9),
      ..v.clone()
    };
    assert!(!clears(Some(&flagged), true, false, &rules, line));
    let guessed = Verdict {
      uncalibrated: true,
      ..v.clone()
    };
    assert!(
      !clears(Some(&guessed), true, false, &rules, line),
      "a fallback's guess"
    );
    let user = Verdict {
      role: Role::User,
      ..v
    };
    assert!(!clears(Some(&user), true, false, &rules, line));
  }
}
