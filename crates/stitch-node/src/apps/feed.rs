//! What the web client's WebSocket pushes in place of `changed` (design
//! §2.24, "Deltas, not re-fetches"): the rows of the threads that moved,
//! the live sections that moved, whole (each is small), and the calls of
//! the threads a connection watches.
//!
//! One pass a second, shared by every connection (`watch`'s tick, while
//! one subscribes): which threads moved is read from what the index took
//! in since the last pass (records and segments past the last rowids, and
//! the threads their edges name) and from the loops starting and ending
//! here; only those rows are built. A running call is not a row's change:
//! it goes to the connections watching that thread (`Push::Call`). What
//! the pass cannot follow (rows removed from the index, a place, a project,
//! a folder or a read floor written: what every row may read) is a
//! `Reset`: the connections send their list again whole.

// Its readers are the WebSocket's (`ws.rs`), which a phone or headset does
// not serve: there the feed is only fed.
#![cfg_attr(mobile, allow(dead_code))]

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use stitch_records::{RecordId, kinds};
use tokio::sync::broadcast;

use crate::node::Shared;

/// Loose ends move with every step of a conversation (unread): read again
/// at most this often.
const LOOSE_EVERY: Duration = Duration::from_secs(10);

/// One push, the same for every connection (each filters and numbers it).
#[derive(Debug)]
pub(crate) enum Push {
  /// Rows to put in place (whole), threads to take out (deleted, compacted
  /// into another); `places`: the per-place counts, when threads came or
  /// went.
  Rows {
    upsert: Vec<Value>,
    remove: Vec<String>,
    places: Option<Value>,
  },
  /// What no delta says: read the list again.
  Reset,
  /// A live section, whole.
  Section { what: &'static str, value: Value },
  /// The call a loop here runs now on `thread` (null: none).
  Call { thread: String, in_flight: Value },
}

pub(crate) struct Feed {
  tx: broadcast::Sender<Arc<Push>>,
  subscribers: AtomicUsize,
  state: Mutex<State>,
}

impl Default for Feed {
  fn default() -> Feed {
    Feed {
      tx: broadcast::channel(256).0,
      subscribers: AtomicUsize::new(0),
      state: Mutex::new(State::default()),
    }
  }
}

#[derive(Default)]
struct State {
  /// `Index::cursor` at the last pass.
  cursor: Option<[i64; 4]>,
  /// The threads a loop runs or starts here.
  running: BTreeSet<String>,
  /// Their calls, as last pushed.
  calls: HashMap<String, Value>,
  loose_at: Option<Instant>,
  loose_due: bool,
}

/// A connection subscribed: the feed runs while one is.
pub(crate) struct Subscribed(Arc<Shared>);

impl Drop for Subscribed {
  fn drop(&mut self) {
    self
      .0
      .apps
      .live
      .feed
      .subscribers
      .fetch_sub(1, Ordering::SeqCst);
  }
}

impl Feed {
  pub(crate) fn subscribe(s: &Arc<Shared>) -> (broadcast::Receiver<Arc<Push>>, Subscribed) {
    let f = &s.apps.live.feed;
    f.subscribers.fetch_add(1, Ordering::SeqCst);
    (f.tx.subscribe(), Subscribed(s.clone()))
  }

  pub(crate) fn active(&self) -> bool {
    self.subscribers.load(Ordering::SeqCst) > 0
  }

  /// The call each of `threads` runs now, as last seen.
  pub(crate) fn calls_of(&self, threads: &[String]) -> Vec<(String, Value)> {
    let st = self.state.lock().expect("feed lock");
    threads
      .iter()
      .map(|t| (t.clone(), st.calls.get(t).cloned().unwrap_or(Value::Null)))
      .collect()
  }

  fn send(&self, p: Push) {
    let _ = self.tx.send(Arc::new(p));
  }
}

fn call_value(s: &Shared, thread: &str) -> Value {
  super::live::running_call(s, thread).map_or(Value::Null, |c| {
    json!({
      "name": c.name,
      "headline": c.headline(),
      "summary": c.summary,
      "since": c.since.map(|t| t.to_string()),
    })
  })
}

/// One pass (`watch`'s tick): `moved`, the kinds whose marks moved.
pub(crate) async fn tick(s: &Arc<Shared>, moved: &[&'static str]) {
  let feed = &s.apps.live.feed;
  if !feed.active() {
    // Nobody listens: start afresh when one does.
    *feed.state.lock().expect("feed lock") = State::default();
    return;
  }
  let s2 = s.clone();
  let moved2: Vec<&'static str> = moved.to_vec();
  let _ = tokio::task::spawn_blocking(move || pass(&s2, &moved2)).await;
  if moved.contains(&"jobs") {
    let value = super::live::jobs(s).await;
    feed.send(Push::Section {
      what: "jobs",
      value,
    });
  }
}

fn pass(s: &Arc<Shared>, moved: &[&'static str]) {
  let feed = &s.apps.live.feed;
  let mut running: BTreeSet<String> = s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .keys()
    .cloned()
    .collect();
  running.extend(
    s.detached
      .starting
      .lock()
      .expect("starting lock")
      .keys()
      .cloned(),
  );
  let calls: HashMap<String, Value> = running
    .iter()
    .map(|t| (t.clone(), call_value(s, t)))
    .collect();
  let cursor = s.state().index.lock().expect("index lock").cursor().ok();
  let mut dirty: BTreeSet<RecordId> = BTreeSet::new();
  let mut reset = false;
  let mut came = false;
  {
    let mut st = feed.state.lock().expect("feed lock");
    // Loops that started or ended: their rows (working, idle).
    for t in st.running.symmetric_difference(&running) {
      if let Ok(id) = t.parse() {
        dirty.insert(id);
      }
    }
    // Calls that started or ended: to their watchers only.
    for (t, v) in &calls {
      if st.calls.get(t).unwrap_or(&Value::Null) != v {
        feed.send(Push::Call {
          thread: t.clone(),
          in_flight: v.clone(),
        });
      }
    }
    for t in st.calls.keys() {
      if !calls.contains_key(t) {
        feed.send(Push::Call {
          thread: t.clone(),
          in_flight: Value::Null,
        });
      }
    }
    st.running = running;
    st.calls = calls;
    if let (Some(before), Some(now)) = (st.cursor, cursor)
      && before != now
    {
      let index = s.state().index.lock().expect("index lock");
      let records = index.records_since(before[1]).unwrap_or_default();
      let segments = index.segment_threads_since(before[3]).unwrap_or_default();
      let added_segments = now[2] - before[2];
      // Rows only come; fewer than came since means some went.
      if now[0] - before[0] != records.len() as i64 || added_segments < 0 {
        reset = true;
      }
      for (id, kind) in records {
        match kind.as_str() {
          kinds::THREAD => {
            came = true;
            dirty.insert(id);
          }
          kinds::PLACE | kinds::PROJECT | kinds::FOLDER | crate::meta::READ_FLOOR => reset = true,
          _ => {
            if kind == kinds::DELETION {
              came = true;
            }
            // What it points at (a `set`'s target, a report's thread, a
            // question's), and that one's thread (an answer's question's).
            for e in index.edges_from(&id).unwrap_or_default() {
              let dst = e.dst.id;
              for t in index.edges_from(&dst).unwrap_or_default() {
                if t.rel == "thread" {
                  dirty.insert(t.dst.id);
                }
              }
              dirty.insert(dst);
            }
          }
        }
      }
      dirty.extend(segments);
      // Only threads, each as lists show its chain: its newest.
      dirty = dirty
        .into_iter()
        .filter(|id| {
          index
            .record(id)
            .ok()
            .flatten()
            .is_some_and(|r| r.kind == kinds::THREAD)
        })
        .map(|id| index.compaction_newest(&id))
        .collect();
    }
    if cursor.is_some() {
      st.cursor = cursor;
    }
    if moved.contains(&"threads") {
      st.loose_due = true;
    }
  }
  if reset {
    feed.send(Push::Reset);
  } else if !dirty.is_empty() {
    let ids: Vec<String> = dirty.iter().map(ToString::to_string).collect();
    let query = format!("all=1&every=1&limit={}&ids={}", ids.len(), ids.join(","));
    let rows = super::live::threads(s, None, &query)
      .ok()
      .and_then(|v| v["threads"].as_array().cloned())
      .unwrap_or_default();
    let shown: BTreeSet<&str> = rows.iter().filter_map(|r| r["id"].as_str()).collect();
    let remove: Vec<String> = ids
      .iter()
      .filter(|t| !shown.contains(t.as_str()))
      .cloned()
      .collect();
    // Threads came or went: the sections' counts again.
    let places = (came || !remove.is_empty())
      .then(|| super::live::threads(s, None, "all=1&every=1&limit=1").ok())
      .flatten()
      .map(|v| v["places"].clone());
    feed.send(Push::Rows {
      upsert: rows,
      remove,
      places,
    });
  }
  if moved.contains(&"attention") {
    feed.send(Push::Section {
      what: "attention",
      value: super::live::attention(s),
    });
  }
  if moved.contains(&"schedules") {
    feed.send(Push::Section {
      what: "schedules",
      value: super::live::schedules(s, ""),
    });
  }
  let loose = {
    let mut st = feed.state.lock().expect("feed lock");
    let due = st.loose_due && st.loose_at.is_none_or(|t| t.elapsed() >= LOOSE_EVERY);
    if due {
      st.loose_due = false;
      st.loose_at = Some(Instant::now());
    }
    due
  };
  if loose {
    feed.send(Push::Section {
      what: "loose-ends",
      value: crate::loose::live(s),
    });
  }
}
