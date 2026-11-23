//! Stitch's memory (design §2.9): engram over EmbeddingGemma 2, with the
//! policy edge talk settled on in the field.
//!
//! **One user, one graph, folded from facts.** Facts are what nodes sync:
//! an observation (a message's place, its int8 embedding and the flags its
//! writer decided), a credit (what was shown, what helped), a dismissal, a
//! sleep, a gist, a thread leaving or rejoining memory. Every node folds the
//! same facts in the same order (by record ID, then position) and arrives
//! at the same graph and the same log; a fact that arrives late, older than
//! what was folded, folds again from the newest checkpoint before it
//! (`Folding`), bit for bit what a fold from the start makes. Nothing
//! mutable syncs. Queries run on a copy of the
//! graph, so asking never changes what every node agrees on.
//!
//! **The log** is edge talk's `engram_log` folded in memory: each observed
//! handle's place, decision, novelty and target; supersession (a newer
//! near-duplicate carrying numbers is the current version of a mutable
//! fact); forgetting; how often each memory was shown, for the sleep audit.
//!
//! **Two arms.** The graph (engram) proposes by association; the cortex
//! (cosine over the stored embeddings) proposes by similarity and verifies.
//! A memory reaches the model only when it is close enough to the cue, so
//! an off-topic store honestly returns nothing.

mod encoder;
mod folding;
mod persist;
#[doc(hidden)]
pub mod synth;
mod words;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use engram::{Decision, Engram, IdentityStore};
use serde::Deserialize;

pub use encoder::{Encoder, ModelFiles};
pub use engram::heuristic_demand;
pub use folding::Folding;
pub use stitch_llama::available_memory;
pub use words::*;

/// The embedding engine and model, as fingerprinted in the center artifact.
pub const ENGINE: &str = "llama.cpp-b11559";
pub const MODEL: &str = "embeddinggemma-2-Q8_0";
/// EmbeddingGemma's prefixes: stored content, and recall cues.
pub const DOC_PREFIX: &str = "title: none | text: ";
pub const QUERY_PREFIX: &str = "task: search result | query: ";

// Tuning. edge talk calibrated every band below on its field data with
// EmbeddingGemma-300m; `examples/transfer.rs` re-embedded 7,324 of those
// messages with EmbeddingGemma 2 and moved each band to the same quantile
// of the new distribution (design §2.9 has the numbers).

/// Active bits per 2048-bit code (fly-hash k).
pub const K: usize = 128;
/// Strengthen (the payload is not kept) above this jaccard. Field: true
/// near-duplicates sit well above, topical cousins below; a false
/// Strengthen loses a memory for good, a false Link adds one, so it is set
/// high (edge talk 0.30 → 0.267).
pub const DUP_JACCARD: f32 = 0.267;
/// Link (same topic) above this (0.05 → 0.045).
pub const LINK_JACCARD: f32 = 0.045;
/// A recall needs an episode this close to anchor it (0.07 → 0.058).
pub const RECALL_ANCHOR: f32 = 0.058;
pub const RECALL_FLOOR: f32 = 0.045;
/// Passive surfacing threshold (0.18 → 0.164).
pub const SURFACE_THRESHOLD: f32 = 0.164;
/// Associative memories a deliberate recall asks the graph for.
pub const RECALL_TOP: usize = 4;
/// Cortex memories fused into a deliberate recall.
pub const FUSION_TOP: usize = 3;
/// Cosine distances (1 − cosine), query cue → stored content: fusion's
/// ceiling (0.60 → 0.327) and the relevance floor a deliberate recall's
/// memory, or one it is linked to, must reach (0.65 → 0.340).
pub const FUSION_MAX_DISTANCE: f32 = 0.327;
pub const DELIBERATE_FLOOR_DISTANCE: f32 = 0.340;
/// Cosine distances, stored message → stored content, for unasked recall,
/// adapted to the cue's novelty: an established topic is strict
/// (0.55 → 0.310), the middle is fusion's (0.60 → 0.322), new territory
/// allows one labeled distant association (0.70 → 0.374).
pub const AMBIENT_STRICT_DISTANCE: f32 = 0.310;
pub const AMBIENT_MIDDLE_DISTANCE: f32 = 0.322;
pub const AMBIENT_WANDER_DISTANCE: f32 = 0.374;
/// And the same pair centered (`center().doc_center` off both), which the
/// raw ceilings above do not see: raw, any two short chatty messages sit
/// 0.20–0.30 apart, inside every ceiling. On the first field threads, the
/// unasked recalls that were relevant came in at ≤ 0.458 centered and the
/// irrelevant ones at ≥ 0.528 (raw: ≤ 0.189 against ≥ 0.200), across cue
/// lengths: a note's memory must be within this too. Provisional (a
/// handful of threads): `stitch memory notes` gathers the outcomes to
/// re-measure it with (design §2.9).
pub const AMBIENT_CENTERED_DISTANCE: f32 = 0.49;
/// Cue novelty at or below this: a dup-band match exists (established); at
/// or above the other: nothing even link-band close (new territory).
pub const ESTABLISHED_NOVELTY: f32 = 1.0 - DUP_JACCARD;
pub const NOVEL_NOVELTY: f32 = 1.0 - LINK_JACCARD;
/// Cortex memories proposed for an unasked recall, before the filters.
pub const PROPOSER_TOP: usize = 5;
/// Memories in one unasked note, at most; handles parked between turns.
pub const AMBIENT_BUDGET: usize = 4;
pub const AMBIENT_BUFFER: usize = 8;
/// Words quoted from a memory in an unasked note; of a paired answer.
pub const QUOTE_WORDS: usize = 120;
pub const ANSWER_WORDS: usize = 80;
/// A same-thread Strengthen is a real duplicate only above this token
/// overlap (a mutable fact ≈ 1.0, a sibling reply ≈ 0.0–0.3).
pub const SAME_THREAD_DUP_OVERLAP: f32 = 0.5;
/// Strength kept by a dismissed or freeloading memory.
pub const PENALIZE_FACTOR: f32 = 0.5;
/// Sleep audit: shown this often and never helped; or, having helped once,
/// shown this often in the last fourteen days.
pub const PENALIZE_INJECTIONS: usize = 3;
pub const PENALIZE_USED_INJECTIONS: usize = 8;
const PENALIZE_WINDOW_SECS: i64 = 14 * 86_400;
/// Gists: a cluster this large, quiet this long, condensed from at most
/// this many of its strongest memories.
pub const GIST_MIN_MEMBERS: usize = 5;
pub const GIST_QUIET_DAYS: i64 = 7;
pub const GIST_MAX_FRAGMENTS: usize = 12;

/// The frozen centering vectors for this model (engram-fit's artifact).
#[derive(Debug, Deserialize)]
pub struct Center {
  pub doc_center: Vec<f32>,
  pub query_center: Vec<f32>,
  pub tag_u64: u64,
  pub dim: usize,
}

/// The center artifact Stitch runs with.
pub fn center() -> &'static Center {
  static C: std::sync::OnceLock<Center> = std::sync::OnceLock::new();
  C.get_or_init(|| {
    serde_json::from_str(include_str!("../assets/embeddinggemma-2.center.json"))
      .expect("the bundled center artifact parses")
  })
}

/// An embedding as stored: int8, unit scale (768 bytes for EmbeddingGemma).
pub fn pack(v: &[f32]) -> Vec<i8> {
  v.iter()
    .map(|x| (x.clamp(-1.0, 1.0) * 127.0).round() as i8)
    .collect()
}

pub fn unpack(v: &[i8]) -> Vec<f32> {
  v.iter().map(|&x| f32::from(x) / 127.0).collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// An embedding as record text: its int8 bytes in base64.
pub fn to_base64(v: &[i8]) -> String {
  let bytes: Vec<u8> = v.iter().map(|&x| x as u8).collect();
  let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
  for chunk in bytes.chunks(3) {
    let n = chunk
      .iter()
      .enumerate()
      .fold(0u32, |n, (i, &b)| n | (u32::from(b) << (16 - 8 * i)));
    for i in 0..4 {
      if i <= chunk.len() {
        out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
      } else {
        out.push('=');
      }
    }
  }
  out
}

/// The embedding back from record text.
pub fn from_base64(s: &str) -> Option<Vec<i8>> {
  let mut bits = 0u32;
  let mut n = 0;
  let mut out = Vec::with_capacity(s.len() / 4 * 3);
  for c in s.bytes().filter(|&c| c != b'=') {
    let v = B64.iter().position(|&x| x == c)? as u32;
    bits = (bits << 6) | v;
    n += 6;
    if n >= 8 {
      n -= 8;
      out.push((bits >> n) as u8 as i8);
    }
  }
  Some(out)
}

/// A message's stable handle on every node: a hash of where it is
/// (`repo:thread#msg`, `:aN` for its Nth part when that is an image).
pub fn handle(place: &str) -> u64 {
  // FNV-1a, as engram's own tags: tiny, fixed, the same everywhere.
  place.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
  })
}

/// The thread a place is in (its ID, whichever repository holds it now);
/// `None` for a gist.
pub fn thread_of(place: &str) -> Option<&str> {
  let (head, _) = place.split_once('#')?;
  Some(head.rsplit_once(':').map_or(head, |(_, t)| t))
}

/// A gist's place, by its handle.
pub fn gist_place(h: u64) -> String {
  format!("gist:{h:016x}")
}

/// Something memory is told: what every node folds.
#[derive(Clone, Debug, PartialEq)]
pub enum Fact {
  /// A message (or an image in one) was seen.
  Observe {
    place: String,
    embedding: Vec<i8>,
    /// How much the moment wanted memory (arms knowledge gaps).
    demand: f32,
    /// A Strengthen into a sibling in the same thread is wrong: keep this
    /// one as its own memory (the writer checked the words).
    own: bool,
    /// A Strengthen of this is a newer version of the memory it
    /// strengthens (it carries numbers): recalling that one shows this.
    supersedes: bool,
  },
  /// Memories shown in an exchange, and those its answer used.
  Credit { shown: Vec<u64>, used: Vec<u64> },
  /// Shown memories the answer dismissed.
  Penalize { handles: Vec<u64> },
  /// Consolidation: the audit's knocks, then decay and prune.
  Sleep,
  /// A quiet cluster condensed: the gist and its embedding, the members
  /// it replaces and the ones dropped as chaff.
  Gist {
    text: String,
    embedding: Vec<i8>,
    kept: Vec<u64>,
    dropped: Vec<u64>,
  },
  /// A thread (by ID) leaves memory, or comes back. Out of memory
  /// (`on` false), `recall` keeps its own searches of memory: nothing of
  /// it is remembered, but it may recall what others left (a spawned
  /// thread's default). In memory, it recalls anyway.
  Remember {
    thread: String,
    on: bool,
    recall: bool,
  },
}

/// Where a fact is in the total order (its record, then its position),
/// and when it was written (the record's time, in seconds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct At {
  pub record: String,
  pub pos: u32,
  pub secs: i64,
}

impl At {
  pub fn new(record: impl Into<String>, pos: u32, secs: i64) -> At {
    At {
      record: record.into(),
      pos,
      secs,
    }
  }

  fn key(&self) -> (&str, u32) {
    (&self.record, self.pos)
  }
}

/// What the gate did with an observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
  New,
  Link,
  Strengthen,
  Skip,
  Gist,
}

impl Kind {
  pub fn as_str(self) -> &'static str {
    match self {
      Kind::New => "new",
      Kind::Link => "link",
      Kind::Strengthen => "strengthen",
      Kind::Skip => "skip",
      Kind::Gist => "gist",
    }
  }
}

/// One handle in the log.
#[derive(Clone, Debug)]
pub struct Entry {
  pub place: String,
  pub kind: Kind,
  pub novelty: f32,
  /// The episode linked to or strengthened.
  pub target: Option<u64>,
  /// A newer version that recall shows instead.
  pub resolved: Option<u64>,
  pub forgotten: bool,
  /// When it was observed.
  pub secs: i64,
}

/// What folding an observation did.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
  pub handle: u64,
  pub kind: Kind,
  pub novelty: f32,
  pub target: Option<u64>,
  /// What the passive channel surfaced: memories this one reminded of.
  pub surfaced: Vec<u64>,
  pub gap: bool,
}

/// A recalled memory.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
  pub handle: u64,
  /// Cosine to the cue.
  pub cosine: f32,
  /// Proposed by association (the graph), not only by similarity.
  pub associative: bool,
}

/// An episode's state, for a note's diagnostics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
  pub strength: f32,
  pub count: u32,
  pub used: bool,
}

type Mem = Engram<IdentityStore<u64>>;

fn new_mem() -> Mem {
  let mut mem = Engram::new(IdentityStore::new());
  configure(&mut mem);
  mem
}

/// Stitch's tuning on an engram (a restored snapshot carries only its
/// learned thresholds).
fn configure(mem: &mut Mem) {
  mem.params.k = K;
  mem.params.recall_anchor = RECALL_ANCHOR;
  mem.params.recall_floor = RECALL_FLOOR;
  mem.params.surface_threshold = SURFACE_THRESHOLD;
  mem.gate.dup_jaccard = DUP_JACCARD;
  mem.gate.link_jaccard = LINK_JACCARD;
}

/// What a checkpoint of the fold depends on besides the facts: this
/// tuning, the encoder, and the fold's own format. A checkpoint written
/// under another is discarded.
pub fn tuning_tag() -> u64 {
  let t = format!(
    "fold-2|{K}|{DUP_JACCARD}|{LINK_JACCARD}|{RECALL_ANCHOR}|{RECALL_FLOOR}|{SURFACE_THRESHOLD}|\
     {SAME_THREAD_DUP_OVERLAP}|{PENALIZE_FACTOR}|{PENALIZE_INJECTIONS}|{PENALIZE_USED_INJECTIONS}|\
     {PENALIZE_WINDOW_SECS}|{:016x}|{}",
    center().tag_u64,
    center().dim,
  );
  persist::digest(t.as_bytes())
}

fn centered(raw: &[f32], c: &[f32]) -> Vec<f32> {
  let mut v = raw.to_vec();
  engram::center_in_place(&mut v, c);
  v
}

/// How an observation's decision reads, with its target payload.
fn decision_of(mem: &Mem, d: Decision) -> (Kind, Option<u64>) {
  let payload = |e| mem.graph().get(e).map(|ep| ep.payload);
  match d {
    Decision::WriteNew => (Kind::New, None),
    Decision::Link(e) => (Kind::Link, payload(e)),
    Decision::Strengthen(e) => (Kind::Strengthen, payload(e)),
    Decision::Skip => (Kind::Skip, None),
  }
}

/// One observation through the graph (`own`: keep it as its own memory
/// when the gate strengthens).
fn observe_in(mem: &mut Mem, raw: &[f32], h: u64, demand: f32, own: bool) -> Applied {
  let doc = centered(raw, &center().doc_center);
  let obs = mem.observe(&doc, h, demand);
  let (mut kind, target) = decision_of(mem, obs.decision);
  if kind == Kind::Strengthen && own {
    mem.upsert_episode(&doc, h, 1.0, 1, false);
    kind = Kind::Link;
  }
  Applied {
    handle: h,
    kind,
    novelty: obs.novelty,
    target,
    surfaced: obs
      .surfaced
      .into_iter()
      .map(|s| s.payload)
      .filter(|&p| p != h)
      .collect(),
    gap: obs.knowledge_gap,
  }
}

fn cosine_i8(a: &[f32], b: &[i8]) -> f32 {
  let (mut d, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
  for (x, &y) in a.iter().zip(b) {
    let y = f32::from(y) / 127.0;
    d += x * y;
    na += x * x;
    nb += y * y;
  }
  d / (na * nb).sqrt().max(1e-9)
}

/// The note ceiling for a cue's novelty: the distance a memory must be
/// within, and whether this is new territory (one labeled leap at most).
pub fn ambient_ceiling(novelty: f32) -> (f32, bool) {
  if novelty <= ESTABLISHED_NOVELTY {
    (AMBIENT_STRICT_DISTANCE, false)
  } else if novelty >= NOVEL_NOVELTY {
    (AMBIENT_WANDER_DISTANCE, true)
  } else {
    (AMBIENT_MIDDLE_DISTANCE, false)
  }
}

/// The folded memory.
///
/// **What costs follow** (design §2.9, "the live set"): the cortex holds
/// only remembered memories. A memory forgotten (by a sleep, a gist's
/// DROP) leaves it, and with it what merged into it: the Strengthens into
/// its episode, a gist's kept members, so the cortex follows the living
/// episodes, not all history. A memory of a thread out of memory waits
/// aside (`parked`), never scanned for proposals, until the thread comes
/// back. The log keeps every handle (a place observed again counts once).
#[derive(Clone)]
pub struct Fold {
  mem: Mem,
  /// Stored int8 embeddings of remembered memories eligible somewhere (not
  /// forgotten, their current place in a thread in memory): the cortex.
  vectors: HashMap<u64, Vec<i8>>,
  /// The embeddings of remembered memories whose current place is in a
  /// thread out of memory.
  parked: HashMap<u64, Vec<i8>>,
  /// What merged into a memory, forgotten with it: the Strengthens into
  /// its episode, a gist's kept members.
  merged: HashMap<u64, Vec<u64>>,
  /// Remembered memories by the thread their current place is in.
  by_thread: HashMap<String, BTreeSet<u64>>,
  log: HashMap<u64, Entry>,
  /// When each memory was shown (seconds), for the sleep audit.
  shown: HashMap<u64, Vec<i64>>,
  /// Threads out of memory.
  off: BTreeSet<String>,
  /// Threads whose memory was ever switched (on or off), by anyone.
  switched: BTreeSet<String>,
  /// Threads out of memory that still recall (`Remember`'s `recall`).
  recalling: BTreeSet<String>,
  /// Gists' text by handle.
  gists: HashMap<u64, String>,
  last: Option<(String, u32)>,
  folded: usize,
}

impl Default for Fold {
  fn default() -> Fold {
    Fold::new()
  }
}

impl Fold {
  pub fn new() -> Fold {
    Fold {
      mem: new_mem(),
      vectors: HashMap::new(),
      parked: HashMap::new(),
      merged: HashMap::new(),
      by_thread: HashMap::new(),
      log: HashMap::new(),
      shown: HashMap::new(),
      off: BTreeSet::new(),
      switched: BTreeSet::new(),
      recalling: BTreeSet::new(),
      gists: HashMap::new(),
      last: None,
      folded: 0,
    }
  }

  /// Facts folded so far.
  pub fn len(&self) -> usize {
    self.folded
  }

  pub fn is_empty(&self) -> bool {
    self.folded == 0
  }

  /// Episodes in the graph.
  pub fn episodes(&self) -> usize {
    self.mem.graph().len()
  }

  /// Embeddings the cortex scans, and handles in the log.
  pub fn sizes(&self) -> (usize, usize) {
    (self.vectors.len(), self.log.len())
  }

  /// Whether a fact at `at` can be folded on top (it comes after what was
  /// folded); otherwise the caller rebuilds from the start.
  pub fn follows(&self, at: &At) -> bool {
    self
      .last
      .as_ref()
      .is_none_or(|(r, p)| at.key() > (r.as_str(), *p))
  }

  /// The record of the last fact folded.
  pub fn last_record(&self) -> Option<&str> {
    self.last.as_ref().map(|(r, _)| r.as_str())
  }

  /// Whether the handle has been observed.
  pub fn knows(&self, handle: u64) -> bool {
    self.log.contains_key(&handle)
  }

  /// The log entry of a handle.
  pub fn entry(&self, handle: u64) -> Option<&Entry> {
    self.log.get(&handle)
  }

  /// A gist's text.
  pub fn gist(&self, handle: u64) -> Option<&str> {
    self.gists.get(&handle).map(String::as_str)
  }

  /// Whether a thread (by ID) is out of memory.
  pub fn is_off(&self, thread: &str) -> bool {
    self.off.contains(thread)
  }

  /// Whether a thread may search memory itself: in memory, or out of it
  /// with `recall` kept.
  pub fn may_recall(&self, thread: &str) -> bool {
    !self.off.contains(thread) || self.recalling.contains(thread)
  }

  /// Whether a thread's memory was ever switched, on or off: a place's
  /// default no longer applies to it.
  pub fn is_switched(&self, thread: &str) -> bool {
    self.switched.contains(thread)
  }

  /// Folds one fact at `at` (after everything folded); for an
  /// observation, what it did.
  pub fn apply(&mut self, at: &At, fact: &Fact) -> Option<Applied> {
    debug_assert!(self.follows(at), "facts fold in order");
    self.last = Some((at.record.clone(), at.pos));
    self.folded += 1;
    match fact {
      Fact::Observe {
        place,
        embedding,
        demand,
        own,
        supersedes,
      } => {
        let h = handle(place);
        // Observed twice (two nodes healing the same hole): once counts.
        if self.log.contains_key(&h) || embedding.len() != center().dim {
          return None;
        }
        let a = observe_in(&mut self.mem, &unpack(embedding), h, *demand, *own);
        self.log.insert(
          h,
          Entry {
            place: place.clone(),
            kind: a.kind,
            novelty: a.novelty,
            target: a.target,
            resolved: None,
            forgotten: false,
            secs: at.secs,
          },
        );
        self.remember(h, embedding.clone());
        if a.kind == Kind::Strengthen
          && let Some(t) = a.target
          && self.log.contains_key(&t)
        {
          self.merged.entry(t).or_default().push(h);
          if *supersedes {
            self.set_resolved(t, Some(h));
          }
        }
        Some(a)
      }
      Fact::Credit { shown, used } => {
        for h in shown {
          self.shown.entry(*h).or_default().push(at.secs);
        }
        if !used.is_empty() {
          self.mem.credit(used, true);
        }
        None
      }
      Fact::Penalize { handles } => {
        for h in handles {
          self.mem.penalize_episode(*h, PENALIZE_FACTOR);
        }
        None
      }
      Fact::Sleep => {
        // The audit first, so a knock decays and prunes in this cycle.
        for h in self.freeloaders(at.secs) {
          self.mem.penalize_episode(h, PENALIZE_FACTOR);
        }
        let report = self.mem.sleep();
        for h in report.forgotten {
          self.forget(h);
        }
        self.settle();
        None
      }
      Fact::Gist {
        text,
        embedding,
        kept,
        dropped,
      } => {
        if embedding.len() != center().dim {
          return None;
        }
        let gh = handle(&format!("gist:{text}"));
        // The gist inherits what its kept fragments earned.
        let (mut strength, mut count, mut used) = (1.0f32, 0u32, false);
        for h in kept.iter().filter(|&&h| h != gh) {
          if let Some(s) = self.stats(*h) {
            strength = strength.max(s.strength);
            count += s.count;
            used |= s.used;
          }
        }
        let doc = centered(&unpack(embedding), &center().doc_center);
        self
          .mem
          .upsert_episode(&doc, gh, strength, count.max(1), used);
        self.gists.insert(gh, text.clone());
        self.log.insert(
          gh,
          Entry {
            place: gist_place(gh),
            kind: Kind::Gist,
            novelty: 0.0,
            target: None,
            resolved: None,
            forgotten: false,
            secs: at.secs,
          },
        );
        self.remember(gh, embedding.clone());
        // Kept members show the gist, and go when it is forgotten.
        for h in kept.iter().filter(|&&h| h != gh) {
          if self.log.contains_key(h) {
            self.set_resolved(*h, Some(gh));
            self.merged.entry(gh).or_default().push(*h);
          }
          self.mem.remove_episode(*h);
        }
        for h in dropped.iter().filter(|&&h| h != gh) {
          self.forget(*h);
          self.mem.remove_episode(*h);
        }
        None
      }
      Fact::Remember { thread, on, recall } => {
        self.switched.insert(thread.clone());
        if *on {
          self.off.remove(thread);
        } else {
          self.off.insert(thread.clone());
        }
        if *recall && !*on {
          self.recalling.insert(thread.clone());
        } else {
          self.recalling.remove(thread);
        }
        // Its memories wait aside while it is out, and come back with it.
        let handles: Vec<u64> = self
          .by_thread
          .get(thread)
          .map(|s| s.iter().copied().collect())
          .unwrap_or_default();
        for h in handles {
          self.place(h);
        }
        None
      }
    }
  }

  /// After a sleep, the graph passes through engram's own snapshot, so
  /// that a checkpoint (`Folding`) is exactly what the fold goes on from:
  /// the snapshot keeps neither the recently written episodes Hebbian
  /// binding links the next one to, nor the order free slots are reused
  /// in; through it, every fold starts each day from what a snapshot
  /// holds. And the flush material engram keeps for hosts that write rows
  /// (Stitch writes none) is dropped, or it would grow with all history.
  fn settle(&mut self) {
    self.mem.drain_dirty();
    let tag = center().tag_u64;
    let mut mem = Mem::restore(&self.mem.snapshot(tag), tag, IdentityStore::new())
      .expect("engram restores its own snapshot");
    configure(&mut mem);
    self.mem = mem;
  }

  /// The thread a remembered memory's current place is in.
  fn thread_now(&self, h: u64) -> Option<String> {
    self.resolve(h).and_then(thread_of).map(str::to_owned)
  }

  /// A memory's embedding, newly observed: in the cortex, or aside when
  /// its thread is out of memory.
  fn remember(&mut self, h: u64, embedding: Vec<i8>) {
    self.parked.remove(&h);
    self.vectors.insert(h, embedding);
    if let Some(t) = self.thread_now(h) {
      self.by_thread.entry(t).or_default().insert(h);
    }
    self.place(h);
  }

  /// Puts a memory's embedding where it belongs now: the cortex, aside, or
  /// nowhere once forgotten.
  fn place(&mut self, h: u64) {
    let Some(e) = self.log.get(&h) else {
      return;
    };
    if e.forgotten {
      self.vectors.remove(&h);
      self.parked.remove(&h);
      return;
    }
    let off = self.thread_now(h).is_some_and(|t| self.off.contains(&t));
    let (from, to) = if off {
      (&mut self.vectors, &mut self.parked)
    } else {
      (&mut self.parked, &mut self.vectors)
    };
    if let Some(v) = from.remove(&h) {
      to.insert(h, v);
    }
  }

  /// Where recalling a memory leads now (a newer version, a gist).
  fn set_resolved(&mut self, h: u64, to: Option<u64>) {
    let before = self.thread_now(h);
    let Some(e) = self.log.get_mut(&h) else {
      return;
    };
    e.resolved = to;
    if e.forgotten {
      return;
    }
    let after = self.thread_now(h);
    if before != after {
      if let Some(t) = before
        && let Some(s) = self.by_thread.get_mut(&t)
      {
        s.remove(&h);
        if s.is_empty() {
          self.by_thread.remove(&t);
        }
      }
      if let Some(t) = after {
        self.by_thread.entry(t).or_default().insert(h);
      }
    }
    self.place(h);
  }

  /// Forgets a memory, and what merged into it.
  fn forget(&mut self, h: u64) {
    let mut todo = vec![h];
    while let Some(h) = todo.pop() {
      let before = self.thread_now(h);
      let Some(e) = self.log.get_mut(&h) else {
        continue;
      };
      if e.forgotten {
        continue;
      }
      e.forgotten = true;
      e.resolved = None;
      if let Some(t) = before
        && let Some(s) = self.by_thread.get_mut(&t)
      {
        s.remove(&h);
        if s.is_empty() {
          self.by_thread.remove(&t);
        }
      }
      self.vectors.remove(&h);
      self.parked.remove(&h);
      self.shown.remove(&h);
      if let Some(m) = self.merged.remove(&h) {
        todo.extend(m);
      }
    }
  }

  /// Folds `facts` from nothing, in order.
  pub fn rebuild(facts: impl IntoIterator<Item = (At, Fact)>) -> Fold {
    let mut all: Vec<(At, Fact)> = facts.into_iter().collect();
    all.sort_by(|a, b| a.0.key().cmp(&b.0.key()));
    let mut fold = Fold::new();
    for (at, f) in &all {
      fold.apply(at, f);
    }
    fold
  }

  /// What observing these (place, raw embedding, demand), in order, would
  /// do, without changing anything: `own` decides, for each Strengthen,
  /// whether it keeps its own memory instead.
  pub fn probe(
    &self,
    items: &[(String, Vec<f32>, f32)],
    mut own: impl FnMut(&Applied) -> bool,
  ) -> Vec<(Applied, bool)> {
    let mut mem = self.mem.clone();
    items
      .iter()
      .map(|(place, raw, demand)| {
        let h = handle(place);
        let mut trial = mem.clone();
        let a = observe_in(&mut trial, raw, h, *demand, false);
        if a.kind == Kind::Strengthen && own(&a) {
          (observe_in(&mut mem, raw, h, *demand, true), true)
        } else {
          mem = trial;
          (a, false)
        }
      })
      .collect()
  }

  /// What a cue (a message about to be sent, as it will be stored) would
  /// do to the graph: its novelty, what it surfaces, whether it arms a
  /// knowledge gap. Nothing changes.
  pub fn peek(&self, place: &str, raw: &[f32], demand: f32) -> Applied {
    let mut mem = self.mem.clone();
    observe_in(&mut mem, raw, handle(place), demand, false)
  }

  /// Where a handle's memory is now: its place, or the newer version's;
  /// `None` when forgotten.
  pub fn resolve(&self, h: u64) -> Option<&str> {
    let e = self.log.get(&h)?;
    if e.forgotten {
      return None;
    }
    match e.resolved.and_then(|r| self.log.get(&r)) {
      Some(newer) => Some(&newer.place),
      None => Some(&e.place),
    }
  }

  /// Whether a memory may be recalled into `thread`: not forgotten, its
  /// (current) place neither in that thread nor in one out of memory.
  pub fn eligible(&self, h: u64, thread: Option<&str>) -> bool {
    self.resolve(h).is_some_and(|p| match thread_of(p) {
      Some(t) => Some(t) != thread && !self.off.contains(t),
      None => true,
    })
  }

  /// An episode's state, when the handle has one.
  pub fn stats(&self, h: u64) -> Option<Stats> {
    let g = self.mem.graph();
    g.episode_of(&h).and_then(|e| g.get(e)).map(|e| Stats {
      strength: e.strength,
      count: e.count,
      used: e.used,
    })
  }

  /// Cosine distance from a cue to a memory's stored embedding.
  pub fn distance(&self, cue: &[f32], h: u64) -> Option<f32> {
    self.vector(h).map(|v| 1.0 - cosine_i8(cue, v))
  }

  /// A remembered memory's stored embedding (in the cortex or aside).
  fn vector(&self, h: u64) -> Option<&Vec<i8>> {
    self.vectors.get(&h).or_else(|| self.parked.get(&h))
  }

  /// `distance` with both sides centered (see `AMBIENT_CENTERED_DISTANCE`).
  pub fn centered_distance(&self, cue: &[f32], h: u64) -> Option<f32> {
    let c = &center().doc_center;
    let v = self.vector(h)?;
    if cue.len() != c.len() || v.len() != c.len() {
      return None;
    }
    let (mut d, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for ((x, &y), m) in cue.iter().zip(v).zip(c) {
      let (x, y) = (x - m, f32::from(y) / 127.0 - m);
      d += x * y;
      na += x * x;
      nb += y * y;
    }
    Some(1.0 - d / (na * nb).sqrt().max(1e-9))
  }

  /// The closest memories to `cue` within `max_distance` that `keep`
  /// allows, filtered before the limit; ties broken by handle. Only
  /// memories eligible somewhere are scanned (not forgotten, not of a
  /// thread out of memory): `keep` decides among them.
  pub fn cortex(
    &self,
    cue: &[f32],
    max_distance: f32,
    top: usize,
    keep: &dyn Fn(u64) -> bool,
  ) -> Vec<(u64, f32)> {
    let mut out: Vec<(u64, f32)> = self
      .vectors
      .iter()
      .filter(|(h, _)| keep(**h))
      .map(|(h, v)| (*h, cosine_i8(cue, v)))
      .filter(|(_, c)| 1.0 - c <= max_distance)
      .collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    out.truncate(top);
    out
  }

  /// A deliberate recall of `query` (a raw, int8 round-tripped query
  /// embedding): the graph's associations at full demand, fused with the
  /// closest stored content; a memory survives only when it, or one it is
  /// linked to, is within the relevance floor. `keep` hides handles (the
  /// current thread's, those out of memory). The graph is not touched.
  pub fn recall(&self, query: &[f32], keep: &dyn Fn(u64) -> bool) -> Vec<Hit> {
    if query.len() != center().dim || (self.vectors.is_empty() && self.parked.is_empty()) {
      return Vec::new();
    }
    let q = centered(query, &center().query_center);
    let mut copy = self.mem.clone();
    let mut handles: Vec<(u64, bool)> = copy
      .recall_with_demand(&q, 1.0, RECALL_TOP)
      .into_iter()
      .filter(|h| keep(*h))
      .map(|h| (h, true))
      .collect();
    for (h, _) in self.cortex(query, FUSION_MAX_DISTANCE, FUSION_TOP, keep) {
      if !handles.iter().any(|(x, _)| *x == h) {
        handles.push((h, false));
      }
    }
    // The relevance floor: associative leaps ride on a relevant seed, any
    // remembered memory (one aside too).
    let near: BTreeSet<u64> = self
      .vectors
      .iter()
      .chain(&self.parked)
      .filter(|(h, v)| {
        self.log.get(h).is_some_and(|e| !e.forgotten)
          && 1.0 - cosine_i8(query, v) <= DELIBERATE_FLOOR_DISTANCE
      })
      .map(|(h, _)| *h)
      .collect();
    let relation = |h: u64| {
      self
        .log
        .get(&h)
        .filter(|e| matches!(e.kind, Kind::Link | Kind::Strengthen))
        .and_then(|e| e.target)
    };
    // Linked: its own target is near, or a near memory's target is it.
    let linked = |c: u64| {
      relation(c).is_some_and(|t| near.contains(&t)) || near.iter().any(|&h| relation(h) == Some(c))
    };
    handles
      .into_iter()
      .filter(|(h, _)| near.contains(h) || linked(*h))
      .map(|(h, associative)| Hit {
        handle: h,
        cosine: self.distance(query, h).map_or(0.0, |d| 1.0 - d),
        associative,
      })
      .collect()
  }

  /// Memories the sleep audit knocks: living episodes shown at least
  /// `PENALIZE_INJECTIONS` times without ever helping, or, having helped,
  /// shown `PENALIZE_USED_INJECTIONS` times in the last fourteen days.
  fn freeloaders(&self, now: i64) -> Vec<u64> {
    let mut out: Vec<u64> = self
      .shown
      .iter()
      .filter_map(|(h, times)| {
        let s = self.stats(*h)?;
        let recent = times
          .iter()
          .filter(|&&t| t >= now - PENALIZE_WINDOW_SECS)
          .count();
        let bad = if s.used {
          recent >= PENALIZE_USED_INJECTIONS
        } else {
          times.len() >= PENALIZE_INJECTIONS
        };
        bad.then_some(*h)
      })
      .collect();
    out.sort_unstable();
    out
  }

  /// The quiet cluster due for a gist at `now`: connected components over
  /// link and strengthen relations between living episodes, the largest
  /// with at least `GIST_MIN_MEMBERS` whose newest memory is a week old;
  /// its strongest members (at most `GIST_MAX_FRAGMENTS`), none from a
  /// thread out of memory. Empty when none is due.
  pub fn gist_due(&self, now: i64) -> Vec<u64> {
    let alive = |h: u64| self.mem.graph().episode_of(&h).is_some();
    let mut adj: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    // Over the living episodes, not the whole log.
    for (_, ep) in self.mem.graph().iter() {
      let h = ep.payload;
      if let Some(e) = self.log.get(&h)
        && let Some(t) = e.target
        && matches!(e.kind, Kind::Link | Kind::Strengthen)
        && h != t
        && alive(t)
      {
        adj.entry(h).or_default().insert(t);
        adj.entry(t).or_default().insert(h);
      }
    }
    let mut seen: BTreeSet<u64> = BTreeSet::new();
    let mut best: Option<(usize, u64, Vec<u64>)> = None;
    for &start in adj.keys() {
      if !seen.insert(start) {
        continue;
      }
      let mut comp = vec![start];
      let mut i = 0;
      while i < comp.len() {
        for &n in &adj[&comp[i]] {
          if seen.insert(n) {
            comp.push(n);
          }
        }
        i += 1;
      }
      let root = *comp.iter().min().expect("non-empty");
      let newest = comp
        .iter()
        .filter_map(|h| {
          let e = self.log.get(h)?;
          let current = e.resolved.and_then(|r| self.log.get(&r)).unwrap_or(e);
          Some(current.secs)
        })
        .max()
        .unwrap_or(now);
      if comp.len() < GIST_MIN_MEMBERS || now - newest < GIST_QUIET_DAYS * 86_400 {
        continue;
      }
      if best
        .as_ref()
        .is_none_or(|(n, r, _)| comp.len() > *n || (comp.len() == *n && root < *r))
      {
        best = Some((comp.len(), root, comp));
      }
    }
    let Some((_, _, comp)) = best else {
      return Vec::new();
    };
    let mut members: Vec<(u64, f32)> = comp
      .into_iter()
      .filter(|h| {
        self
          .resolve(*h)
          .and_then(thread_of)
          .is_none_or(|t| !self.off.contains(t))
      })
      .map(|h| (h, self.stats(h).map_or(1.0, |s| s.strength)))
      .collect();
    members.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    members.truncate(GIST_MAX_FRAGMENTS);
    if members.len() < GIST_MIN_MEMBERS {
      return Vec::new();
    }
    members.into_iter().map(|(h, _)| h).collect()
  }

  /// Decisions by novelty band, for observations since `since`: the drift
  /// check to run after any threshold or encoder change. Bands: up to the
  /// dup jaccard's complement read as novelty 0.267, to established, to
  /// novel, beyond.
  pub fn bands(&self, since: i64) -> BTreeMap<Kind, [usize; 4]> {
    let mut out: BTreeMap<Kind, [usize; 4]> = BTreeMap::new();
    for e in self.log.values().filter(|e| e.secs >= since) {
      let b = if e.novelty <= DUP_JACCARD {
        0
      } else if e.novelty <= ESTABLISHED_NOVELTY {
        1
      } else if e.novelty < NOVEL_NOVELTY {
        2
      } else {
        3
      };
      out.entry(e.kind).or_default()[b] += 1;
    }
    out
  }

  /// Living episodes' strengths, and how many memories are forgotten.
  pub fn strengths(&self) -> (Vec<f32>, usize) {
    let s = self.mem.graph().iter().map(|(_, e)| e.strength).collect();
    (s, self.log.values().filter(|e| e.forgotten).count())
  }

  /// How often a memory was shown.
  pub fn shown(&self, h: u64) -> usize {
    self.shown.get(&h).map_or(0, Vec::len)
  }
}

#[cfg(test)]
mod tests;
