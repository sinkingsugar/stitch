//! Apps over the node's live state (design §2.10): what the node knows of
//! its threads, their jobs and the questions waiting on the user, read by a
//! page as it is now (`stitch.live.*` in the bridge, GETs under
//! `/_stitch/live/`), and its changes heard through the page's long poll.
//!
//! - **Read-only, and no model:** every read comes from the index, the
//!   node's memory, and the transcripts (each read once per step).
//! - **Consent per app,** off by default: the user's whole working state
//!   is not an agent-made page's to read. Kept on the pin (`live` on its
//!   `app` record) or, for an app in its conversation, on its thread (a
//!   `set` of `live:<name>`); given by the phone's own sheet or `stitch app
//!   allow-live`, never by the page.
//! - **Changes** are told by marks the node takes every second while a
//!   page with consent listens: the index's records and segments with the
//!   loops running (threads), the questions open (attention), the jobs
//!   (jobs). A kind whose mark moved gets a new generation, and the pages
//!   waiting hear of it.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use jiff::Timestamp;
use serde_json::{Value, json};
use stitch_records::{Author, RecordId, kinds};

use super::{Manifest, Pinned};
use crate::node::Shared;
use crate::place::Place;
use crate::{Error, Result};

/// What may be listened to.
pub(crate) const KINDS: [&str; 5] = ["threads", "jobs", "attention", "schedules", "events"];
/// A page's wait for changes, and how often the node looks for them.
const TICK: Duration = Duration::from_secs(1);
/// The watch goes on this long after the last page stopped listening.
const LINGER: u32 = 30;
/// Threads a read gives at most, by default and in all.
const THREADS: usize = 100;
const THREADS_MAX: usize = 500;

/// The most `conversations` lists in one call: the whole forest, for an
/// app that draws it (design §2.25).
const THREADS_MAX_FOREST: usize = 5000;
/// A thread's last words, at most.
const LAST_WORDS: usize = 300;

/// The node's live state for apps: the generations of each kind, and the
/// watch that moves them.
#[derive(Default)]
pub(crate) struct Live {
  pub(crate) rows: crate::weave::RowCache,
  /// What the WebSocket pushes in place of `changed` (`feed`).
  pub(crate) feed: super::feed::Feed,
  /// Bumped with any kind's.
  generation: AtomicU64,
  gens: Mutex<BTreeMap<&'static str, u64>>,
  marks: Mutex<Option<[u64; 5]>>,
  changed: tokio::sync::Notify,
  listeners: AtomicUsize,
  watching: AtomicBool,
}

/// A page listening: the watch runs while one is.
pub(crate) struct Listening(Arc<Shared>);

impl Drop for Listening {
  fn drop(&mut self) {
    self.0.apps.live.listeners.fetch_sub(1, Ordering::SeqCst);
  }
}

impl Live {
  pub(crate) fn generation(&self) -> u64 {
    self.generation.load(Ordering::SeqCst)
  }

  /// Each kind's generation, and the whole.
  pub(crate) fn said(&self) -> Value {
    let gens = self.gens.lock().expect("gens lock");
    let mut out = serde_json::Map::new();
    out.insert("generation".into(), json!(self.generation()));
    for k in KINDS {
      out.insert(k.into(), json!(gens.get(k).copied().unwrap_or(0)));
    }
    Value::Object(out)
  }

  /// Waits until the generation is past `seen`, or `limit` passes.
  pub(crate) async fn wait(&self, seen: u64, limit: Duration) {
    let changed = self.changed.notified();
    if self.generation() != seen {
      return;
    }
    let _ = tokio::time::timeout(limit, changed).await;
  }

  /// A page listens: the watch starts, if it is not running.
  pub(crate) fn listen(s: &Arc<Shared>) -> Listening {
    s.apps.live.listeners.fetch_add(1, Ordering::SeqCst);
    watch(s);
    Listening(s.clone())
  }

  pub(crate) fn bump(&self, kinds: &[&'static str]) {
    {
      let mut gens = self.gens.lock().expect("gens lock");
      for k in kinds {
        *gens.entry(k).or_default() += 1;
      }
    }
    self.generation.fetch_add(1, Ordering::SeqCst);
    self.changed.notify_waiters();
  }
}

/// Starts the watch unless it runs: every second, while a page listens
/// (and a little after), the marks of each kind are taken, and a kind
/// whose mark moved is told.
fn watch(s: &Arc<Shared>) {
  if s.apps.live.watching.swap(true, Ordering::SeqCst) {
    return;
  }
  let weak: Weak<Shared> = Arc::downgrade(s);
  tokio::spawn(async move {
    let mut idle = 0;
    loop {
      let Some(s) = weak.upgrade() else { return };
      let now = marks(&s).await;
      let live = &s.apps.live;
      let moved: Vec<&'static str> = {
        let mut last = live.marks.lock().expect("marks lock");
        let moved = match *last {
          Some(before) => KINDS
            .iter()
            .zip(before.iter().zip(now.iter()))
            .filter(|(_, (a, b))| a != b)
            .map(|(k, _)| *k)
            .collect(),
          None => Vec::new(),
        };
        *last = Some(now);
        moved
      };
      if !moved.is_empty() {
        live.bump(&moved);
      }
      super::feed::tick(&s, &moved).await;
      if live.listeners.load(Ordering::SeqCst) == 0 {
        idle += 1;
        if idle >= LINGER {
          live.watching.store(false, Ordering::SeqCst);
          // Taken up again if one came meanwhile.
          if live.listeners.load(Ordering::SeqCst) == 0
            || live.watching.swap(true, Ordering::SeqCst)
          {
            *live.marks.lock().expect("marks lock") = None;
            return;
          }
        }
      } else {
        idle = 0;
      }
      drop(s);
      tokio::time::sleep(TICK).await;
    }
  });
}

fn hash_of(v: impl Hash) -> u64 {
  let mut h = std::collections::hash_map::DefaultHasher::new();
  v.hash(&mut h);
  h.finish()
}

/// The marks of threads, jobs, attention, schedules and events now.
async fn marks(s: &Arc<Shared>) -> [u64; 5] {
  // The index reads run on the blocking pool, one at a time (the watch is
  // one task), so a slow index (a swapping machine) never holds a runtime
  // worker.
  let s2 = s.clone();
  let (index_mark, attention, schedules, needs_mark) = tokio::task::spawn_blocking(move || {
    let s = s2;
    let index = s.state().index.lock().expect("index lock");
    let mark = index
      .mark(&[
        kinds::THREAD,
        kinds::REPORT,
        kinds::QUESTION,
        kinds::ANSWER,
        kinds::ASSIGNMENT,
        kinds::SET,
      ])
      .unwrap_or_default();
    let open: Vec<String> = index
      .attention()
      .unwrap_or_default()
      .into_iter()
      .filter_map(|a| match a {
        stitch_store::AttentionItem::Question { id, .. } => Some(id.to_string()),
        _ => None,
      })
      .collect();
    // Attention and schedules do not list transcript segments (each model
    // step writes one): only the threads' mark carries them.
    let schedules = index
      .mark_records(&[kinds::SCHEDULE, kinds::SET])
      .unwrap_or_default();
    let needs_mark = index
      .mark_records(&[kinds::REPORT, kinds::SET])
      .unwrap_or_default();
    (mark, open, schedules, needs_mark)
  })
  .await
  .unwrap_or_default();
  let mut running: Vec<String> = s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .keys()
    .cloned()
    .collect();
  running.sort();
  // A call starting in a running step moves it too (its step is not
  // written till its calls return).
  let calls: Vec<Option<String>> = running
    .iter()
    .map(|t| running_call(s, t).map(|c| c.id))
    .collect();
  let mut starting: Vec<String> = s
    .detached
    .starting
    .lock()
    .expect("starting lock")
    .keys()
    .cloned()
    .collect();
  starting.sort();
  let jobs: Vec<String> = s.shells.live().await.into_iter().map(|(k, _)| k).collect();
  [
    hash_of((index_mark, running, calls, starting)),
    hash_of(jobs),
    // A dismissal (a `set`) and a report moves what needs the user too.
    hash_of((attention, needs_mark)),
    hash_of((schedules, crate::schedules::mark(s))),
    // Events and their asks move it (and bump it at once as they come).
    s.events.generation(),
  ]
}

/// The call a loop here runs now on `thread` (`chat::step_in_flight`).
pub(crate) fn running_call(s: &Shared, thread: &str) -> Option<crate::chat::CallInFlight> {
  let id: RecordId = thread.parse().ok()?;
  crate::chat::step_in_flight(&s.state().index, &id, Some(std::process::id()))?
    .running()
    .cloned()
}

/// A query's value for `key`.
fn param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
  query
    .split('&')
    .filter_map(|kv| kv.split_once('=').or(Some((kv, ""))))
    .find(|(k, _)| *k == key)
    .map(|(_, v)| v)
}

fn flag(query: &str, key: &str) -> bool {
  matches!(param(query, key), Some("" | "1" | "true" | "yes"))
}

/// The call's `limit`: 100 by default, 500 at most, 5000 with `conversations`.
fn limit_of(query: &str) -> usize {
  let most = if flag(query, "conversations") {
    THREADS_MAX_FOREST
  } else {
    THREADS_MAX
  };
  param(query, "limit")
    .and_then(|v| v.parse::<usize>().ok())
    .unwrap_or(THREADS)
    .clamp(1, most)
}

/// `text` cut to `n` characters, with an ellipsis.
fn clip(text: &str, n: usize) -> String {
  match text.char_indices().nth(n) {
    Some((i, _)) => format!("{}…", &text[..i]),
    None => text.to_owned(),
  }
}

fn stamp(t: Option<Timestamp>) -> Value {
  t.map_or(Value::Null, |t| json!(t.to_string()))
}

fn title_of(s: &Shared, thread: &str) -> Option<String> {
  let id: RecordId = thread.parse().ok()?;
  s.state()
    .index
    .lock()
    .expect("index lock")
    .title(&id)
    .ok()
    .flatten()
}

/// `stitch.live.threads({all, status, since, limit, id})`: the threads the
/// app's thread spawned or sent to, or with `all` every detached thread
/// this node knows of, on any node; `status` one of `working`, `blocked`,
/// `idle`, `ended`, `cancelled`; `since` an instant (RFC 3339): those that
/// stepped (or started) since. With `conversations` (an app that reads the
/// whole forest: `{all: true, conversations: true, limit}`): the user's own
/// list, every conversation and thread with compaction chains folded to
/// their newest (deleted ones left out), `limit` up to 5000, and rows
/// without `in_flight` and `app` (the costly per-row reads an overview does
/// not draw).
pub(crate) fn threads(
  s: &Arc<Shared>,
  thread: Option<&RecordId>,
  query: &str,
) -> std::result::Result<Value, String> {
  let all = flag(query, "all");
  let status = param(query, "status").filter(|w| !w.is_empty());
  if let Some(w) = status
    && !["working", "blocked", "idle", "ended", "cancelled"].contains(&w)
  {
    return Err(format!(
      "status `{w}`: working, blocked, idle, ended or cancelled"
    ));
  }
  let since = match param(query, "since").filter(|v| !v.is_empty()) {
    Some(v) => {
      let v = super::serve::percent_decode(v);
      Some(
        v.parse::<Timestamp>()
          .map_err(|e| format!("since `{v}`: {e} (an instant, as RFC 3339)"))?,
      )
    }
    None => None,
  };
  let forest = flag(query, "conversations");
  let limit = limit_of(query);
  // `every` is the client's own flag; `conversations` lists the same set.
  let every = flag(query, "every") || forest;
  let mut ids = if every {
    Vec::new()
  } else {
    crate::weave::listed(s, thread, all)
  };
  let mut placed: std::collections::HashMap<RecordId, Place> = std::collections::HashMap::new();
  if every {
    // The user's own client: every conversation and thread, the latest
    // activity first (deleted ones left out), a compaction chain as one
    // row, its newest thread (design §2.15, "One conversation").
    // Each one's place comes from the same rows (one pass, `places()` read
    // once), for the client's sections below.
    let state = s.state();
    let home = state.home().map(|h| h.repo().clone());
    let known = s.places();
    let index = state.index.lock().expect("index lock");
    ids.clear();
    // `ids`: these rows alone (the deltas the WebSocket pushes, `feed`),
    // without reading every thread: those deleted, compacted into another
    // or not threads are left out, so a caller knows them gone.
    let listed = match param(query, "ids").filter(|v| !v.is_empty()) {
      Some(only) => only
        .split(',')
        .filter_map(|t| t.parse::<RecordId>().ok())
        .filter_map(|t| index.record(&t).ok().flatten())
        .filter(|r| r.kind == kinds::THREAD && index.compacted_into(&r.id).is_none())
        .map(|r| {
          let at = r.created;
          (r, at)
        })
        .collect(),
      None => index.listed_threads().unwrap_or_default(),
    };
    for (r, _) in listed {
      if index.is_deleted(&r.id).unwrap_or(false) {
        continue;
      }
      // As `Node::thread_place`: a thread whose repository is not held
      // here has no place to name (a row says `?`).
      if state.root(&r.repo).is_some() {
        placed.insert(r.id.clone(), Place::of_thread(&r, home.as_ref(), &known));
      }
      ids.push(r.id);
    }
  } else if all {
    // Conversations too, while a turn runs on them here or one waits on
    // the user: `listed` gives only spawned threads, and "what's going on"
    // without the chat at work is wrong.
    let mut extra: Vec<RecordId> = s
      .detached
      .steers
      .lock()
      .expect("steers lock")
      .keys()
      .filter_map(|k| k.parse().ok())
      .collect();
    {
      let index = s.state().index.lock().expect("index lock");
      for item in index.attention().unwrap_or_default() {
        if let stitch_store::AttentionItem::Question { id, .. } = item
          && let Some(t) = index
            .edges_from(&id)
            .ok()
            .and_then(|e| e.into_iter().find(|e| e.rel == "thread"))
        {
          extra.push(t.dst.id);
        }
      }
    }
    extra.sort();
    extra.dedup();
    extra.retain(|t| !ids.contains(t));
    extra.extend(ids);
    ids = extra;
  }
  // Each listed thread's place (a row's `place`: the label), for the
  // client's sections: counts over all of them, whatever the window, and
  // `place`'s filter, before the limit. Only the user's own list (`every`).
  let want_place = param(query, "place")
    .filter(|v| !v.is_empty())
    .map(super::serve::percent_decode);
  let mut places: Vec<Value> = Vec::new();
  if every {
    let here = s.state().node.to_string();
    let known = s.places();
    let mut counts: Vec<(String, Option<Place>, usize)> = Vec::new();
    let mut keep = Vec::with_capacity(ids.len());
    for id in &ids {
      let place = placed.get(id);
      let label = place.map_or_else(|| "?".to_owned(), |p| p.label(&here, &known));
      match counts.iter_mut().find(|c| c.0 == label) {
        Some(c) => c.2 += 1,
        None => counts.push((label.clone(), place.cloned(), 1)),
      }
      if want_place.as_ref().is_none_or(|w| *w == label) {
        keep.push(id.clone());
      }
    }
    places = counts
      .into_iter()
      .map(|(place, p, count)| {
        // `memory: false` only when the place's settings say so (§2.9).
        if p.is_some_and(|p| s.place_memory_off(&p)) {
          json!({"place": place, "count": count, "memory": false})
        } else {
          json!({"place": place, "count": count})
        }
      })
      .collect();
    ids = keep;
  }
  // `id`: that one thread's row, wherever it falls in the list (a client
  // opening a thread older than its window).
  // A thread compacted into another is asked for as that one: its row is
  // the newest's (its `earlier` names the one asked for).
  if let Some(one) = param(query, "id").filter(|v| !v.is_empty()) {
    let one = match one.parse::<RecordId>() {
      Ok(t) => crate::fork::current(s, &t).to_string(),
      Err(_) => one.to_owned(),
    };
    ids.retain(|t| t.as_str() == one);
  }
  // The floors of the read cursors, once for the request.
  let floors = crate::meta::Floors::read(&s.state().index.lock().expect("index lock"));
  // Every thread's latest digest, read once (design §2.25, step 2).
  let digests: std::collections::HashMap<RecordId, stitch_store::DigestRow> = s
    .state()
    .index
    .lock()
    .expect("index lock")
    .digests()
    .unwrap_or_default()
    .into_iter()
    .map(|d| (d.thread.clone(), d))
    .collect();
  let all_places = s.places();
  if forest {
    // Their transcripts, read at once on a few threads, not one by one.
    let wanted = if status.is_some() || since.is_some() {
      &ids[..]
    } else {
      &ids[..ids.len().min(limit)]
    };
    crate::weave::warm_rows(s, wanted, &s.apps.live.rows, &all_places);
  }
  let mut out = Vec::new();
  let mut more = 0;
  for id in &ids {
    if since.is_some() {
      // Cheap first: when it started and last stepped.
      let started = s
        .state()
        .index
        .lock()
        .expect("index lock")
        .record(id)
        .ok()
        .flatten()
        .map(|r| r.created);
      let last = s
        .state()
        .index
        .lock()
        .expect("index lock")
        .last_segment_at(id)
        .ok()
        .flatten();
      let latest = started.max(last);
      let running = s
        .detached
        .steers
        .lock()
        .expect("steers lock")
        .contains_key(id.as_str());
      if !running && latest.is_some_and(|t| Some(t) < since) {
        continue;
      }
    }
    // Past the limit with no status to match, a thread only counts into
    // `more`: its row (the costly part) is not built.
    if status.is_none() && out.len() == limit {
      more += 1;
      continue;
    }
    let r = crate::weave::row_in(s, id, Some(&s.apps.live.rows), &all_places);
    let state = match (r.word, r.outcome.as_deref()) {
      ("ended", Some("cancelled")) => "cancelled",
      (w, _) => w,
    };
    if status.is_some_and(|w| w != state) {
      continue;
    }
    if out.len() == limit {
      more += 1;
      continue;
    }
    let last_words = r.last_words.as_deref().map(|w| {
      let cut: String = w.chars().take(LAST_WORDS).collect();
      if cut.len() < w.len() {
        format!("{cut}…")
      } else {
        cut
      }
    });
    // A compaction chain lists as one row: its title and pin are the
    // chain's, `earlier` the threads it was compacted from, oldest first.
    // Outside the index lock: `thread_place` takes it.
    let place_key = placed
      .get(&r.id)
      .cloned()
      .or_else(|| s.thread_place_in(&r.id, &all_places).ok().map(|(_, p)| p))
      .map(|p| p.key());
    let (title, pinned, earlier_ids, cursor) = {
      let index = s.state().index.lock().expect("index lock");
      let mut lineage = index.compaction_lineage(&r.id);
      lineage.retain(|t| t != &r.id);
      (
        index.lineage_title(&r.id),
        index.lineage_pinned(&r.id),
        lineage,
        crate::meta::cursor_of(&index, &r.id, place_key.as_deref(), &floors),
      )
    };
    let digest = crate::digests::row_digest(&digests, &r.id, &earlier_ids, r.last_step);
    let earlier = earlier_ids
      .iter()
      .map(ToString::to_string)
      .collect::<Vec<_>>();
    // Activity after what the user has read; never read is unread (design
    // §2.25). A thread with no step yet has nothing to read.
    let unread = r.last_step.is_some_and(|l| cursor.is_none_or(|c| l > c));
    let mut row = json!({
      "id": r.id.to_string(),
      "title": title.or_else(|| r.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| r.goal.clone()),
      "earlier": earlier,
      "goal": if forest { clip(&r.goal, 300) } else { r.goal.clone() },
      "node": r.node,
      "started_on": r.started_on,
      "place": r.place,
      "parent": r.parent.as_ref().map(|p| p.to_string()),
      // A parent the user deleted reads as such (design §2.5).
      "parent_title": r.parent.as_ref().and_then(|p| {
        title_of(s, p.as_str()).or_else(|| {
          let deleted = s.state().index.lock().expect("index lock").is_deleted(p).unwrap_or(false);
          deleted.then(|| "A deleted thread".to_owned())
        })
      }),
      "detached": r.detached,
      "read_only": s.read_only(&r.id),
      // A `set` of `pinned` by the user, on any thread of its chain: first
      // in lists.
      "pinned": pinned,
      "unread": unread,
      // An app's thread names its app (design §2.22).
      "app": if forest { None } else { super::threads::app_said(s, &r.id) },
      "state": state,
      "outcome": r.outcome,
      "said": r.state,
      "question": r.question.as_ref().map(|(q, c)| json!({"id": q.to_string(), "command": c})),
      "started": stamp(r.started),
      "last_step": stamp(r.last_step),
      "model_calls": r.steps,
      "cost": r.usd,
      "last_words": last_words,
      // The call its turn runs here now, its step not written till it
      // returns: what runs, its input whole, since when.
      "in_flight": (!forest).then(|| running_call(s, r.id.as_str())).flatten().map(|c| json!({
        "name": c.name,
        "headline": c.headline(),
        "summary": c.summary,
        "since": stamp(c.since),
      })),
    });
    // Its digest's headline and counts (absent when it has none).
    if let Some(d) = digest {
      row["digest"] = d;
    }
    // A room (design §2.27): its participants and mode; absent on other
    // rows, so the list stays lean.
    {
      let index = s.state().index.lock().expect("index lock");
      if crate::rooms::is_room(&index, &r.id) {
        row["room"] = json!(true);
        row["participants"] = Value::Array(crate::rooms::participants(&index, &r.id));
        row["room_mode"] = json!(crate::rooms::policy(&index, &r.id).mode);
      }
      // Imported from a shared file (design §2.31): absent otherwise.
      if let Some(o) = crate::share::origin(&index, &r.id) {
        row["imported_from"] = json!(o.from);
        row["imported_source"] = json!(o.source);
        row["imported_signed"] = json!(o.signed);
        row["imported_own"] = json!(o.own);
      }
    }
    out.push(row);
  }
  Ok(json!({
    "node": s.state().node.to_string(),
    "at": Timestamp::now().to_string(),
    "threads": out,
    "more": more,
    "places": places,
  }))
}

/// `Request::Live places`: where a new thread can go (what the phone's
/// `places()` lists): the places of the threads, and the projects. Each is
/// `{key, label}`, `key` what `Chat.place` takes; the global place is the
/// client's own option.
pub(crate) fn places(s: &Arc<Shared>) -> Value {
  use stitch_store::Role;
  let here = s.state().node.to_string();
  let home = s
    .state()
    .roots()
    .iter()
    .find(|r| r.config().role == Role::Home)
    .map(|r| r.repo().clone());
  let places = s.places();
  let index = s.state().index.lock().expect("index lock");
  let threads = index.threads().unwrap_or_default();
  let mut seen = std::collections::HashSet::new();
  let used = threads
    .iter()
    .map(|(r, _)| Place::of_thread(r, home.as_ref(), &places));
  let projects = places.listed().map(|p| Place::Project(p.repo.clone()));
  let out: Vec<Value> = used
    .chain(projects)
    .filter(|p| *p != Place::Global && seen.insert(p.key()))
    .map(|p| json!({"key": p.key(), "label": p.label(&here, &places)}))
    .collect();
  json!({"node": here, "places": out})
}

/// `stitch.live.attention()`: the questions waiting on the user, from any
/// node, the oldest first (approvals with their exact command).
pub(crate) fn attention(s: &Arc<Shared>) -> Value {
  let here = s.state().node.to_string();
  let places = s.places();
  let mut out = Vec::new();
  let items = {
    let index = s.state().index.lock().expect("index lock");
    index.attention().unwrap_or_default()
  };
  for item in items {
    let stitch_store::AttentionItem::Question { id, title } = item else {
      continue;
    };
    let (record, thread) = {
      let index = s.state().index.lock().expect("index lock");
      let record = index.record(&id).ok().flatten();
      let thread = index
        .edges_from(&id)
        .ok()
        .and_then(|e| e.into_iter().find(|e| e.rel == "thread"))
        .map(|e| e.dst.id);
      (record, thread)
    };
    let Some(r) = record else { continue };
    let field = |k: &str| r.fields.get(k).and_then(Value::as_str).map(str::to_owned);
    let place = thread
      .as_ref()
      .and_then(|t| s.thread_place(t).ok())
      .map(|(_, p)| p.label(&here, &places));
    out.push(json!({
      "id": id.to_string(),
      "title": title,
      "thread": thread.as_ref().map(|t| t.to_string()),
      "thread_title": thread.as_ref().and_then(|t| title_of(s, t.as_str())),
      "node": r.node.to_string(),
      "place": place,
      "session": field("session"),
      "command": field("command"),
      "approval": r.fields.get("approval").and_then(Value::as_bool) == Some(true),
      "grant": r.fields.get("grant").cloned().unwrap_or(json!([])),
      "asked": r.created.to_string(),
    }));
  }
  out.sort_by(|a, b| a["asked"].as_str().cmp(&b["asked"].as_str()));
  json!({
    "node": here,
    "at": Timestamp::now().to_string(),
    "questions": out,
    "threads": needs_threads(s),
  })
}

/// The threads that wait on the user without an approval of their own (an
/// open question is a `questions` item): the one list the web client, the
/// dashboard and the phone count (design §2.24, "Needs you").
///
/// - blocked: a thread `blocked` for a reason other than an open approval
///   (an answer that could not be delivered), or whose last report's
///   outcome is `blocked`;
/// - except a spawned thread whose report went to its parent (its goal's
///   `reports-to`/`spawned-by`) and was not kept for the user
///   (`wake = false`): the parent has it, and answers or relays it;
/// - except what the user dismissed (`loose-end-dismissed`, the same set
///   `LooseEnds { dismiss }` writes) since the thread's last step: a new
///   step brings it back.
pub(crate) fn needs_threads(s: &Arc<Shared>) -> Vec<Value> {
  let ids: Vec<RecordId> = {
    let index = s.state().index.lock().expect("index lock");
    index
      .threads()
      .unwrap_or_default()
      .into_iter()
      .map(|(r, _)| r.id)
      .filter(|id| !index.is_deleted(id).unwrap_or(false))
      .collect()
  };
  let mut out = Vec::new();
  for id in ids {
    // Cheap first, from the index alone: only a thread `blocked`, or idle
    // with a report whose outcome is `blocked`, can be one. Its row (its
    // transcript's last step read) only then: building every thread's row
    // read all of them, each time a client asked (370 ms and 13 MB on
    // gamer's 1,800 threads).
    let (_, word) = crate::weave::state_of(s, &id);
    let maybe = match word {
      "blocked" => true,
      "idle" => {
        let index = s.state().index.lock().expect("index lock");
        index
          .record(&id)
          .ok()
          .flatten()
          .and_then(|r| crate::chat::detached_state(&index, &r).ok().flatten())
          .as_deref()
          == Some("blocked")
      }
      _ => false,
    };
    if !maybe {
      continue;
    }
    let r = crate::weave::row(s, &id, Some(&s.apps.live.rows));
    let by_report = r.word == "idle" && r.outcome.as_deref() == Some("blocked");
    let by_state = r.word == "blocked" && r.question.is_none();
    if !by_report && !by_state {
      continue;
    }
    let index = s.state().index.lock().expect("index lock");
    let last = index.last_segment_at(&id).ok().flatten();
    if let (Some(d), Some(l)) = (crate::loose::dismissed_at(&index, &id), last)
      && d >= l
    {
      continue;
    }
    if by_report && let Some(parent) = &r.parent {
      let parent_here =
        index.record(parent).ok().flatten().is_some() && !index.is_deleted(parent).unwrap_or(false);
      let kept = index
        .sources(kinds::REPORT, "thread", &id, None)
        .ok()
        .and_then(|rs| rs.into_iter().rfind(|r| !crate::chat::is_woven_reply(r)))
        .is_some_and(|r| r.fields.get("wake").and_then(Value::as_bool) == Some(false));
      if parent_here && !kept {
        continue;
      }
    }
    drop(index);
    out.push(json!({
      "id": id.to_string(),
      "title": r.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| r.goal.clone()),
      "why": if by_report { "reported-blocked" } else { "blocked" },
      "said": r.state,
      "parent": r.parent.as_ref().map(|p| p.to_string()),
      "since": stamp(last),
    }));
  }
  out.sort_by(|a, b| b["since"].as_str().cmp(&a["since"].as_str()));
  out
}

/// `stitch.live.jobs()`: this node's jobs, every thread's: running, and
/// ended within the hour (a phone has none).
pub(crate) async fn jobs(s: &Arc<Shared>) -> Value {
  let mut out = Vec::new();
  for (_, mut j) in s.shells.live().await {
    if let Some(t) = j["thread"].as_str().map(str::to_owned) {
      j["thread_title"] = json!(title_of(s, &t));
    }
    out.push(j);
  }
  json!({
    "node": s.state().node.to_string(),
    "at": Timestamp::now().to_string(),
    "jobs": out,
  })
}

/// `stitch.live.schedules({all})`: the schedules of every node, self-wakes
/// included, the soonest due first (with `all`, the ended ones too).
pub(crate) fn schedules(s: &Arc<Shared>, query: &str) -> Value {
  json!({
    "node": s.state().node.to_string(),
    "at": Timestamp::now().to_string(),
    "schedules": crate::schedules::live(s, flag(query, "all")),
  })
}

/// The `set` field an app's consent is kept in, on its thread.
fn field(name: &str) -> String {
  format!("live:{name}")
}

/// A thread's `set` of `field` standing now (its latest head), as text.
fn standing(index: &stitch_store::Index, thread: &RecordId, field: &str) -> Option<String> {
  let sets: Vec<stitch_store::IndexedRecord> = index
    .sources(kinds::SET, "target", thread, None)
    .ok()?
    .into_iter()
    .filter(|r| r.fields.get("field").and_then(Value::as_str) == Some(field))
    .collect();
  let mut superseded = std::collections::HashSet::new();
  for r in &sets {
    for e in index.edges_from(&r.id).ok()? {
      if e.rel == "supersedes" {
        superseded.insert(e.dst.id.to_string());
      }
    }
  }
  sets
    .into_iter()
    .filter(|r| !superseded.contains(r.id.as_str()))
    .max_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)))
    .and_then(|r| match r.fields.get("value") {
      Some(Value::String(v)) => Some(v.clone()),
      Some(Value::Bool(b)) => Some(b.to_string()),
      _ => None,
    })
}

/// Whether the user let an app read live state: `Some(true)` allowed,
/// `Some(false)` denied, `None` never asked. A pin's own answer first,
/// else its thread's for its version's name.
pub(crate) fn consent(s: &Shared, m: &Manifest, pinned: Option<&Pinned>) -> Option<bool> {
  consent_in(&s.state().index.lock().expect("index lock"), m, pinned)
}

/// `consent`, from the index.
pub fn consent_in(
  index: &stitch_store::Index,
  m: &Manifest,
  pinned: Option<&Pinned>,
) -> Option<bool> {
  if let Some(v) = pinned.and_then(|p| p.live) {
    return Some(v);
  }
  let thread = m.thread.as_ref()?;
  standing(index, thread, &field(&m.name)).map(|v| v == "true" || v == "allow")
}

/// The app served at `host`, as the phone's consent sheet names it (a
/// pin's name, else its version's), and the user's answer so far.
pub fn asking(
  index: &stitch_store::Index,
  blobs: &stitch_store::BlobStore,
  host: &str,
) -> Option<(String, Option<bool>)> {
  match super::Host::parse(host)? {
    super::Host::Pinned(h) => {
      let p = super::pinned(index)
        .ok()?
        .into_iter()
        .find(|p| p.host == h)?;
      let m = Manifest::read(blobs, &p.manifest);
      let consent = match &m {
        Some(m) => consent_in(index, m, Some(&p)),
        None => p.live,
      };
      Some((p.name, consent))
    }
    super::Host::Version(v) => {
      let m = Manifest::read(blobs, &v)?;
      let consent = consent_in(index, &m, None);
      Some((m.name, consent))
    }
  }
}

/// The app a name, an ID or a host names: a pinned app, else (a version's
/// host, or a name with `thread`) an app of a conversation.
enum Named {
  Pinned(Pinned),
  InThread { thread: RecordId, name: String },
}

fn named(s: &Shared, app: &str, thread: Option<&str>) -> Result<Named> {
  if let Some(t) = thread {
    let thread: RecordId = t
      .parse()
      .map_err(|_| Error::Config(format!("`{t}` is not a thread's ID")))?;
    s.thread_place(&thread)?;
    return Ok(Named::InThread {
      thread,
      name: app.to_owned(),
    });
  }
  if let Some(super::Host::Version(h)) = super::Host::parse(app) {
    let m = Manifest::read(&s.state().blobs, &h)
      .ok_or_else(|| Error::Config(format!("no app {h} here")))?;
    let thread = m.thread.ok_or_else(|| {
      Error::Config(format!(
        "app \"{}\" has no conversation to keep the answer in: pin it, and allow the pin",
        m.name
      ))
    })?;
    return Ok(Named::InThread {
      thread,
      name: m.name,
    });
  }
  let pinned = super::pinned(&s.state().index.lock().expect("index lock"))?;
  super::find_pinned(&pinned, app)
    .cloned()
    .map(Named::Pinned)
    .ok_or_else(|| {
      Error::Config(format!(
        "no pinned app {app} (stitch app list); for an app in its conversation, name its \
         thread too (--thread <id>)"
      ))
    })
}

/// The user lets an app read live state, or not: on its pin, or on its
/// thread for its name. What was done, as said.
pub(crate) fn allow(s: &Shared, app: &str, thread: Option<&str>, allow: bool) -> Result<String> {
  let said = if allow { "may now" } else { "may no longer" };
  match named(s, app, thread)? {
    Named::Pinned(p) => {
      super::pin_with(
        s,
        &p.manifest,
        Some(&p.name),
        None,
        Some(&p.id),
        p.fixed,
        super::Extra {
          live: Some(allow),
          builtin: None,
        },
      )?;
      Ok(format!(
        "The pinned app \"{}\" {said} see your threads, jobs and approvals.",
        p.name
      ))
    }
    Named::InThread { thread, name } => {
      crate::meta::set(
        s,
        &thread,
        &field(&name),
        if allow { "allow" } else { "deny" }.into(),
        Author::user(s.state().node.clone()),
      )?;
      Ok(format!(
        "App \"{name}\" of thread {thread} {said} see your threads, jobs and approvals."
      ))
    }
  }
}

/// What the page is told when it may not read live state: whether it may
/// ask the user (`ask`: nobody answered yet) or was denied, and how the
/// user allows it (`thread`: an app in its conversation, not pinned).
pub(crate) fn refused(name: &str, thread: Option<&RecordId>, consent: Option<bool>) -> Value {
  let how = match thread {
    Some(t) => format!("stitch app allow-live \"{name}\" --thread {t}"),
    None => format!("stitch app allow-live \"{name}\""),
  };
  let (state, error) = match consent {
    Some(false) => (
      "denied",
      format!(
        "\"{name}\" may not see your threads, jobs and approvals: you denied it. To allow it, \
         run `{how}` on a computer of yours."
      ),
    ),
    _ => (
      "ask",
      format!(
        "\"{name}\" asks to see your threads, jobs and approvals. Allow it in the Stitch app, or \
         run `{how}`."
      ),
    ),
  };
  json!({"error": error, "consent": state, "name": name, "allow": how})
}

/// The built-in dashboard (design §2.10): its files, in the binary. Since
/// version 8 a second entry of the web client's bundle (`apps/client/src/dashboard`,
/// built by `just client` into `apps/client/dist/dashboard/`): the client's
/// home view on `stitch.live`.
pub(crate) const DASHBOARD: &[(&str, &str)] = &[
  (
    "index.html",
    include_str!("../../../../apps/client/dist/dashboard/index.html"),
  ),
  (
    "app.js",
    include_str!("../../../../apps/client/dist/dashboard/app.js"),
  ),
  (
    "app.css",
    include_str!("../../../../apps/client/dist/dashboard/app.css"),
  ),
];
/// Raised when the dashboard changes: a node with a newer one moves the
/// pin to it, never back.
pub(crate) const DASHBOARD_VERSION: u64 = 8;
/// Its `builtin` on the pin.
pub(crate) const DASHBOARD_KEY: &str = "dashboard";

/// Pins the built-in dashboard, or moves its pin to this node's version,
/// allowed to read live state (the user asked for it by name: `stitch
/// dashboard`, the phone's offer).
pub(crate) fn dashboard(s: &Shared) -> Result<Pinned> {
  let state = s.state();
  let files: Vec<(&str, &[u8])> = DASHBOARD.iter().map(|(p, t)| (*p, t.as_bytes())).collect();
  let v = super::snapshot_files(&state.blobs, &files, "Dashboard", false, None)?;
  let existing = super::pinned(&state.index.lock().expect("index lock"))?
    .into_iter()
    .find(|p| p.builtin.as_ref().is_some_and(|(k, _)| k == DASHBOARD_KEY));
  if let Some(p) = &existing
    && p.manifest == v.manifest
    && p.live == Some(true)
  {
    return Ok(p.clone());
  }
  super::pin_with(
    s,
    &v.manifest,
    Some(existing.as_ref().map_or("Dashboard", |p| p.name.as_str())),
    Some("gauge.with.dots.needle.50percent"),
    existing.as_ref().map(|p| &p.id),
    false,
    super::Extra {
      live: Some(true),
      builtin: Some((DASHBOARD_KEY.into(), DASHBOARD_VERSION)),
    },
  )
}

/// At start: a dashboard pinned from an older built-in moves to this
/// node's (never to an older one: two nodes of other versions do not move
/// it back and forth).
pub(crate) fn upgrade_dashboard(s: &Shared) -> Result<bool> {
  let older = super::pinned(&s.state().index.lock().expect("index lock"))?
    .into_iter()
    .any(|p| matches!(&p.builtin, Some((k, v)) if k == DASHBOARD_KEY && *v < DASHBOARD_VERSION));
  if older {
    dashboard(s)?;
  }
  Ok(older)
}

/// The built-in web client (design §2.24): its bundle, in the binary
/// (`apps/client/dist/`, built at development time and committed). The one app
/// whose origin may open the node's WebSocket (`/_stitch/ws`, `ws.rs`).
pub(crate) const CLIENT: &[(&str, &str)] = &[
  (
    "index.html",
    include_str!("../../../../apps/client/dist/index.html"),
  ),
  (
    "app.js",
    include_str!("../../../../apps/client/dist/app.js"),
  ),
  (
    "app.css",
    include_str!("../../../../apps/client/dist/app.css"),
  ),
  // Space, the map in 3D (three.js): an ES module the page imports the first
  // time 3D is shown, so `app.js` stays small (design §2.24).
  (
    "space.js",
    include_str!("../../../../apps/client/dist/space.js"),
  ),
];
/// Raised when the client changes: a node with a newer one moves the pin
/// to it, never back.
pub(crate) const CLIENT_VERSION: u64 = 2;
/// Its `builtin` on the pin.
pub(crate) const CLIENT_KEY: &str = "client";

/// Whether a pin is the built-in client.
pub(crate) fn is_client(p: &Pinned) -> bool {
  p.builtin.as_ref().is_some_and(|(k, _)| k == CLIENT_KEY)
}

/// The version a pin is served from. The client's pin syncs between nodes
/// of different builds, but the page must speak the protocol of the node
/// it talks to (its `Hello`): a client pin is always served from the bundle
/// in this binary, whatever manifest the pin holds.
pub(crate) fn serving_manifest(
  blobs: &stitch_store::BlobStore,
  p: &Pinned,
) -> Result<stitch_records::BlobHash> {
  if !is_client(p) {
    return Ok(p.manifest);
  }
  // Hashed once per process; the blobs are stored again only if the store
  // lacks them (a fresh data directory, as in tests).
  static LOCAL: std::sync::OnceLock<stitch_records::BlobHash> = std::sync::OnceLock::new();
  if let Some(m) = LOCAL.get()
    && blobs.contains(m)
  {
    return Ok(*m);
  }
  let files: Vec<(&str, &[u8])> = CLIENT.iter().map(|(p, t)| (*p, t.as_bytes())).collect();
  let m = super::snapshot_files(blobs, &files, "Stitch", false, None)?.manifest;
  let _ = LOCAL.set(m);
  Ok(m)
}

/// Pins the built-in client, or moves its pin to this node's version.
pub(crate) fn client(s: &Shared) -> Result<Pinned> {
  let state = s.state();
  let files: Vec<(&str, &[u8])> = CLIENT.iter().map(|(p, t)| (*p, t.as_bytes())).collect();
  let v = super::snapshot_files(&state.blobs, &files, "Stitch", false, None)?;
  let existing = super::pinned_all(&state.index.lock().expect("index lock"))?
    .into_iter()
    .find(is_client);
  if let Some(p) = &existing
    && p.manifest == v.manifest
  {
    return Ok(p.clone());
  }
  super::pin_with(
    s,
    &v.manifest,
    Some(existing.as_ref().map_or("Stitch", |p| p.name.as_str())),
    Some("bubble.left.and.bubble.right"),
    existing.as_ref().map(|p| &p.id),
    false,
    super::Extra {
      live: Some(true),
      builtin: Some((CLIENT_KEY.into(), CLIENT_VERSION)),
    },
  )
}

/// At start: the client is pinned when it is not yet, or moved when the pin
/// is of an older `CLIENT_VERSION`; never for the same or a newer one (what
/// is served is this binary's bundle either way, `serving_manifest`).
/// Whether it changed.
pub(crate) fn ensure_client(s: &Shared) -> Result<bool> {
  let existing = super::pinned_all(&s.state().index.lock().expect("index lock"))?
    .into_iter()
    .find(is_client);
  // Nodes of different builds share the pin and each serves its own bundle
  // (`serving_manifest`): the pin is not moved for the same version, or
  // the builds would move it back and forth.
  if existing
    .as_ref()
    .and_then(|p| p.builtin.as_ref())
    .is_some_and(|(_, v)| *v >= CLIENT_VERSION)
  {
    return Ok(false);
  }
  let before = existing.map(|p| p.manifest);
  let now = client(s)?;
  Ok(before.as_ref() != Some(&now.manifest))
}

/// The built-in dashboard on sample data, for the phone's screenshots
/// (its fixtures, with no node): the page with the bridge, and canned
/// live state (`dashboard/fixture.json`, its `"@-N"` N seconds ago and
/// `"@+N"` N seconds on).
pub fn fixture(path: &str) -> super::serve::Response {
  let path = path.split('?').next().unwrap_or("/");
  let ok = |media: &str, body: Vec<u8>| super::serve::Response {
    status: 200,
    headers: vec![("Content-Type".into(), media.into())],
    body,
  };
  let data: Value = {
    let now = Timestamp::now();
    let mut text = include_str!("dashboard/fixture.json").to_owned();
    // `"@-N"`: N seconds ago; `"@+N"`: in N seconds.
    while let Some(at) = text.find("\"@-").or_else(|| text.find("\"@+")) {
      let end = text[at + 3..].find('"').map_or(text.len(), |e| at + 3 + e);
      let secs: i64 = text[at + 3..end].parse().unwrap_or(0);
      let secs = if &text[at + 2..at + 3] == "-" {
        -secs
      } else {
        secs
      };
      let t = now + jiff::SignedDuration::from_secs(secs);
      text.replace_range(at..=end, &format!("\"{t}\""));
    }
    serde_json::from_str(&text).unwrap_or_default()
  };
  match path {
    "/" | "/index.html" => {
      let start = json!({
        "name": "Dashboard", "version": "fixture", "pinned": true, "state": {}, "generation": 0,
        "live": {"consent": "allowed", "generation": 0, "threads": 0, "jobs": 0, "attention": 0, "schedules": 0},
      });
      let page = DASHBOARD[0].1.replacen(
        "<head>",
        &format!(
          "<head><script>window.__stitch={start};</script><script src=\"/_stitch/bridge.js\"></script>"
        ),
        1,
      );
      ok("text/html; charset=utf-8", page.into_bytes())
    }
    "/app.js" => ok("text/javascript", DASHBOARD[1].1.into()),
    "/app.css" => ok("text/css", DASHBOARD[2].1.into()),
    "/_stitch/bridge.js" => ok("text/javascript", include_bytes!("bridge.js").to_vec()),
    "/_stitch/stitch.css" => ok("text/css", include_bytes!("stitch.css").to_vec()),
    "/_stitch/live/threads" => ok("application/json", data["threads"].to_string().into()),
    "/_stitch/live/attention" => ok("application/json", data["attention"].to_string().into()),
    "/_stitch/live/jobs" => ok("application/json", data["jobs"].to_string().into()),
    "/_stitch/live/schedules" => ok("application/json", data["schedules"].to_string().into()),
    "/_stitch/live/loose-ends" => ok("application/json", data["loose_ends"].to_string().into()),
    "/_stitch/events" => ok(
      "application/json",
      json!({"state": {}, "generation": 0, "live": {"consent": "allowed", "generation": 0, "threads": 0, "jobs": 0, "attention": 0, "schedules": 0}})
        .to_string()
        .into(),
    ),
    _ => super::serve::Response {
      status: 404,
      headers: Vec::new(),
      body: Vec::new(),
    },
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn queries_are_read() {
    assert_eq!(param("all=1&status=working", "status"), Some("working"));
    assert!(flag("all&x=2", "all"));
    assert!(flag("all=true", "all"));
    assert!(!flag("all=0", "all"));
    assert_eq!(param("a=1", "b"), None);
  }

  #[test]
  fn the_limit_goes_above_500_only_for_the_forest() {
    assert_eq!(limit_of(""), 100);
    assert_eq!(limit_of("all=1&limit=2000"), 500);
    assert_eq!(limit_of("all=1&every=1&limit=2000"), 500);
    assert_eq!(limit_of("all=1&conversations=1&limit=2000"), 2000);
    assert_eq!(limit_of("conversations=1&limit=9000"), 5000);
    assert_eq!(limit_of("conversations=1&limit=0"), 1);
    assert_eq!(limit_of("conversations=0&limit=900"), 500);
  }

  #[test]
  fn long_goals_are_cut_on_a_character() {
    assert_eq!(clip("abc", 5), "abc");
    assert_eq!(clip("abcdef", 3), "abc…");
    assert_eq!(clip("àèìòù", 2), "àè…");
  }

  fn pin(manifest: stitch_records::BlobHash, builtin: Option<(String, u64)>) -> Pinned {
    Pinned {
      id: "2026-10-14T120000Z-mbp-k3f9".parse().unwrap(),
      name: "Stitch".into(),
      icon: None,
      manifest,
      thread: None,
      fixed: false,
      at: Timestamp::now(),
      host: "h".into(),
      live: None,
      builtin,
    }
  }

  #[test]
  fn a_client_pin_of_another_bundle_serves_this_binarys_app() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = stitch_store::BlobStore::open(&dir.path().join("blobs")).unwrap();
    // Another build's bundle, synced into the pin.
    let other = super::super::snapshot_files(
      &blobs,
      &[("index.html", b"<p>x</p>"), ("app.js", b"local-OTHER")],
      "Stitch",
      false,
      None,
    )
    .unwrap();
    let client_pin = pin(other.manifest, Some((CLIENT_KEY.into(), CLIENT_VERSION)));
    let m = serving_manifest(&blobs, &client_pin).unwrap();
    assert_ne!(m, other.manifest);
    let manifest = Manifest::read(&blobs, &m).unwrap();
    let flat = crate::folders::load_tree(&blobs, &manifest.tree).unwrap();
    let Some(crate::folders::Leaf::File { hash, .. }) = flat.get("app.js") else {
      panic!("no app.js")
    };
    let want = CLIENT.iter().find(|(p, _)| *p == "app.js").unwrap().1;
    assert_eq!(std::fs::read(blobs.path(hash)).unwrap(), want.as_bytes());
    // Any other pin is served as it is.
    let plain = pin(other.manifest, None);
    assert_eq!(serving_manifest(&blobs, &plain).unwrap(), other.manifest);
  }
}
