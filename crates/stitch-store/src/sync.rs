//! Record sync: range-based set reconciliation of the files two nodes hold
//! for one repository (docs/records.md §7, design §2.6).
//!
//! An item is a file: its root-relative path (the key) and the blake3 hash of
//! its content. A range's fingerprint is the XOR of `blake3(key, 0, hash)`
//! over its items, with their count. Equal ranges end; small ranges are
//! answered with their items; large ones are split. Both sides then know what
//! to send. The exchange is plain messages, independent of the transport.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use stitch_records::BlobHash;

use crate::{Inserted, ItemPath, RecordRoot, Result};

/// Ranges with at most this many items are answered with their items.
pub const LIST_THRESHOLD: usize = 32;
/// How many subranges a large range is split into.
pub const BRANCHING: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
  pub key: String,
  pub hash: [u8; 32],
}

/// A sorted set of items, with each item's fingerprint contribution.
#[derive(Clone, Debug, Default)]
pub struct ItemSet {
  items: Vec<Item>,
  digests: Vec<[u8; 32]>,
}

pub(crate) fn digest(item: &Item) -> [u8; 32] {
  let mut h = blake3::Hasher::new();
  h.update(item.key.as_bytes());
  h.update(&[0]);
  h.update(&item.hash);
  *h.finalize().as_bytes()
}

impl ItemSet {
  pub fn new(mut items: Vec<Item>) -> Self {
    items.sort_by(|a, b| a.key.cmp(&b.key));
    items.dedup_by(|a, b| a.key == b.key);
    let digests = items.iter().map(digest).collect();
    ItemSet { items, digests }
  }

  /// Reads and hashes every file of a root.
  pub fn from_root(root: &RecordRoot) -> Result<Self> {
    let mut items = Vec::new();
    for item in root.scan()? {
      let bytes = root.read(&item)?;
      items.push(Item {
        key: item.to_string(),
        hash: *BlobHash::of(&bytes).0.as_bytes(),
      });
    }
    Ok(ItemSet::new(items))
  }

  pub fn len(&self) -> usize {
    self.items.len()
  }

  pub fn is_empty(&self) -> bool {
    self.items.is_empty()
  }

  pub fn items(&self) -> &[Item] {
    &self.items
  }

  /// The fingerprint of the whole set: equal sets have equal ones, so two
  /// nodes compare a repository in one message.
  pub fn summary(&self) -> Fingerprint {
    self.fingerprint(0..self.items.len())
  }

  fn bounds(&self, range: &Range) -> std::ops::Range<usize> {
    let lo = self
      .items
      .partition_point(|i| i.key.as_str() < range.lo.as_str());
    let hi = match &range.hi {
      Some(hi) => self.items.partition_point(|i| i.key.as_str() < hi.as_str()),
      None => self.items.len(),
    };
    lo..hi.max(lo)
  }

  fn fingerprint(&self, span: std::ops::Range<usize>) -> Fingerprint {
    let mut xor = [0u8; 32];
    for d in &self.digests[span.clone()] {
      for (x, b) in xor.iter_mut().zip(d) {
        *x ^= b;
      }
    }
    Fingerprint {
      xor,
      count: span.len() as u64,
    }
  }
}

/// Keys from `lo` (inclusive) to `hi` (exclusive; `None` is the end).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
  pub lo: String,
  pub hi: Option<String>,
}

impl Range {
  pub fn all() -> Self {
    Range {
      lo: String::new(),
      hi: None,
    }
  }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
  pub xor: [u8; 32],
  pub count: u64,
}

impl Fingerprint {
  /// One more item (XOR is its own inverse: the order never matters).
  pub fn add(&mut self, item: &Item) {
    for (x, b) in self.xor.iter_mut().zip(digest(item)) {
      *x ^= b;
    }
    self.count += 1;
  }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
  Fingerprint {
    range: Range,
    fingerprint: Fingerprint,
  },
  /// The sender's items in `range`; `reply` asks for the receiver's.
  Items {
    range: Range,
    items: Vec<Item>,
    reply: bool,
  },
}

/// What one side learned: what to send, what it will receive, and paths both
/// hold with different content.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
  pub give: BTreeSet<String>,
  pub need: BTreeSet<String>,
  pub conflicts: BTreeSet<String>,
}

/// One side of a reconciliation.
pub struct Reconciler<'a> {
  set: &'a ItemSet,
  outcome: Outcome,
}

impl<'a> Reconciler<'a> {
  pub fn new(set: &'a ItemSet) -> Self {
    Reconciler {
      set,
      outcome: Outcome::default(),
    }
  }

  /// The opening message: the fingerprint of everything.
  pub fn initiate(&self) -> Vec<Message> {
    vec![Message::Fingerprint {
      range: Range::all(),
      fingerprint: self.set.fingerprint(self.set.bounds(&Range::all())),
    }]
  }

  pub fn handle(&mut self, message: Message) -> Vec<Message> {
    match message {
      Message::Fingerprint { range, fingerprint } => self.on_fingerprint(range, fingerprint),
      Message::Items {
        range,
        items,
        reply,
      } => {
        let span = self.set.bounds(&range);
        self.compare(&self.set.items[span.clone()], &items);
        if reply {
          vec![Message::Items {
            range,
            items: self.set.items[span].to_vec(),
            reply: false,
          }]
        } else {
          Vec::new()
        }
      }
    }
  }

  pub fn finish(self) -> Outcome {
    self.outcome
  }

  fn on_fingerprint(&mut self, range: Range, theirs: Fingerprint) -> Vec<Message> {
    let span = self.set.bounds(&range);
    let mine = &self.set.items[span.clone()];
    if self.set.fingerprint(span.clone()) == theirs {
      return Vec::new();
    }
    if theirs.count == 0 {
      // They hold nothing here: everything of ours goes.
      self.outcome.give.extend(mine.iter().map(|i| i.key.clone()));
      return vec![Message::Items {
        range,
        items: mine.to_vec(),
        reply: false,
      }];
    }
    if mine.len() <= LIST_THRESHOLD {
      return vec![Message::Items {
        range,
        items: mine.to_vec(),
        reply: true,
      }];
    }
    let chunk = mine.len().div_ceil(BRANCHING);
    let starts: Vec<usize> = (0..mine.len()).step_by(chunk).collect();
    starts
      .iter()
      .enumerate()
      .map(|(n, &start)| {
        let lo = if n == 0 {
          range.lo.clone()
        } else {
          mine[start].key.clone()
        };
        let hi = match starts.get(n + 1) {
          Some(&next) => Some(mine[next].key.clone()),
          None => range.hi.clone(),
        };
        let end = starts.get(n + 1).copied().unwrap_or(mine.len());
        Message::Fingerprint {
          range: Range { lo, hi },
          fingerprint: self.set.fingerprint(span.start + start..span.start + end),
        }
      })
      .collect()
  }

  fn compare(&mut self, mine: &[Item], theirs: &[Item]) {
    let (mut i, mut j) = (0, 0);
    while i < mine.len() || j < theirs.len() {
      match (mine.get(i), theirs.get(j)) {
        (Some(a), Some(b)) if a.key == b.key => {
          if a.hash != b.hash {
            self.outcome.conflicts.insert(a.key.clone());
          }
          i += 1;
          j += 1;
        }
        (Some(a), b) if b.is_none_or(|b| a.key < b.key) => {
          self.outcome.give.insert(a.key.clone());
          i += 1;
        }
        (_, Some(b)) => {
          self.outcome.need.insert(b.key.clone());
          j += 1;
        }
        (None, None) => unreachable!(),
        (Some(_), None) => unreachable!("handled by the arm above"),
      }
    }
  }
}

/// Counts of one in-process reconciliation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exchange {
  pub rounds: usize,
  pub messages: usize,
}

/// Runs both sides in process until neither has anything to say.
pub fn reconcile(a: &ItemSet, b: &ItemSet) -> (Outcome, Outcome, Exchange) {
  let mut ra = Reconciler::new(a);
  let mut rb = Reconciler::new(b);
  let mut stats = Exchange::default();
  let mut to_b = ra.initiate();
  let mut to_a = Vec::new();
  while !to_b.is_empty() || !to_a.is_empty() {
    stats.rounds += 1;
    stats.messages += to_b.len() + to_a.len();
    let from_b: Vec<Message> = to_b.drain(..).flat_map(|m| rb.handle(m)).collect();
    let from_a: Vec<Message> = to_a.drain(..).flat_map(|m| ra.handle(m)).collect();
    to_a = from_b;
    to_b = from_a;
  }
  (ra.finish(), rb.finish(), stats)
}

#[derive(Clone, Debug, Default)]
pub struct SyncReport {
  /// Paths written into `a` and into `b`.
  pub into_a: Vec<String>,
  pub into_b: Vec<String>,
  pub conflicts: Vec<String>,
  pub exchange: Exchange,
}

/// Syncs two roots of the same repository held by this process (two
/// checkouts, or a test). Over a network the same messages travel instead.
pub fn sync_roots(
  a: &RecordRoot,
  a_set: &ItemSet,
  b: &RecordRoot,
  b_set: &ItemSet,
) -> Result<SyncReport> {
  if a.repo() != b.repo() {
    return Err(crate::Error::Mismatch {
      path: b.dir().display().to_string(),
      message: format!("repository {} is not {}", b.repo(), a.repo()),
    });
  }
  let (oa, ob, exchange) = reconcile(a_set, b_set);
  debug_assert_eq!(oa.give, ob.need);
  debug_assert_eq!(ob.give, oa.need);
  let mut report = SyncReport {
    exchange,
    conflicts: oa.conflicts.into_iter().collect(),
    ..SyncReport::default()
  };
  let copy = |from: &RecordRoot, to: &RecordRoot, key: &str, log: &mut Vec<String>| -> Result<()> {
    let item = ItemPath::parse(key).ok_or_else(|| crate::Error::InvalidPath(key.to_owned()))?;
    if to.insert(key, &from.read(&item)?)? == Inserted::New {
      log.push(key.to_owned());
    }
    Ok(())
  };
  for key in &oa.give {
    copy(a, b, key, &mut report.into_b)?;
  }
  for key in &ob.give {
    copy(b, a, key, &mut report.into_a)?;
  }
  Ok(report)
}

#[cfg(test)]
mod tests {
  use super::*;
  use rand::RngExt;

  fn item(n: u32, version: u8) -> Item {
    Item {
      key: format!("records/2026-10/{n:08}.md"),
      hash: [version; 32],
    }
  }

  fn check(a: &ItemSet, b: &ItemSet) -> Exchange {
    let (oa, ob, ex) = reconcile(a, b);
    let ka: BTreeSet<_> = a.items().iter().map(|i| i.key.clone()).collect();
    let kb: BTreeSet<_> = b.items().iter().map(|i| i.key.clone()).collect();
    assert_eq!(oa.give, &ka - &kb);
    assert_eq!(oa.need, &kb - &ka);
    assert_eq!(oa.give, ob.need);
    assert_eq!(ob.give, oa.need);
    ex
  }

  #[test]
  fn equal_sets_exchange_one_fingerprint() {
    let s = ItemSet::new((0..5000).map(|n| item(n, 1)).collect());
    let ex = check(&s, &s.clone());
    assert_eq!(ex.messages, 1);
  }

  #[test]
  fn an_empty_side_receives_everything_in_two_messages() {
    let full = ItemSet::new((0..5000).map(|n| item(n, 1)).collect());
    let empty = ItemSet::default();
    assert_eq!(check(&empty, &full).messages, 2);
    assert_eq!(check(&full, &empty).messages, 3);
  }

  #[test]
  fn random_differences_are_found_exactly() {
    let mut rng = rand::rng();
    for round in 0..40 {
      let n = [0, 1, 10, 100, 3000][round % 5];
      let mut a = Vec::new();
      let mut b = Vec::new();
      for k in 0..n {
        match rng.random_range(0..10) {
          0 => a.push(item(k, 1)),
          1 => b.push(item(k, 1)),
          _ => {
            a.push(item(k, 1));
            b.push(item(k, 1));
          }
        }
      }
      check(&ItemSet::new(a), &ItemSet::new(b));
    }
  }

  #[test]
  fn a_day_of_new_records_costs_few_rounds() {
    let base: Vec<Item> = (0..20_000).map(|n| item(n, 1)).collect();
    let mut newer = base.clone();
    newer.extend((20_000..20_050).map(|n| item(n, 1)));
    let ex = check(&ItemSet::new(base), &ItemSet::new(newer));
    assert!(ex.rounds <= 8, "{ex:?}");
  }

  #[test]
  fn same_path_with_other_content_is_a_conflict() {
    let a = ItemSet::new(vec![item(1, 1), item(2, 1)]);
    let b = ItemSet::new(vec![item(1, 1), item(2, 2)]);
    let (oa, ob, _) = reconcile(&a, &b);
    assert!(oa.give.is_empty() && oa.need.is_empty());
    assert_eq!(oa.conflicts.len(), 1);
    assert_eq!(ob.conflicts, oa.conflicts);
  }
}
