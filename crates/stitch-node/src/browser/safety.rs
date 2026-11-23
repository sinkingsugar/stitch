//! What asks, in the user's own browser (docs/browser-tool.md §8.3).
//!
//! Pure decisions, so they are tested on their own: the engine gathers the
//! facts (the session's mode, whether the user is watching, the action's
//! declared intent, what its target looks like, the sites involved) and
//! does what [`decide`] says.

use std::fmt;

/// An action's declared purpose (`intent` on `browser_act` and
/// `browser_fill`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
  Read,
  Navigate,
  Input,
  Submit,
  Purchase,
  Send,
  Delete,
  Account,
}

impl Intent {
  pub const NAMES: &'static [&'static str] = &[
    "read", "navigate", "input", "submit", "purchase", "send", "delete", "account",
  ];

  pub fn parse(s: &str) -> Option<Intent> {
    Some(match s {
      "read" => Intent::Read,
      "navigate" => Intent::Navigate,
      "input" => Intent::Input,
      "submit" => Intent::Submit,
      "purchase" => Intent::Purchase,
      "send" => Intent::Send,
      "delete" => Intent::Delete,
      "account" => Intent::Account,
      _ => return None,
    })
  }

  /// In the always-ask set (§8.3): asks in every mode, yolo included.
  pub fn consequential(self) -> bool {
    matches!(
      self,
      Intent::Purchase | Intent::Send | Intent::Delete | Intent::Account
    )
  }

  /// Changes something (anything but reading).
  pub fn changes(self) -> bool {
    self != Intent::Read
  }
}

impl fmt::Display for Intent {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let i = *self as usize;
    f.write_str(Intent::NAMES[i])
  }
}

/// What a click's or a key's target looks like, as the page's script
/// describes it (`resolve`).
#[derive(Clone, Debug, Default)]
pub struct Target {
  /// Its visible text or value.
  pub text: String,
  /// `button`, `a`, `input`…
  pub tag: String,
  /// The input's type (`submit`, `password`…).
  pub kind: String,
  pub autocomplete: String,
  /// A password, one-time code or card field.
  pub sensitive: bool,
  /// The URL of the form it submits, if any.
  pub form_action: String,
}

/// The consequential intent a target's look suggests, if any. Words in
/// English only for now (the user's languages are a later addition).
pub fn looks_like(t: &Target) -> Option<Intent> {
  if t.autocomplete.starts_with("cc-") {
    return Some(Intent::Purchase);
  }
  let text = t.text.to_lowercase();
  let has = |words: &[&str]| words.iter().any(|w| contains_word(&text, w));
  if has(&[
    "buy",
    "buy now",
    "purchase",
    "pay",
    "pay now",
    "place order",
    "place your order",
    "checkout",
    "check out",
    "subscribe",
    "complete order",
    "confirm order",
    "donate",
    "book now",
  ]) {
    return Some(Intent::Purchase);
  }
  if has(&[
    "delete",
    "remove",
    "erase",
    "destroy",
    "close account",
    "deactivate",
    "cancel subscription",
    "unsubscribe",
    "empty trash",
  ]) {
    return Some(Intent::Delete);
  }
  if has(&[
    "send",
    "post",
    "publish",
    "tweet",
    "reply",
    "comment",
    "submit review",
    "share",
    "merge",
    "invite",
    "approve",
    "transfer",
  ]) {
    return Some(Intent::Send);
  }
  if has(&[
    "change password",
    "reset password",
    "authorize",
    "allow access",
    "grant access",
    "permissions",
    "two-factor",
    "2fa",
    "security settings",
    "add member",
    "make admin",
    "sign in with",
  ]) {
    return Some(Intent::Account);
  }
  None
}

fn contains_word(hay: &str, w: &str) -> bool {
  let mut from = 0;
  while let Some(i) = hay[from..].find(w) {
    let start = from + i;
    let end = start + w.len();
    let before = hay[..start].chars().next_back();
    let after = hay[end..].chars().next();
    if !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric) {
      return true;
    }
    from = end;
  }
  false
}

/// The site a URL belongs to: its registrable domain, roughly (the last
/// two labels, three under common second-level suffixes), or its host for
/// IP addresses and single labels.
pub fn site_of(url: &str) -> Option<String> {
  let u = url::Url::parse(url).ok()?;
  let host = u.host_str()?.trim_end_matches('.').to_lowercase();
  if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
    return Some(host);
  }
  let labels: Vec<&str> = host.split('.').collect();
  if labels.len() <= 2 {
    return Some(host);
  }
  const SECOND: &[&str] = &[
    "co", "com", "org", "net", "ac", "gov", "edu", "ne", "or", "go", "gob", "nic", "ltd", "plc",
  ];
  let n = labels.len();
  let take = if SECOND.contains(&labels[n - 2]) && labels[n - 1].len() == 2 {
    3
  } else {
    2
  };
  Some(labels[n - take..].join("."))
}

/// Whether `site` is one of `patterns` (`example.com`, `*.example.com`,
/// or a site's whole name).
pub fn listed(site: &str, host: &str, patterns: &[String]) -> bool {
  patterns.iter().any(|p| {
    let p = p.trim().to_lowercase();
    let p = p.trim_start_matches("*.");
    !p.is_empty() && (site == p || host == p || host.ends_with(&format!(".{p}")))
  })
}

/// The facts a decision is made on.
#[derive(Clone, Debug, Default)]
pub struct Facts {
  /// The declared intent.
  pub intent: Option<Intent>,
  /// What the target looks like, when the action has one.
  pub looks: Option<Intent>,
  /// The target is a password, code or card field and the action types.
  pub types_secret: bool,
  /// The action leaves the tab's current site (navigation).
  pub cross_site: Option<String>,
  /// The current site was granted (config, an earlier approval in this
  /// session).
  pub granted: bool,
  /// The site is in `[browser] sensitive`.
  pub sensitive_site: bool,
  /// The user is watching (§8.2).
  pub watching: bool,
}

/// What to do with an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
  Run,
  /// Ask the user; yolo allows it.
  Ask(String),
  /// Ask the user, yolo or not (the always-ask set).
  AlwaysAsk(String),
  /// Do not act: the reason says what to do instead.
  Refuse(String),
}

pub fn decide(f: &Facts) -> Decision {
  let intent = f.intent.unwrap_or(Intent::Input);
  if f.types_secret {
    return Decision::Refuse(
      "that is a password, one-time code or card field: Stitch never types those. Ask the user with browser_handoff".into(),
    );
  }
  if !intent.changes() && f.cross_site.is_none() {
    return Decision::Run;
  }
  if f.sensitive_site && !f.watching {
    return Decision::Refuse(
      "this site is on the user's sensitive list and nobody is watching: ask the user to watch the tab or take over (browser_handoff)".into(),
    );
  }
  if let Some(site) = &f.cross_site {
    return Decision::AlwaysAsk(format!("go to another site, {site}"));
  }
  if intent.consequential() {
    return Decision::AlwaysAsk(format!("a consequential action ({intent})"));
  }
  if let Some(seen) = f.looks
    && seen.consequential()
  {
    return Decision::AlwaysAsk(format!(
      "an action that looks like {seen}, declared as {intent}"
    ));
  }
  if f.sensitive_site {
    return Decision::Ask("a change on a sensitive site".into());
  }
  if f.granted || f.watching {
    return Decision::Run;
  }
  Decision::Ask("act on a site not granted, with nobody watching".into())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn sites_are_registrable_domains() {
    assert_eq!(
      site_of("https://www.amazon.co.uk/x").as_deref(),
      Some("amazon.co.uk")
    );
    assert_eq!(
      site_of("https://mail.google.com/").as_deref(),
      Some("google.com")
    );
    assert_eq!(
      site_of("http://127.0.0.1:8080/").as_deref(),
      Some("127.0.0.1")
    );
    assert_eq!(
      site_of("http://localhost:3000/").as_deref(),
      Some("localhost")
    );
    assert_eq!(site_of("not a url"), None);
    assert!(listed("bank.com", "www.bank.com", &["*.bank.com".into()]));
    assert!(listed("bank.com", "www.bank.com", &["bank.com".into()]));
    assert!(!listed("notbank.com", "notbank.com", &["bank.com".into()]));
  }

  #[test]
  fn targets_that_look_consequential() {
    let t = |text: &str| Target {
      text: text.into(),
      ..Target::default()
    };
    assert_eq!(looks_like(&t("Place your order")), Some(Intent::Purchase));
    assert_eq!(looks_like(&t("Delete repository")), Some(Intent::Delete));
    assert_eq!(looks_like(&t("Send")), Some(Intent::Send));
    assert_eq!(looks_like(&t("Sender name")), None);
    assert_eq!(looks_like(&t("Search")), None);
    assert_eq!(
      looks_like(&Target {
        autocomplete: "cc-number".into(),
        ..Target::default()
      }),
      Some(Intent::Purchase)
    );
  }

  #[test]
  fn the_table() {
    let base = Facts {
      intent: Some(Intent::Input),
      ..Facts::default()
    };
    // Reading runs anywhere.
    assert_eq!(
      decide(&Facts {
        intent: Some(Intent::Read),
        ..base.clone()
      }),
      Decision::Run
    );
    // In-site input: runs when granted or watched, asks otherwise.
    assert!(matches!(decide(&base), Decision::Ask(_)));
    assert_eq!(
      decide(&Facts {
        watching: true,
        ..base.clone()
      }),
      Decision::Run
    );
    assert_eq!(
      decide(&Facts {
        granted: true,
        ..base.clone()
      }),
      Decision::Run
    );
    // Another site, and the consequential set: always, watched or not.
    for f in [
      Facts {
        cross_site: Some("evil.example".into()),
        watching: true,
        granted: true,
        ..base.clone()
      },
      Facts {
        intent: Some(Intent::Purchase),
        watching: true,
        granted: true,
        ..base.clone()
      },
      Facts {
        looks: Some(Intent::Send),
        watching: true,
        granted: true,
        ..base.clone()
      },
    ] {
      assert!(matches!(decide(&f), Decision::AlwaysAsk(_)), "{f:?}");
    }
    // Secrets are never typed.
    assert!(matches!(
      decide(&Facts {
        types_secret: true,
        watching: true,
        ..base.clone()
      }),
      Decision::Refuse(_)
    ));
    // Sensitive sites: nothing unwatched, every change asks when watched.
    assert!(matches!(
      decide(&Facts {
        sensitive_site: true,
        granted: true,
        ..base.clone()
      }),
      Decision::Refuse(_)
    ));
    assert!(matches!(
      decide(&Facts {
        sensitive_site: true,
        watching: true,
        ..base.clone()
      }),
      Decision::Ask(_)
    ));
  }
}
