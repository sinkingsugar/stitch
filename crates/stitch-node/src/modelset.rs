//! The user's models (design §2.7, "Models"), synced to every node: the
//! default model, the quick one (`spawn` with `model: "quick"`), and
//! settings by pattern (reserved, price, note), all records in the home
//! workspace. A project's `.stitch/settings.toml` may set its own default
//! and quick model (`[models]`), over the synced ones.
//!
//! ```text
//! stitch model default anthropic/claude-opus-5-5
//! stitch model quick anthropic/claude-sonnet-5-5
//! stitch model set 'anthropic/claude-fable-*' --reserved
//! ```
//!
//! A thread's model when none is named: the project's, else the node's
//! `default_model` (its `config.toml`, unset by default), else the synced
//! one, else the built-in one of the
//! first provider with credentials here (`BUILTIN`; subscriptions before
//! API keys, then anthropic, openai, gemini, grok, kimi, deepseek,
//! openrouter, antigravity; nothing
//! reserved), else `DEFAULT_MODEL`. The quick model likewise (the node's
//! `quick_model` over the synced one); when none is set, it is
//! the mid tier of the default's provider when the default is that
//! provider's own built-in one, else the default. One
//! set elsewhere that this node cannot run (an official endpoint with no
//! credentials here) gives way to the node's own, and the log says so.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use stitch_records::{Author, Draft};
use stitch_store::Index;

use crate::node::Shared;
use crate::place::Place;
use crate::{Error, Result};

/// Settings of a pattern: `pattern`, then `price`, `note`, `reserved` as
/// set, or `removed = true`; the latest of a pattern stands.
pub const SETTING: &str = "model-setting";

/// A choice: `role` (`default` or `quick`), `model`, or `removed = true`;
/// the latest of a role stands.
pub const CHOICE: &str = "model-choice";

/// What `spawn`'s `model` takes besides `provider/model`.
pub const ALIASES: [&str; 2] = ["default", "quick"];

/// One pattern's settings: `*` matches any run of characters; where
/// several fit a model, the most specific (longest without its `*`) says
/// first, field by field.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub price: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub note: Option<String>,
  /// An agent's spawn on it is refused unless the user named it in the
  /// thread or one above it (`weave`).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub reserved: Option<bool>,
}

/// The settings of the patterns that fit `model`, merged field by field,
/// the most specific pattern's first; `None` when none fits.
fn configured(config: &BTreeMap<String, ModelSettings>, model: &str) -> Option<ModelSettings> {
  let mut fits: Vec<(&String, &ModelSettings)> = config
    .iter()
    .filter(|(p, _)| crate::mcp::glob(p, model))
    .collect();
  fits.sort_by_key(|(p, _)| std::cmp::Reverse(p.replace('*', "").len()));
  let mut fits = fits.into_iter().map(|(_, s)| s);
  let mut out = fits.next()?.clone();
  for s in fits {
    out.price = out.price.or_else(|| s.price.clone());
    out.note = out.note.or_else(|| s.note.clone());
    out.reserved = out.reserved.or(s.reserved);
  }
  Some(out)
}

/// Whether the settings reserve `model`.
pub fn reserved(config: &BTreeMap<String, ModelSettings>, model: &str) -> bool {
  configured(config, model).is_some_and(|s| s.reserved == Some(true))
}

/// Words many models share, which name none of them.
const GENERIC: &[&str] = &[
  "claude",
  "gemini",
  "gpt",
  "grok",
  "kimi",
  "deepseek",
  "latest",
  "preview",
  "exp",
  "experimental",
  "chat",
  "pro",
  "flash",
  "lite",
  "mini",
  "nano",
  "customtools",
];

/// Whether `text` names `model`: its ID, its name without the provider, or
/// a word of its name that is its own (`fable` of `claude-fable-5-1`,
/// `astra` of `gpt-6-astra`), as a whole word, in any case.
pub fn mentions(text: &str, model: &str) -> bool {
  let text = text.to_lowercase();
  let model = model.to_lowercase();
  let name = model.split_once('/').map_or(model.as_str(), |(_, m)| m);
  if text.contains(name) {
    return true;
  }
  let words: Vec<&str> = text
    .split(|c: char| !c.is_alphanumeric())
    .filter(|w| !w.is_empty())
    .collect();
  name
    .split(['-', '.', '_'])
    .filter(|w| w.len() >= 3 && !w.chars().all(|c| c.is_ascii_digit()))
    .filter(|w| !GENERIC.contains(w))
    .any(|w| words.contains(&w))
}

/// Models a provider lists beyond which `render` shows only those in use.
const LONG_CATALOG: usize = 100;

fn num(x: f64) -> String {
  let s = format!("{x:.2}");
  s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// What `models` says: the default and quick models, then each model with
/// its price where known, reserved ones marked; then why some providers
/// listed none.
pub fn render(
  settings: &BTreeMap<String, ModelSettings>,
  models: &[String],
  problems: &[String],
  default: &str,
  quick: &str,
) -> String {
  let mut out = format!("default: {default}\nquick: {quick} (spawn with model \"quick\")\n");
  if models.is_empty() {
    out.push_str("(none listed)\n");
  } else {
    out.push_str("Prices in USD per million tokens, input/output, where known.\n");
  }
  // A catalog as long as a router's (OpenRouter: hundreds) is listed by
  // the models in use (the default, quick, those with settings), the rest
  // counted.
  let mut count: BTreeMap<&str, usize> = BTreeMap::new();
  for m in models {
    *count.entry(provider_of(m)).or_default() += 1;
  }
  let is =
    |said: &str, m: &str| said == m || said.strip_prefix(m).is_some_and(|r| r.starts_with(' '));
  let mut left: BTreeMap<&str, usize> = BTreeMap::new();
  for m in models {
    let p = provider_of(m);
    if count[p] > LONG_CATALOG
      && !is(default, m)
      && !is(quick, m)
      && configured(settings, m).is_none()
    {
      *left.entry(p).or_default() += 1;
      continue;
    }
    let set = configured(settings, m).unwrap_or_default();
    let name = m.split_once('/').map_or(m.as_str(), |(_, n)| n);
    out.push_str(m);
    let price = set
      .price
      .or_else(|| crate::pricing::list_price(name).map(|(i, o)| format!("{}/{}", num(i), num(o))));
    if let Some(p) = price {
      out.push_str(&format!("  {p}"));
    }
    if set.reserved == Some(true) {
      out.push_str("  (reserved: only when the user asks for it by name)");
    }
    if let Some(n) = set.note {
      out.push_str(&format!("  ({n})"));
    }
    out.push('\n');
  }
  for (p, n) in left {
    out.push_str(&format!(
      "{p}: {n} more models, not listed here; any of its model IDs works as {p}/<id>\n"
    ));
  }
  for p in problems {
    out.push_str(&format!("not listed: {p}\n"));
  }
  out
}

// ---------------------------------------------------------------------
// The records.

/// The standing records of `kind`, oldest first, by `key` (removed ones
/// gone), with the node that wrote each.
fn standing(index: &Index, kind: &str, key: &str) -> BTreeMap<String, (Value, String)> {
  let mut records = index.records(Some(kind)).unwrap_or_default();
  records.sort_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
  let mut out = BTreeMap::new();
  for r in records {
    let Some(k) = r.fields.get(key).and_then(Value::as_str).map(str::to_owned) else {
      continue;
    };
    if r.fields.get("removed").and_then(Value::as_bool) == Some(true) {
      out.remove(&k);
    } else {
      out.insert(k, (r.fields.clone(), r.node.clone()));
    }
  }
  out
}

fn settings_of(fields: &Value) -> ModelSettings {
  let s = |k: &str| fields.get(k).and_then(Value::as_str).map(str::to_owned);
  ModelSettings {
    price: s("price"),
    note: s("note"),
    reserved: fields.get("reserved").and_then(Value::as_bool),
  }
}

/// The user's model settings, as every node has them.
pub(crate) fn settings(s: &Shared) -> BTreeMap<String, ModelSettings> {
  standing(
    &s.state().index.lock().expect("index lock"),
    SETTING,
    "pattern",
  )
  .into_iter()
  .map(|(p, (f, _))| (p, settings_of(&f)))
  .collect()
}

/// The synced model of `role` (`default`, `quick`).
fn synced(s: &Shared, role: &str) -> Option<String> {
  standing(&s.state().index.lock().expect("index lock"), CHOICE, "role")
    .remove(role)
    .and_then(|(f, _)| f.get("model").and_then(Value::as_str).map(str::to_owned))
}

/// The model of `role` this node's `config.toml` sets (`default_model`,
/// `quick_model`).
fn node_own(s: &Shared, role: &str) -> Option<String> {
  match role {
    "quick" => s.config.quick_model.clone(),
    _ => s.config.default_model.clone(),
  }
}

/// The model of `role` set for `place`: the project's, else this node's
/// `config.toml`, else the synced one.
fn chosen(s: &Shared, place: Option<&Place>, role: &str) -> Option<String> {
  let own = place.and_then(|p| {
    let models = s.place_files(p).settings.models;
    match role {
      "quick" => models.quick,
      _ => models.default,
    }
  });
  own
    .or_else(|| node_own(s, role))
    .or_else(|| synced(s, role))
}

/// The built-in models of each official provider: (provider, default,
/// quick), in the order the providers are tried. Quick is the mid tier,
/// since spawned worker threads do real edits on it. `None`: no tier is
/// known, so quick is the default.
pub(crate) const BUILTIN: &[(&str, &str, Option<&str>)] = &[
  ("anthropic", "claude-opus-5-5", Some("claude-sonnet-5-5")),
  // The tiers of OpenAI's `-luna`, `-sol` and `-terra` are not told
  // anywhere in the code, so no quick one is named.
  ("openai", "gpt-5.6-sol", None),
  ("gemini", "gemini-3.1-pro-preview", Some("gemini-3.5-flash")),
  ("kimi", "kimi-k3.1-pro", Some("kimi-k2.7-code")),
  ("deepseek", "deepseek-v4-pro", Some("deepseek-flash")),
  // A router to every vendor's models: Claude's pair, by its IDs there.
  (
    "openrouter",
    "anthropic/claude-opus-5.5",
    Some("anthropic/claude-sonnet-5.5"),
  ),
  // The Google Antigravity plan through agy (design §2.8, Antigravity):
  // its largest Gemini, and Flash for quick.
  ("antigravity", "gemini-4-hydrogen", Some("gemini-3.8-flash")),
];

/// The provider's order when none is configured.
const ORDER: &[&str] = &[
  "anthropic",
  "openai",
  "gemini",
  "grok",
  "kimi",
  "deepseek",
  "openrouter",
  // A subscription, so before every API key; last among them: it runs
  // through another program (agy) rather than a native adapter.
  "antigravity",
];

/// The provider of `model` (`provider/model`).
fn provider_of(model: &str) -> &str {
  model.split_once('/').map_or(model, |(p, _)| p)
}

/// The built-in default for the providers with these `billing`s (name and
/// "subscription", "API key" or "none"): the first of subscriptions, else
/// of API keys, in `ORDER`, whose model `usable` accepts.
fn pick_default(billing: &[(&str, &str)], usable: &dyn Fn(&str) -> bool) -> Option<String> {
  for tier in ["subscription", "API key"] {
    for p in ORDER {
      let has = billing.iter().any(|(n, b)| n == p && *b == tier);
      let Some((_, d, _)) = BUILTIN.iter().find(|(n, ..)| n == p) else {
        continue;
      };
      let model = format!("{p}/{d}");
      if has && usable(&model) {
        return Some(model);
      }
    }
  }
  None
}

/// The built-in quick model beside `default`: its provider's mid tier when
/// `default` is that provider's own table default, else the default itself
/// (a Haiku default keeps Haiku for quick). The flag says whether the table
/// gave it.
fn pick_quick(default: &str, usable: &dyn Fn(&str) -> bool) -> (String, bool) {
  let p = provider_of(default);
  if let Some((_, d, Some(q))) = BUILTIN.iter().find(|(n, ..)| *n == p)
    && default == format!("{p}/{d}")
  {
    let model = format!("{p}/{q}");
    if usable(&model) {
      return (model, true);
    }
  }
  (default.to_owned(), false)
}

/// Whether the built-in `model` may be picked: not reserved, and in the
/// provider's model list when this node already has one (none is asked
/// for here).
fn usable(s: &Shared, settings: &BTreeMap<String, ModelSettings>, model: &str) -> bool {
  if reserved(settings, model) {
    return false;
  }
  let (provider, name) = model.split_once('/').unwrap_or((model, ""));
  match s.models.try_lock() {
    Ok(cached) => cached
      .get(provider)
      .is_none_or(|(_, ids)| ids.iter().any(|i| i == name)),
    Err(_) => true,
  }
}

/// This node's own default and whether it is the built-in one: its
/// config's, else the first provider with credentials here (a sign-in
/// before a key), else `DEFAULT_MODEL`.
async fn node_default(s: &Shared) -> (String, bool) {
  if let Some(m) = &s.config.default_model {
    return (m.clone(), false);
  }
  let mut billing = Vec::new();
  for p in ORDER {
    billing.push((*p, s.config.billing(&s.data, p).await));
  }
  let settings = settings(s);
  match pick_default(&billing, &|m| usable(s, &settings, m)) {
    Some(m) => (m, true),
    None => (crate::config::DEFAULT_MODEL.into(), true),
  }
}

/// The quick model when none is set: see `pick_quick`; the flag says
/// whether it is built-in (its own, or the built-in default).
async fn quick_fallback(s: &Shared, default: &str, default_builtin: bool) -> (String, bool) {
  let settings = settings(s);
  let (m, table) = pick_quick(default, &|m| usable(s, &settings, m));
  (m, table || default_builtin)
}

/// The default model for `place`, as set (no check that this node runs
/// it), and whether it is the built-in one: for what is only said.
pub(crate) async fn default_said(s: &Shared, place: Option<&Place>) -> (String, bool) {
  match chosen(s, place, "default") {
    Some(m) => (m, false),
    None => node_default(s).await,
  }
}

/// The quick model for `place`, as set, else the built-in one beside the
/// default; and whether it is built-in.
pub(crate) async fn quick_said(s: &Shared, place: Option<&Place>) -> (String, bool) {
  match chosen(s, place, "quick") {
    Some(m) => (m, false),
    None => {
      let (d, builtin) = default_said(s, place).await;
      quick_fallback(s, &d, builtin).await
    }
  }
}

/// A model as shown: `(built-in)` after one nobody chose.
pub(crate) fn shown((model, builtin): &(String, bool)) -> String {
  if *builtin {
    format!("{model} (built-in)")
  } else {
    model.clone()
  }
}

/// Whether this node can run `model`: its provider is here, and an
/// official endpoint has credentials. (A custom endpoint may need none.)
async fn runnable(s: &Shared, model: &str) -> bool {
  let provider = provider_of(model);
  let Some(cfg) = s.config.provider_config(provider) else {
    return false;
  };
  // A built-in endpoint (kimi, deepseek, openrouter) is an official one.
  let custom = cfg.base_url.is_some() && s.config.providers.contains_key(provider);
  custom || s.config.billing(&s.data, provider).await != "none"
}

/// The model a thread in `place` runs on when none is named.
pub(crate) async fn default_model(s: &Shared, place: Option<&Place>) -> String {
  let mine = node_default(s).await.0;
  match chosen(s, place, "default") {
    Some(m) if m == mine || runnable(s, &m).await => m,
    Some(m) => {
      eprintln!("models: this node does not run the default {m}: {mine} instead");
      mine
    }
    None => mine,
  }
}

/// The quick model for `place`, else the built-in one beside the default.
pub(crate) async fn quick_model(s: &Shared, place: Option<&Place>) -> String {
  match chosen(s, place, "quick") {
    Some(m) if runnable(s, &m).await => return m,
    Some(m) => {
      eprintln!("models: this node does not run the quick model {m}: the default's instead")
    }
    None => {}
  }
  let d = default_model(s, place).await;
  quick_fallback(s, &d, false).await.0
}

/// A model as named (`None`, `default`, `quick`, or `provider/model`),
/// for a thread in `place`.
pub(crate) async fn named(s: &Shared, place: Option<&Place>, model: Option<&str>) -> String {
  match model {
    None | Some("default") => default_model(s, place).await,
    Some("quick") => quick_model(s, place).await,
    Some(m) => m.to_owned(),
  }
}

/// Shared resolution for a chat and its clients. Only deliberate choices
/// are stored on a thread; merely displaying a default must never pin it.
pub(crate) async fn chat_model(
  s: &Shared,
  thread: Option<&stitch_records::RecordId>,
  place: &Place,
  model: Option<&str>,
) -> String {
  let inherited =
    thread.and_then(|t| crate::meta::thread_model(&s.state().index.lock().expect("index lock"), t));
  named(s, Some(place), model.or(inherited.as_deref())).await
}

// ---------------------------------------------------------------------
// The user's requests (`stitch model`).

/// What `Request::ModelSettings` takes: `action` `ls`; `set` or `rm` of a
/// pattern (`set` changes only the fields given: an empty `price` or
/// `note` clears it); `default` or `quick` (with `model`: sets it, with
/// `unset`: removes it, else says it).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
  pub action: String,
  #[serde(default)]
  pub pattern: Option<String>,
  #[serde(default)]
  pub price: Option<String>,
  #[serde(default)]
  pub note: Option<String>,
  #[serde(default)]
  pub reserved: Option<bool>,
  #[serde(default)]
  pub model: Option<String>,
  #[serde(default)]
  pub unset: bool,
  /// `ls`: the models of this node instead.
  #[serde(default)]
  pub node: Option<String>,
}

/// Acts on the user's models; what to print.
pub(crate) async fn request(s: &Arc<Shared>, input: Input) -> Result<Vec<String>> {
  let pattern = || {
    input
      .pattern
      .clone()
      .filter(|p| !p.trim().is_empty())
      .ok_or_else(|| Error::Config("which models: a pattern (`anthropic/claude-fable-*`)".into()))
  };
  let index = || s.state().index.lock().expect("index lock");
  match input.action.as_str() {
    "ls" => {
      let (models, problems, default) = crate::node::models_of(s, input.node.as_deref()).await?;
      let quick = shown(&quick_said(s, None).await);
      let here = input
        .node
        .as_deref()
        .is_none_or(|n| n == s.state().node.as_str());
      let default = match default_said(s, None).await {
        d @ (_, true) if here && d.0 == default => shown(&d),
        _ => default,
      };
      let mut out: Vec<String> = render(&settings(s), &models, &problems, &default, &quick)
        .lines()
        .map(str::to_owned)
        .collect();
      if here {
        for role in ["default", "quick"] {
          if let Some(m) = node_own(s, role) {
            out.push(format!(
              "this node's {role}: {m} ({role}_model in config.toml, over every node's)"
            ));
          }
        }
      }
      let choices = standing(&index(), CHOICE, "role");
      let patterns = standing(&index(), SETTING, "pattern");
      out.push(String::new());
      if choices.is_empty() && patterns.is_empty() {
        out.push(
          "nothing set: stitch model default|quick <provider/model>, stitch model set <pattern> \
           --reserved"
            .into(),
        );
      } else {
        out.push("set for every node:".into());
        for (role, (f, node)) in choices {
          let m = f.get("model").and_then(Value::as_str).unwrap_or("");
          out.push(format!("  {role}: {m}  (set on {node})"));
        }
        for (p, (f, node)) in patterns {
          out.push(format!(
            "  {p}  {}  (set on {node})",
            said(&settings_of(&f))
          ));
        }
      }
      Ok(out)
    }
    "set" => {
      let pattern = pattern()?;
      let mut now = standing(&index(), SETTING, "pattern")
        .remove(&pattern)
        .map(|(f, _)| settings_of(&f))
        .unwrap_or_default();
      if let Some(p) = &input.price {
        now.price = Some(p.clone()).filter(|p| !p.is_empty());
      }
      if let Some(n) = &input.note {
        now.note = Some(n.clone()).filter(|n| !n.is_empty());
      }
      if let Some(r) = input.reserved {
        now.reserved = r.then_some(true);
      }
      publish_setting(s, &pattern, Some(&now))?;
      Ok(vec![format!("{pattern}: {}", said(&now))])
    }
    "rm" => {
      let pattern = pattern()?;
      if !standing(&index(), SETTING, "pattern").contains_key(&pattern) {
        return Err(Error::Config(format!("no model settings for `{pattern}`")));
      }
      publish_setting(s, &pattern, None)?;
      Ok(vec![format!("{pattern}: removed")])
    }
    role @ ("default" | "quick") => {
      if input.unset {
        if synced(s, role).is_none() {
          return Err(Error::Config(format!("no {role} model set")));
        }
        publish_choice(s, role, None)?;
        return Ok(vec![format!(
          "{role}: unset (now {})",
          if role == "quick" {
            shown(&quick_said(s, None).await)
          } else {
            shown(&default_said(s, None).await)
          }
        )]);
      }
      match input.model.as_deref().filter(|m| !m.trim().is_empty()) {
        Some(m) => {
          if !m.contains('/') {
            return Err(Error::Config(format!(
              "`{m}`: name a model as provider/model"
            )));
          }
          publish_choice(s, role, Some(m))?;
          let mut out = vec![format!("{role}: {m} (every node)")];
          if let Some(own) = node_own(s, role) {
            out.push(format!(
              "this node keeps its own: {own} ({role}_model in config.toml)"
            ));
          }
          Ok(out)
        }
        None => Ok(vec![match (node_own(s, role), synced(s, role)) {
          (Some(own), every) => format!(
            "{role}: {own} (this node's config.toml; every node: {})",
            every.as_deref().unwrap_or("none set")
          ),
          (None, Some(m)) => format!("{role}: {m}"),
          (None, None) if role == "quick" => {
            format!("quick: none set ({})", shown(&quick_said(s, None).await))
          }
          (None, None) => format!(
            "default: none set (this node's, {})",
            shown(&node_default(s).await)
          ),
        }]),
      }
    }
    other => Err(Error::Config(format!(
      "no action `{other}`: ls, set, rm, default or quick"
    ))),
  }
}

/// Settings as a line.
fn said(s: &ModelSettings) -> String {
  let mut out = Vec::new();
  if s.reserved == Some(true) {
    out.push("reserved".to_owned());
  }
  if let Some(p) = &s.price {
    out.push(format!("price {p}"));
  }
  if let Some(n) = &s.note {
    out.push(format!("note \"{n}\""));
  }
  if out.is_empty() {
    "(nothing)".into()
  } else {
    out.join(", ")
  }
}

fn publish(s: &Shared, draft: Draft) -> Result<()> {
  let state = s.state();
  let home = state
    .home()
    .ok_or_else(|| Error::Config("no home workspace here".into()))?;
  home.publish(&state.node, draft)?;
  state.index.lock().expect("index lock").refresh(&home)?;
  Ok(())
}

/// Writes a pattern's settings (`None`: removed) for every node.
fn publish_setting(s: &Shared, pattern: &str, settings: Option<&ModelSettings>) -> Result<()> {
  let mut draft =
    Draft::new(SETTING, Author::user(s.state().node.clone()))?.field("pattern", pattern)?;
  match settings {
    None => {
      draft = draft
        .field("removed", true)?
        .body(format!("# Model settings of {pattern} removed\n"));
    }
    Some(set) => {
      if let Some(p) = &set.price {
        draft = draft.field("price", p.as_str())?;
      }
      if let Some(n) = &set.note {
        draft = draft.field("note", n.as_str())?;
      }
      if let Some(r) = set.reserved {
        draft = draft.field("reserved", r)?;
      }
      draft = draft.body(format!("# Model settings of {pattern}: {}\n", said(set)));
    }
  }
  publish(s, draft)
}

/// Writes the model of `role` (`None`: removed) for every node.
fn publish_choice(s: &Shared, role: &str, model: Option<&str>) -> Result<()> {
  let mut draft = Draft::new(CHOICE, Author::user(s.state().node.clone()))?.field("role", role)?;
  draft = match model {
    Some(m) => draft
      .field("model", m)?
      .body(format!("# The {role} model: {m}\n")),
    None => draft
      .field("removed", true)?
      .body(format!("# The {role} model unset\n")),
  };
  publish(s, draft)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_model_is_named_by_its_own_words() {
    assert!(mentions("ask Fable about it", "anthropic/claude-fable-5-1"));
    assert!(mentions("run it on gpt-6-astra", "openai/gpt-6-astra"));
    assert!(mentions("use astra.", "openai/gpt-6-astra"));
    assert!(mentions("try argon", "gemini/gemini-4-argon"));
    assert!(!mentions(
      "a fabled bug in the claude 5 code",
      "anthropic/claude-fable-5-1"
    ));
    assert!(!mentions("use gemini pro", "gemini/gemini-4-argon"));
    assert!(!mentions("gpt 6 please", "openai/gpt-6-astra"));
  }

  #[test]
  fn settings_merge_from_the_most_specific_pattern() {
    let mut config = BTreeMap::new();
    config.insert(
      "anthropic/claude-fable-*".into(),
      ModelSettings {
        reserved: Some(true),
        ..Default::default()
      },
    );
    config.insert(
      "anthropic/claude-fable-5-1".into(),
      ModelSettings {
        note: Some("newest".into()),
        ..Default::default()
      },
    );
    assert!(reserved(&config, "anthropic/claude-fable-5-1"));
    assert!(reserved(&config, "anthropic/claude-fable-5"));
    assert!(!reserved(&config, "anthropic/claude-opus-5-5"));
    let models = ["anthropic/claude-fable-5-1", "anthropic/claude-haiku-5-5"].map(String::from);
    let out = render(
      &config,
      &models,
      &[],
      "anthropic/claude-opus-5-5",
      "anthropic/claude-sonnet-5-5",
    );
    assert!(out.starts_with(
      "default: anthropic/claude-opus-5-5\nquick: anthropic/claude-sonnet-5-5 (spawn with \
       model \"quick\")\n"
    ));
    assert!(out.contains(
      "\nanthropic/claude-fable-5-1  10/50  (reserved: only when the user asks for it by \
       name)  (newest)\n"
    ));
    assert!(out.contains("\nanthropic/claude-haiku-5-5  0.1/0.5\n"));
  }

  #[test]
  fn a_routers_catalog_lists_the_models_in_use() {
    let mut models: Vec<String> = (0..150)
      .map(|i| format!("openrouter/vendor/model-{i}"))
      .collect();
    models.push("anthropic/claude-haiku-5-5".into());
    let mut settings = BTreeMap::new();
    settings.insert(
      "openrouter/vendor/model-7".to_owned(),
      ModelSettings {
        note: Some("cheap".into()),
        ..Default::default()
      },
    );
    let out = render(
      &settings,
      &models,
      &[],
      "openrouter/vendor/model-3 (built-in)",
      "openrouter/vendor/model-4",
    );
    let listed: Vec<&str> = out
      .lines()
      .filter(|l| l.starts_with("openrouter/") || l.starts_with("anthropic/"))
      .collect();
    assert_eq!(
      listed,
      [
        "openrouter/vendor/model-3",
        "openrouter/vendor/model-4",
        "openrouter/vendor/model-7  (cheap)",
        "anthropic/claude-haiku-5-5  0.1/0.5",
      ]
    );
    assert!(out.contains(
      "\nopenrouter: 147 more models, not listed here; any of its model IDs works as \
       openrouter/<id>\n"
    ));
  }

  fn always(_: &str) -> bool {
    true
  }

  fn pick(billing: &[(&str, &str)]) -> (String, String) {
    let d = pick_default(billing, &always).unwrap_or_else(|| crate::config::DEFAULT_MODEL.into());
    let q = pick_quick(&d, &always).0;
    (d, q)
  }

  #[test]
  fn antigravity_is_the_last_subscription_and_before_any_key() {
    let pair = (
      "antigravity/gemini-4-hydrogen".to_owned(),
      "antigravity/gemini-3.8-flash".to_owned(),
    );
    assert_eq!(pick(&[("antigravity", "subscription")]), pair);
    assert_eq!(
      pick(&[("anthropic", "API key"), ("antigravity", "subscription")]),
      pair
    );
    let both = [("openai", "subscription"), ("antigravity", "subscription")];
    assert_eq!(pick(&both).0, "openai/gpt-5.6-sol");
    // A list already held without the model: skipped.
    let listed = |m: &str| m != "antigravity/gemini-4-hydrogen";
    let billing = [("antigravity", "subscription"), ("gemini", "API key")];
    assert_eq!(
      pick_default(&billing, &listed).unwrap(),
      "gemini/gemini-3.1-pro-preview"
    );
  }

  #[test]
  fn an_openai_key_alone_gives_the_openai_pair() {
    let (d, q) = pick(&[("openai", "API key")]);
    assert_eq!(
      (d.as_str(), q.as_str()),
      ("openai/gpt-5.6-sol", "openai/gpt-5.6-sol")
    );
  }

  #[test]
  fn subscriptions_come_before_keys_then_the_fixed_order() {
    let both = [("anthropic", "subscription"), ("openai", "subscription")];
    assert_eq!(pick(&both).0, "anthropic/claude-opus-5-5");
    assert_eq!(pick(&both).1, "anthropic/claude-sonnet-5-5");
    let mixed = [("anthropic", "API key"), ("openai", "subscription")];
    assert_eq!(pick(&mixed).0, "openai/gpt-5.6-sol");
    let keys = [("deepseek", "API key"), ("gemini", "API key")];
    assert_eq!(
      pick(&keys),
      (
        "gemini/gemini-3.1-pro-preview".into(),
        "gemini/gemini-3.5-flash".into()
      )
    );
    // OpenRouter last, with Claude's pair by its IDs there.
    let keys = [("openrouter", "API key"), ("deepseek", "API key")];
    assert_eq!(pick(&keys).0, "deepseek/deepseek-v4-pro");
    assert_eq!(
      pick(&[("openrouter", "API key")]),
      (
        "openrouter/anthropic/claude-opus-5.5".into(),
        "openrouter/anthropic/claude-sonnet-5.5".into()
      )
    );
  }

  #[test]
  fn a_reserved_or_unlisted_model_is_skipped() {
    let mut settings = BTreeMap::new();
    settings.insert(
      "anthropic/claude-opus-*".to_owned(),
      ModelSettings {
        reserved: Some(true),
        ..Default::default()
      },
    );
    let ok = |m: &str| !reserved(&settings, m);
    let billing = [("anthropic", "subscription"), ("openai", "API key")];
    assert_eq!(pick_default(&billing, &ok).unwrap(), "openai/gpt-5.6-sol");
    let (q, table) = pick_quick("anthropic/claude-opus-5-5", &|m| {
      m != "anthropic/claude-sonnet-5-5"
    });
    assert_eq!((q.as_str(), table), ("anthropic/claude-opus-5-5", false));
  }

  #[test]
  fn a_gemini_key_alone_picks_models_the_real_gemini_list_has() {
    // Gemini's list (2026-11-03): no `gemini-3.1-pro`, no `gemini-3-flash`.
    let listed = [
      "gemini-3.1-pro-preview",
      "gemini-3-flash-preview",
      "gemini-3.5-flash",
      "gemini-3.6-flash",
      "gemini-pro-latest",
      "gemini-flash-latest",
    ];
    let in_list = |m: &str| {
      m.strip_prefix("gemini/")
        .is_some_and(|id| listed.contains(&id))
    };
    let d = pick_default(&[("gemini", "API key")], &in_list).unwrap();
    assert_eq!(d, "gemini/gemini-3.1-pro-preview");
    assert_eq!(
      pick_quick(&d, &in_list),
      ("gemini/gemini-3.5-flash".into(), true)
    );
  }

  #[test]
  fn nothing_configured_is_the_built_in_default() {
    assert_eq!(
      pick(&[("anthropic", "none")]).0,
      crate::config::DEFAULT_MODEL
    );
    assert_eq!(pick(&[]).0, crate::config::DEFAULT_MODEL);
  }

  #[test]
  fn an_explicit_default_gets_its_providers_quick() {
    let (q, table) = pick_quick("deepseek/deepseek-v4-pro", &always);
    assert_eq!((q.as_str(), table), ("deepseek/deepseek-flash", true));
    assert_eq!(pick_quick("ollama/x", &always).0, "ollama/x");
    // Only a provider's own table default gets the table quick.
    for other in ["anthropic/claude-haiku-5-5", "anthropic/claude-opus-5"] {
      assert_eq!(pick_quick(other, &always), (other.to_owned(), false));
    }
    assert_eq!(
      pick_quick("anthropic/claude-opus-5-5", &always).0,
      "anthropic/claude-sonnet-5-5"
    );
  }
}
