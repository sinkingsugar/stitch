//! Synthetic memory for tests and the fold benchmark (`examples/foldbench.rs`):
//! deterministic vectors shaped like the encoder's, and a history of facts
//! shaped like the field's (design §2.9, "As built": observations in
//! threads on drifting topics, Strengthens with and without supersession,
//! feedback, a sleep every 250 facts, gists when a cluster is due, threads
//! leaving and rejoining memory). Not part of the API.

use std::collections::VecDeque;

use crate::{At, Fact, Fold, Kind, center, handle, pack, unpack};

pub fn splitmix(mut x: u64) -> u64 {
  x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
  x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
  x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
  x ^ (x >> 31)
}

/// A deterministic unit vector shaped like the model's: its common
/// component (the doc center) plus a direction of its own, scaled so two
/// unrelated vectors sit near cosine 0.55, as real unrelated messages do.
pub fn vector(seed: u64) -> Vec<f32> {
  let c = &center().doc_center;
  let c2: f32 = c.iter().map(|x| x * x).sum();
  let scale = (c2 * (1.0 / 0.55 - 1.0)).sqrt();
  let mut r: Vec<f32> = (0..center().dim as u64)
    .map(|i| {
      (splitmix(seed.wrapping_mul(1_000_003).wrapping_add(i)) >> 11) as f32 / (1u64 << 53) as f32
        - 0.5
    })
    .collect();
  stitch_llama::normalize(&mut r);
  let mut v: Vec<f32> = c.iter().zip(&r).map(|(a, b)| a + scale * b).collect();
  stitch_llama::normalize(&mut v);
  unpack(&pack(&v))
}

/// `base` moved toward another vector by `t` (0: itself, 1: the other):
/// a paraphrase when small.
pub fn near(base: &[f32], seed: u64, t: f32) -> Vec<f32> {
  let other = vector(seed);
  let mut v: Vec<f32> = base
    .iter()
    .zip(&other)
    .map(|(a, b)| (1.0 - t) * a + t * b)
    .collect();
  stitch_llama::normalize(&mut v);
  unpack(&pack(&v))
}

/// A small deterministic generator.
struct Rng(u64);

impl Rng {
  fn next(&mut self) -> u64 {
    self.0 = splitmix(self.0);
    self.0
  }
  /// Uniform in [0, 1).
  fn unit(&mut self) -> f32 {
    (self.next() >> 40) as f32 / (1u64 << 24) as f32
  }
  fn below(&mut self, n: usize) -> usize {
    (self.next() % n.max(1) as u64) as usize
  }
}

/// Seconds between facts: a sleep (every twenty hours) per 250 facts.
pub const SECS_PER_FACT: i64 = 72_000 / 250;
/// Facts between sleeps.
pub const SLEEP_EVERY: usize = 250;

/// The record ID of the `i`th record (sortable, as record IDs are).
pub fn record(i: usize) -> String {
  format!("r{i:010}")
}

/// `n` facts (about), in order, shaped like the field's, and the fold they
/// make. Each exchange is one record (the user's message and the reply,
/// sometimes the feedback of a note); a sleep and a gist are records of
/// their own.
pub fn history(n: usize, seed: u64) -> (Vec<(At, Fact)>, Fold) {
  let mut rng = Rng(seed);
  let mut fold = Fold::new();
  let mut out: Vec<(At, Fact)> = Vec::with_capacity(n + 8);
  // Live topics: each a base vector; new ones drift in.
  let mut topics: Vec<Vec<f32>> = (0..40).map(|i| vector(seed ^ (i + 1) << 20)).collect();
  let mut next_topic = 40u64;
  let mut recent: VecDeque<(u64, Vec<f32>)> = VecDeque::new();
  let (mut thread, mut msg, mut topic, mut left) = (0u32, 0u64, 0usize, 0usize);
  let mut rec = 0usize;
  let mut since_sleep = 0usize;
  let mut gists = 0usize;
  let push = |out: &mut Vec<(At, Fact)>, fold: &mut Fold, rec: usize, facts: Vec<Fact>| {
    let secs = out.len() as i64 * SECS_PER_FACT;
    for (i, f) in facts.into_iter().enumerate() {
      let at = At::new(record(rec), i as u32, secs);
      fold.apply(&at, &f);
      out.push((at, f));
    }
  };
  while out.len() < n {
    if left == 0 {
      // A new thread on a topic; now and then a new topic.
      thread += 1;
      msg = 0;
      left = 4 + rng.below(30);
      if rng.unit() < 0.15 {
        let i = rng.below(topics.len());
        topics[i] = vector(seed ^ (next_topic << 20));
        next_topic += 1;
        topic = i;
      } else {
        topic = rng.below(topics.len());
      }
      // A spawned thread leaves memory before it says anything; now and
      // then one the user switches off, or back on.
      if rng.unit() < 0.1 {
        let t = format!("t{thread}");
        push(
          &mut out,
          &mut fold,
          rec,
          vec![Fact::Remember {
            thread: t,
            on: false,
            recall: true,
          }],
        );
        rec += 1;
        left = 0;
        continue;
      }
      if thread > 20 && rng.unit() < 0.03 {
        let t = format!("t{}", thread - 1 - rng.below(20) as u32);
        let on = rng.unit() < 0.5;
        push(
          &mut out,
          &mut fold,
          rec,
          vec![Fact::Remember {
            thread: t,
            on,
            recall: false,
          }],
        );
        rec += 1;
      }
    }
    // An exchange: the user's message and the reply.
    let mut facts = Vec::new();
    for _ in 0..2 {
      let place = format!("home:t{thread}#m{msg}");
      msg += 1;
      let r = rng.unit();
      let (v, supersedes) = if r < 0.08 && !recent.is_empty() {
        // A near-duplicate of something recent: a Strengthen, a newer
        // number now and then.
        let (_, base) = &recent[recent.len() - 1 - rng.below(recent.len().min(50))];
        (near(base, rng.next(), 0.05), rng.unit() < 0.4)
      } else {
        (
          near(&topics[topic], rng.next(), 0.3 + 0.45 * rng.unit()),
          false,
        )
      };
      recent.push_back((handle(&place), v.clone()));
      if recent.len() > 2000 {
        recent.pop_front();
      }
      facts.push(Fact::Observe {
        place,
        embedding: pack(&v),
        demand: rng.unit() * 0.5,
        own: rng.unit() < 0.2,
        supersedes,
      });
    }
    // The note's feedback, now and then.
    if rng.unit() < 0.3 && recent.len() > 10 {
      let shown: Vec<u64> = (0..1 + rng.below(3))
        .map(|_| recent[rng.below(recent.len())].0)
        .collect();
      let used = if rng.unit() < 0.3 {
        vec![shown[0]]
      } else {
        Vec::new()
      };
      facts.push(Fact::Credit { shown, used });
    } else if rng.unit() < 0.03 && !recent.is_empty() {
      facts.push(Fact::Penalize {
        handles: vec![recent[rng.below(recent.len())].0],
      });
    }
    since_sleep += facts.len();
    left = left.saturating_sub(1);
    push(&mut out, &mut fold, rec, facts);
    rec += 1;
    if since_sleep >= SLEEP_EVERY {
      since_sleep = 0;
      push(&mut out, &mut fold, rec, vec![Fact::Sleep]);
      rec += 1;
      // REM: a quiet cluster due becomes a gist, its weakest dropped.
      let now = out.len() as i64 * SECS_PER_FACT;
      let due = fold.gist_due(now);
      if !due.is_empty() {
        let split = due.len() - due.len() / 4;
        let (kept, dropped) = (due[..split].to_vec(), due[split..].to_vec());
        let base = recent
          .iter()
          .find(|(h, _)| *h == due[0])
          .map_or_else(|| vector(seed ^ gists as u64), |(_, v)| v.clone());
        gists += 1;
        push(
          &mut out,
          &mut fold,
          rec,
          vec![Fact::Gist {
            text: format!("gist {gists} of cluster {:016x}", due[0]),
            embedding: pack(&near(&base, rng.next(), 0.1)),
            kept,
            dropped,
          }],
        );
        rec += 1;
      }
    }
  }
  (out, fold)
}

/// Decisions in a fold's log, for calibrating `history` to the field.
pub fn decisions(fold: &Fold) -> Vec<(Kind, usize)> {
  fold
    .bands(i64::MIN)
    .into_iter()
    .map(|(k, b)| (k, b.iter().sum()))
    .collect()
}
