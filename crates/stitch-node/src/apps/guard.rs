//! The one check every tool call of an app's thread passes (design
//! §2.22): `Guard::authorize`, before anything of the call runs, under
//! the grant standing at that call; and the money, before every model call
//! (`Guard::admit`) and after it (`Guard::metered`), against the family's
//! ledger (`Ledgers`).
//!
//! Only the tools listed in `authorize` ever run for an app's thread,
//! whatever its tool set says: the tool set decides what the model is
//! offered, the guard what runs.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use serde_json::Value;
use stitch_records::{Attachment, BlobHash, Event, Part, RecordId};

use super::threads::{self, Owner, Per, ThreadNeeds};
use crate::node::Shared;
use crate::place::Place;

/// Directories inside a place an app's thread never writes at once: what
/// they hold steers the user's own agents and tools (a place's skills and
/// settings, other agents' settings, CI, editors, cargo's config).
const STEERING_DIRS: &[&str] = &[
  ".stitch",
  ".claude",
  ".github",
  ".vscode",
  ".cargo",
  ".devcontainer",
  ".git",
  ".husky",
  ".idea",
  ".cursor",
  ".gemini",
  ".codex",
  ".zed",
];
/// Files likewise, at any depth (instructions are read up the tree).
const STEERING_FILES: &[&str] = &[
  "agents.md",
  "claude.md",
  "gemini.md",
  ".mcp.json",
  ".envrc",
  ".cursorrules",
  ".npmrc",
  ".yarnrc",
  ".yarnrc.yml",
  ".pre-commit-config.yaml",
  ".gitmodules",
  ".windsurfrules",
  "rust-toolchain.toml",
  "rust-toolchain",
];

/// Whether writing `file` would steer the user's own agents: such writes
/// go to the user, inside the place too.
pub(crate) fn steers_agents(file: &Path) -> bool {
  let low = |c: &std::ffi::OsStr| c.to_string_lossy().to_ascii_lowercase();
  file
    .components()
    .any(|c| STEERING_DIRS.contains(&low(c.as_os_str()).as_str()))
    || file
      .file_name()
      .is_some_and(|n| STEERING_FILES.contains(&low(n).as_str()))
}

/// The images of a thread's lineage (every record it was compacted into
/// or from): its messages' image attachments, the app's and its tools'.
/// What an app's thread may name in `zoom_image` and `generate_image`, and
/// what its app may read (`/_stitch/threads/<id>/files/<hash>`).
pub(crate) fn lineage_images(s: &Shared, thread: &RecordId) -> Vec<Attachment> {
  let mut out: Vec<Attachment> = Vec::new();
  for t in threads::lineage_of(s, thread) {
    let Ok((root, _)) = s.thread_place(&t) else {
      continue;
    };
    let state = s.state();
    let ctx = crate::chat::ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    for e in crate::chat::thread_events(&ctx, &t).unwrap_or_default() {
      if let Event::Msg(m) = e {
        for p in m.parts {
          if let Part::Attachment { att } = p
            && att.media_type.starts_with("image/")
            && !out.iter().any(|a| a.hash == att.hash)
          {
            out.push(att);
          }
        }
      }
    }
  }
  out
}

/// The image of `images` an ID names: its full hash, or its short ID
/// (`img:` and 12 hex digits) as shown to the model, exactly; never a
/// prefix looked up among the node's blobs.
pub(crate) fn image_named(images: &[Attachment], id: &str) -> Option<BlobHash> {
  let id = id.trim();
  images
    .iter()
    .find(|a| a.hash.to_string() == id || crate::images::short_id(&a.hash) == id)
    .map(|a| a.hash)
}

/// What a write would write, for the user approving it: its content, or
/// the change, cut at 2000 characters each.
fn shown(input: &Value) -> String {
  let cut = |k: &str| {
    let t = input.get(k).and_then(Value::as_str).unwrap_or("");
    let mut out: String = t.chars().take(2000).collect();
    if t.chars().count() > 2000 {
      out.push_str("\n[… cut]");
    }
    out
  };
  if input.get("content").is_some() {
    format!("\n--- content ---\n{}", cut("content"))
  } else {
    format!(
      "\n--- replacing ---\n{}\n--- with ---\n{}",
      cut("old_string"),
      cut("new_string")
    )
  }
}

/// What the guard says of a call.
#[derive(Debug, PartialEq)]
pub(crate) enum Verdict {
  /// It runs as asked.
  Run,
  /// It runs with this input (image IDs made full hashes).
  RunWith(Value),
  /// It is the user's to approve: this command, the app named.
  Ask(String),
  Refuse(String),
}

/// What a turn of an app's thread spent and may still spend.
struct Meter {
  started: Timestamp,
  steps: u64,
  /// Spent this turn, as metered.
  turn_usd: f64,
  /// The last step's cost: what the next one reserves.
  last_cost: f64,
  /// What the thread's lineage spent before this turn, once read.
  before: Option<f64>,
  /// When its lineage started, once read.
  born: Option<Timestamp>,
  /// A step whose price was not known: the turn stops.
  unknown: Option<String>,
}

/// An app's thread's guard, for a turn.
pub(crate) struct Guard {
  s: Weak<Shared>,
  pub(crate) owner: Owner,
  pub(crate) thread: RecordId,
  place: Place,
  /// Where it reads and writes (its place, or a global thread's own
  /// folder), resolved.
  roots: Vec<PathBuf>,
  /// Where it may read besides: its granted skills' directories.
  reads: Vec<PathBuf>,
  cwd: PathBuf,
  aliases: crate::place::Aliases,
  /// For tests: the grant, instead of reading it.
  fixed: Option<Option<ThreadNeeds>>,
  /// The node the thread started on, when it is not this one: an app's
  /// thread runs only there, where its family's ledger is (design §2.22,
  /// "One node runs a family").
  elsewhere: Option<String>,
  meter: Mutex<Meter>,
  /// This guard's own key in the ledger's reservations: a turn's guard
  /// dropped late never releases the next turn's.
  key: u64,
}

/// Guards made, for their keys.
static GUARDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Guard {
  #[allow(clippy::too_many_arguments)]
  pub(crate) fn new(
    s: &Arc<Shared>,
    owner: Owner,
    thread: RecordId,
    place: Place,
    roots: Vec<PathBuf>,
    reads: Vec<PathBuf>,
    cwd: PathBuf,
    aliases: crate::place::Aliases,
  ) -> Guard {
    let started_on = s
      .state()
      .index
      .lock()
      .expect("index lock")
      .record(&thread)
      .ok()
      .flatten()
      .map(|r| r.node.to_string());
    let elsewhere = started_on.filter(|n| n != s.state().node.as_str());
    Guard {
      key: GUARDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
      elsewhere,
      s: Arc::downgrade(s),
      owner,
      thread,
      place,
      roots: roots
        .into_iter()
        .map(|r| std::fs::canonicalize(&r).unwrap_or(r))
        .collect(),
      reads: reads
        .into_iter()
        .map(|r| std::fs::canonicalize(&r).unwrap_or(r))
        .collect(),
      cwd,
      aliases,
      fixed: None,
      meter: Mutex::new(Meter {
        started: Timestamp::now(),
        steps: 0,
        turn_usd: 0.0,
        last_cost: 0.0,
        before: None,
        born: None,
        unknown: None,
      }),
    }
  }

  /// The node, while it runs.
  pub(crate) fn shared(&self) -> Option<Arc<Shared>> {
    self.s.upgrade()
  }

  /// The grant standing now for this thread (`None`: revoked, denied, or
  /// its place no longer granted).
  pub(crate) fn needs(&self) -> Option<ThreadNeeds> {
    if self.elsewhere.is_some() {
      return None;
    }
    if let Some(f) = &self.fixed {
      return f.clone();
    }
    let s = self.s.upgrade()?;
    threads::needs_now(&s, &self.owner, Some(&self.place))
  }

  fn revoked(&self) -> String {
    if let Some(n) = &self.elsewhere {
      return format!(
        "app \"{}\"'s threads run only on {n}, where its family's money is kept: nothing runs \
         here",
        self.owner.name
      );
    }
    format!(
      "app \"{}\" may no longer run threads (the user revoked or changed its grant): nothing runs",
      self.owner.name
    )
  }

  fn refused(&self, what: &str) -> Verdict {
    Verdict::Refuse(format!(
      "not in app \"{}\"'s grant: {what} is refused, never asked. Do without it, or say in your \
       reply what the user would need to grant.",
      self.owner.name
    ))
  }

  fn inside(&self, file: &Path) -> bool {
    self.roots.iter().any(|r| file.starts_with(r))
  }

  /// The one check: whether `name` may run with `input` for this thread,
  /// before anything of it runs (nothing read, nothing dispatched).
  pub(crate) fn authorize(
    &self,
    name: &str,
    input: &Value,
    otter: &crate::otter::OtterTools,
    mcp: &crate::mcp::McpTools,
  ) -> Verdict {
    let Some(n) = self.needs() else {
      return Verdict::Refuse(self.revoked());
    };
    let (tools, skills) = ThreadNeeds::entries(Some(&n));
    let sel = crate::toolset::Selection {
      tools,
      skills,
      ..Default::default()
    };
    if let Some(group) = crate::toolset::group_of(name) {
      if !sel.group_on(group) {
        return self.refused(&format!("{name} (the {group} tools)"));
      }
      return match name {
        "view_file" => {
          let path = input.get("path").and_then(Value::as_str).unwrap_or("");
          let file = self
            .aliases
            .map(path)
            .unwrap_or_else(|| crate::viewer::resolve(path, &self.cwd));
          match std::fs::canonicalize(&file) {
            Ok(real) if self.inside(&real) || self.reads.iter().any(|r| real.starts_with(r)) => {
              Verdict::Run
            }
            _ => Verdict::Refuse(format!(
              "{}: outside this thread's place; an app's thread reads files only there",
              file.display()
            )),
          }
        }
        "write_file" | "edit_file" => {
          let path = input.get("path").and_then(Value::as_str).unwrap_or("");
          let file = crate::edits::target(&self.cwd, path, &self.aliases);
          if file
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
          {
            return Verdict::Refuse("files inside .git are git's, not to be written".into());
          }
          if !self.inside(&file) || steers_agents(&file) {
            Verdict::Ask(format!("{name} {}{}", file.display(), shown(input)))
          } else {
            Verdict::Run
          }
        }
        "zoom_image" => self.images(input, &["image"]),
        "generate_image" => {
          let model = input
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
              let s = self.s.upgrade()?;
              crate::imagegen::model(&s.config, &s.data)
            })
            .unwrap_or_default();
          if crate::pricing::image_price(model.rsplit('/').next().unwrap_or("")).is_none() {
            return Verdict::Refuse(format!(
              "generate_image with {model}: its price is not known here, and an app's thread \
               spends only what can be counted"
            ));
          }
          self.images(input, &["images"])
        }
        "search_web" | "browse_web" => Verdict::Run,
        // Granted with its group, priced (the grant's money counts it).
        "decide" => Verdict::Run,
        "web_eval" => Verdict::Refuse(
          "web_eval is off for an app's thread: its browser cannot be kept to public \
           addresses; read pages with browse_web"
            .into(),
        ),
        _ => self.refused(name),
      };
    }
    if name == "read_skill" {
      let wanted = input.get("name").and_then(Value::as_str).unwrap_or("");
      return if sel.skill_on(wanted) {
        Verdict::Run
      } else {
        self.refused(&format!("the skill {wanted}"))
      };
    }
    if name == "app_command" || name == "app_shell" {
      // Its container grants (design §2.23), standing at this call, and a
      // data folder for the files.
      let Some(s) = self.s.upgrade() else {
        return Verdict::Refuse("the node is stopping".into());
      };
      let kind = if name == "app_command" {
        super::containers::Kind::Run
      } else {
        super::containers::Kind::Shell
      };
      let Some(g) = super::containers::granted(&s, &self.owner, kind) else {
        return self.refused(&format!("{name} (the {} grant)", kind.word()));
      };
      if n.data.as_ref().and_then(|d| d.folder.as_ref()).is_none() {
        return self.refused(&format!("{name} (no data folder)"));
      }
      if name == "app_command" {
        let c = input.get("command").and_then(Value::as_str).unwrap_or("");
        if !g.spec.commands.contains_key(c) {
          return self.refused(&format!("the command {c}"));
        }
      }
      return Verdict::Run;
    }
    if otter.has(name) {
      if crate::otter::language::NAMES.contains(&name) {
        return self.refused(&format!("{name} (code of its own)"));
      }
      return match otter.effects_of(name) {
        None => self.refused(name),
        Some((_, true)) => self.refused(&format!("{name} (edge talk's conversations)")),
        Some((effects, false)) => {
          if effects == otter_core::signature::Effects::NONE
            || n.otter.iter().any(|p| crate::mcp::glob(p, name))
          {
            Verdict::Run
          } else {
            self.refused(&format!("the Otter tool {name}"))
          }
        }
      };
    }
    if mcp.has(name) {
      if !mcp.is_here(name) {
        return self.refused(&format!("{name} (an MCP server of another node)"));
      }
      return if n.mcp_allow.iter().any(|p| crate::mcp::glob(p, name)) {
        Verdict::Run
      } else {
        self.refused(&format!("the MCP call {name}"))
      };
    }
    self.refused(name)
  }

  /// Image IDs in `keys` of `input`: each an image of this thread's
  /// lineage, made its full hash.
  fn images(&self, input: &Value, keys: &[&str]) -> Verdict {
    let Some(s) = self.s.upgrade() else {
      return Verdict::Refuse("the node is stopping".into());
    };
    let own = lineage_images(&s, &self.thread);
    let mut input = input.clone();
    for k in keys {
      let ids: Vec<&mut Value> = match input.get_mut(*k) {
        Some(Value::String(_)) => vec![input.get_mut(*k).expect("there")],
        Some(Value::Array(a)) => a.iter_mut().collect(),
        _ => Vec::new(),
      };
      for v in ids {
        let id = v.as_str().unwrap_or("").to_owned();
        match image_named(&own, &id) {
          Some(h) => *v = Value::String(h.to_string()),
          None => {
            return Verdict::Refuse(format!(
              "no image {id} in this thread: an app's thread works on its own images only"
            ));
          }
        }
      }
    }
    Verdict::RunWith(input)
  }

  /// Before a model call: why the turn must stop instead, if it must (the
  /// grant revoked, the thread's or the family's money spent, its time
  /// up); else a step's likely cost is reserved in the family's ledger.
  pub(crate) async fn admit(&self) -> Option<String> {
    let n = match self.needs() {
      Some(n) => n,
      None => return Some(self.revoked()),
    };
    let s = self.s.upgrade()?;
    // What the lineage spent before this turn, and when it started: read
    // once a turn.
    let (before, born) = {
      let m = self.meter.lock().expect("meter lock");
      (m.before, m.born)
    };
    if before.is_none() || born.is_none() {
      let (s2, t) = (s.clone(), self.thread.clone());
      let (spent, start) =
        tokio::task::spawn_blocking(move || threads::lineage_spent(&s2, &t, None))
          .await
          .unwrap_or((0.0, None));
      let mut m = self.meter.lock().expect("meter lock");
      m.before.get_or_insert(spent);
      m.born = m.born.or(start).or(Some(m.started));
    }
    let name = &self.owner.name;
    let (spent, per_turn, est) = {
      let mut m = self.meter.lock().expect("meter lock");
      if let Some(u) = &m.unknown {
        return Some(u.clone());
      }
      m.steps += 1;
      if let Some(max) = n.budget.steps
        && m.steps > max
      {
        return Some(format!(
          "took the {max} model calls a turn app \"{name}\"'s grant gives"
        ));
      }
      let per_turn = n.budget.per == Per::Turn;
      if let Some(min) = n.budget.minutes {
        let from = if per_turn {
          m.started
        } else {
          m.born.unwrap_or(m.started)
        };
        if Timestamp::now() >= from + Duration::from_secs(min * 60) {
          return Some(format!(
            "past the {min} minutes app \"{name}\"'s grant gives {}",
            if per_turn { "a turn" } else { "a thread" }
          ));
        }
      }
      let spent = if per_turn {
        m.turn_usd
      } else {
        m.before.unwrap_or(0.0) + m.turn_usd
      };
      (spent, per_turn, m.last_cost.max(0.01))
    };
    let cap = n.budget.usd.unwrap_or(threads::DEFAULT_USD);
    if spent >= cap {
      return Some(format!(
        "spent ${spent:.2} of the ${cap} app \"{name}\"'s grant gives {}",
        if per_turn { "a turn" } else { "a thread" }
      ));
    }
    let daily = n.daily_usd.unwrap_or(threads::DEFAULT_DAILY_USD);
    s.apps
      .family
      .ledgers
      .reserve(&s, &self.owner, self.key, est, daily)
      .await
      .err()
  }

  /// What a step or a tool spent: priced (at list prices when the
  /// provider gave none), kept in the turn's meter and the family's
  /// ledger; a cost that cannot be priced stops the turn at its next step.
  pub(crate) fn metered(&self, model: &str, usage: &stitch_providers::Usage, usd: Option<f64>) {
    let Some(s) = self.s.upgrade() else {
      return;
    };
    let cost = crate::pricing::priced(model, usage, usd);
    {
      let mut m = self.meter.lock().expect("meter lock");
      match cost {
        Some(c) => {
          m.turn_usd += c;
          m.last_cost = c;
        }
        None => {
          m.unknown = Some(format!(
            "the price of {model} is not known here, and an app's thread spends only what can \
             be counted: give the app a model with a known price"
          ));
        }
      }
    }
    s.apps
      .family
      .ledgers
      .spend(&self.owner, self.key, cost.unwrap_or(0.0));
  }
}

impl Drop for Guard {
  fn drop(&mut self) {
    if let Some(s) = self.s.upgrade() {
      s.apps.family.ledgers.release(&self.owner, self.key);
    }
  }
}

/// A family's money and admissions today, on the node that runs it.
struct Ledger {
  day: jiff::civil::Date,
  /// Spent today: read from the family's usage once, then metered.
  spent: f64,
  /// Reserved by steps admitted and not metered yet, by guard.
  reserved: HashMap<u64, f64>,
  /// Image bytes the app sent today.
  uploads: u64,
}

/// How long a claimed slot counts as at work, at most, while its turn
/// starts.
const CLAIM: Duration = Duration::from_secs(60);

/// Every family's ledger, and the slots claimed by admissions whose
/// turns are not listed as running yet.
#[derive(Default)]
pub(crate) struct Ledgers {
  by: Mutex<HashMap<Owner, Ledger>>,
  claims: Mutex<HashMap<Owner, Vec<(RecordId, Instant)>>>,
}

fn today() -> jiff::civil::Date {
  jiff::Zoned::now().date()
}

impl Ledgers {
  /// The family's ledger is read for today (from its usage events, off
  /// the async threads), once.
  async fn seeded(&self, s: &Arc<Shared>, owner: &Owner) {
    let fresh = self
      .by
      .lock()
      .expect("ledger lock")
      .get(owner)
      .is_some_and(|l| l.day == today());
    if fresh {
      return;
    }
    let (s2, o) = (s.clone(), owner.clone());
    let spent = tokio::task::spawn_blocking(move || threads::spent_today(&s2, &o))
      .await
      .unwrap_or(0.0);
    let mut by = self.by.lock().expect("ledger lock");
    let l = by.entry(owner.clone()).or_insert_with(|| Ledger {
      day: today(),
      spent,
      reserved: HashMap::new(),
      uploads: 0,
    });
    if l.day != today() {
      *l = Ledger {
        day: today(),
        spent,
        reserved: HashMap::new(),
        uploads: 0,
      };
    }
  }

  /// What the family spent today.
  pub(crate) async fn spent(&self, s: &Arc<Shared>, owner: &Owner) -> f64 {
    self.seeded(s, owner).await;
    self
      .by
      .lock()
      .expect("ledger lock")
      .get(owner)
      .map_or(0.0, |l| l.spent)
  }

  /// Reserves `est` for a step of `thread`, unless the day's spend and
  /// what the family's other turns reserved would pass `daily`.
  async fn reserve(
    &self,
    s: &Arc<Shared>,
    owner: &Owner,
    key: u64,
    est: f64,
    daily: f64,
  ) -> Result<(), String> {
    self.seeded(s, owner).await;
    let mut by = self.by.lock().expect("ledger lock");
    let Some(l) = by.get_mut(owner) else {
      return Err("the app's ledger is not here".into());
    };
    let others: f64 = l
      .reserved
      .iter()
      .filter(|(k, _)| **k != key)
      .map(|(_, u)| u)
      .sum();
    if l.spent + others + est > daily {
      return Err(format!(
        "app \"{}\"'s threads spent ${:.2} of the ${daily} a day its grant gives{}",
        owner.name,
        l.spent,
        if others > 0.0 {
          format!(" (${others:.2} more held by its other turns at work)")
        } else {
          String::new()
        }
      ));
    }
    l.reserved.insert(key, est);
    Ok(())
  }

  fn spend(&self, owner: &Owner, key: u64, usd: f64) {
    if let Some(l) = self.by.lock().expect("ledger lock").get_mut(owner) {
      l.spent += usd;
      l.reserved.remove(&key);
    }
  }

  fn release(&self, owner: &Owner, key: u64) {
    if let Some(l) = self.by.lock().expect("ledger lock").get_mut(owner) {
      l.reserved.remove(&key);
    }
  }

  /// Counts what was spent for the family outside a turn's steps (a
  /// compaction's summary).
  pub(crate) async fn charge(&self, s: &Arc<Shared>, owner: &Owner, usd: f64) {
    self.seeded(s, owner).await;
    if let Some(l) = self.by.lock().expect("ledger lock").get_mut(owner) {
      l.spent += usd;
    }
  }

  /// Whether the family has spent its day (`daily`), for admissions.
  pub(crate) async fn spent_day(&self, s: &Arc<Shared>, owner: &Owner, daily: f64) -> Option<f64> {
    let spent = self.spent(s, owner).await;
    (spent >= daily).then_some(spent)
  }

  /// Counts `bytes` of images against the family's daily quota.
  pub(crate) fn upload(&self, owner: &Owner, bytes: u64, quota: u64) -> Result<(), String> {
    let mut by = self.by.lock().expect("ledger lock");
    let l = by.entry(owner.clone()).or_insert_with(|| Ledger {
      day: today(),
      spent: 0.0,
      reserved: HashMap::new(),
      uploads: 0,
    });
    if l.uploads + bytes > quota {
      return Err(format!(
        "this app sent {} MiB of images today, its quota {} MiB: tomorrow",
        l.uploads >> 20,
        quota >> 20
      ));
    }
    l.uploads += bytes;
    Ok(())
  }

  /// Under the family's lock: whether one more thread may start work
  /// (`at_work` counts those running here), claiming a slot for `thread`
  /// until its turn is listed as running (`unclaim`).
  pub(crate) fn claim(
    &self,
    owner: &Owner,
    thread: &RecordId,
    max: usize,
    at_work: impl Fn() -> HashSet<RecordId>,
  ) -> Result<(), String> {
    let mut claims = self.claims.lock().expect("claims lock");
    let list = claims.entry(owner.clone()).or_default();
    list.retain(|(_, at)| at.elapsed() < CLAIM);
    let mut busy = at_work();
    busy.extend(list.iter().map(|(t, _)| t.clone()));
    if busy.contains(thread) {
      return Ok(());
    }
    if busy.len() >= max {
      return Err(format!(
        "{max} of this app's threads are at work already: wait for one to finish"
      ));
    }
    list.push((thread.clone(), Instant::now()));
    Ok(())
  }

  pub(crate) fn unclaim(&self, owner: &Owner, thread: &RecordId) {
    if let Some(list) = self.claims.lock().expect("claims lock").get_mut(owner) {
      list.retain(|(t, _)| t != thread);
    }
  }

  /// Forgets families with nothing kept for today (a day passed).
  pub(crate) fn prune(&self) {
    let day = today();
    self
      .by
      .lock()
      .expect("ledger lock")
      .retain(|_, l| l.day == day || !l.reserved.is_empty());
    self
      .claims
      .lock()
      .expect("claims lock")
      .retain(|_, l| l.iter().any(|(_, at)| at.elapsed() < CLAIM));
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn writes_that_steer_agents_are_told_apart() {
    for p in [
      "/p/.stitch/skills/x/tools/a.otr",
      "/p/AGENTS.md",
      "/p/sub/CLAUDE.md",
      "/p/.claude/settings.json",
      "/p/.github/workflows/ci.yml",
      "/p/.vscode/tasks.json",
      "/p/.cargo/config.toml",
      "/p/.envrc",
      "/p/.mcp.json",
      "/p/.Stitch/settings.toml",
      "/p/Agents.md",
    ] {
      assert!(steers_agents(Path::new(p)), "{p}");
    }
    for p in ["/p/src/main.rs", "/p/notes/agents-notes.md", "/p/design.md"] {
      assert!(!steers_agents(Path::new(p)), "{p}");
    }
  }

  #[test]
  fn images_are_named_exactly_never_by_prefix() {
    let a = Attachment {
      hash: BlobHash::of(b"a"),
      media_type: "image/png".into(),
      name: None,
      bytes: 1,
    };
    let own = vec![a.clone()];
    assert_eq!(image_named(&own, &a.hash.to_string()), Some(a.hash));
    assert_eq!(
      image_named(&own, &crate::images::short_id(&a.hash)),
      Some(a.hash)
    );
    let short = crate::images::short_id(&a.hash);
    assert_eq!(image_named(&own, &short[..12]), None, "a prefix");
    let other = BlobHash::of(b"b");
    assert_eq!(image_named(&own, &other.to_string()), None);
    assert_eq!(image_named(&own, &crate::images::short_id(&other)), None);
  }

  fn guard(dir: &Path, needs: Value) -> Guard {
    let t: RecordId = "2026-10-27T100000Z-mbp-abcd".parse().unwrap();
    Guard {
      s: Weak::new(),
      owner: Owner {
        thread: t.clone(),
        name: "Atelier".into(),
      },
      thread: t,
      place: Place::Global,
      roots: vec![std::fs::canonicalize(dir).unwrap()],
      reads: Vec::new(),
      cwd: dir.to_owned(),
      aliases: Default::default(),
      elsewhere: None,
      key: 0,
      fixed: Some(
        serde_json::from_value::<super::super::threads::Needs>(needs)
          .unwrap()
          .checked()
          .unwrap()
          .threads,
      ),
      meter: Mutex::new(Meter {
        started: Timestamp::now(),
        steps: 0,
        turn_usd: 0.0,
        last_cost: 0.0,
        before: None,
        born: None,
        unknown: None,
      }),
    }
  }

  #[test]
  fn every_call_is_authorized_by_the_grant_standing() {
    let d = tempfile::tempdir().unwrap();
    let dir = std::fs::canonicalize(d.path()).unwrap();
    std::fs::write(dir.join("a.md"), "x").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), "s").unwrap();
    let g = guard(
      &dir,
      json!({"threads": {"tools": ["files", "viewer", "web"], "otter": ["render"]}}),
    );
    let (otter, mcp) = (
      crate::otter::OtterTools::default(),
      crate::mcp::McpTools::default(),
    );
    let call = |name: &str, input: Value| g.authorize(name, &input, &otter, &mcp);
    assert_eq!(call("view_file", json!({"path": "a.md"})), Verdict::Run);
    assert!(matches!(
      call(
        "view_file",
        json!({"path": outside.path().join("secret").display().to_string()})
      ),
      Verdict::Refuse(_)
    ));
    assert_eq!(
      call("write_file", json!({"path": "b.md", "content": "x"})),
      Verdict::Run
    );
    // Out of the place by a `..` after a missing directory: theirs.
    assert!(matches!(
      call(
        "write_file",
        json!({"path": "n/../../x.md", "content": "x"})
      ),
      Verdict::Ask(_)
    ));
    // Inside the place, but it would steer the user's agents: theirs.
    for p in [".stitch/skills/x/tools/t.otr", "AGENTS.md", "sub/CLAUDE.md"] {
      assert!(
        matches!(
          call("write_file", json!({"path": p, "content": "x"})),
          Verdict::Ask(_)
        ),
        "{p}"
      );
    }
    // Outside: theirs too; reading it to edit is not done first.
    assert!(matches!(
      call(
        "edit_file",
        json!({"path": outside.path().join("secret").display().to_string(), "old_string": "s", "new_string": "t"})
      ),
      Verdict::Ask(_)
    ));
    assert!(matches!(
      call(
        "write_file",
        json!({"path": ".git/hooks/pre-commit", "content": "x"})
      ),
      Verdict::Refuse(_)
    ));
    // Not granted, or never for an app's thread.
    for (name, input) in [
      (
        "web_eval",
        json!({"url": "https://example.com", "code": "1"}),
      ),
      ("shell_run", json!({"command": "ls"})),
      ("spawn", json!({"goal": "x"})),
      ("recall", json!({"query": "x"})),
      ("show_app", json!({})),
      ("browser_tabs", json!({})),
      ("commit_push", json!({})),
      ("zoom_image", json!({"image": "img:000000000000"})),
      ("otter_run", json!({"code": "1"})),
      ("github__create_issue", json!({})),
    ] {
      assert!(matches!(call(name, input), Verdict::Refuse(_)), "{name}");
    }
    // Revoked: nothing.
    let mut revoked = guard(&dir, json!({"threads": {"tools": ["viewer"]}}));
    revoked.fixed = Some(None);
    assert!(matches!(
      revoked.authorize("view_file", &json!({"path": "a.md"}), &otter, &mcp),
      Verdict::Refuse(w) if w.contains("no longer")
    ));
  }
}
