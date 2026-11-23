//! The fold kept up to date as facts arrive, in any order, with
//! checkpoints (design §2.9, "As built").
//!
//! Facts fold in record-ID order, and a fact older than what was folded
//! (another node's, synced late) used to fold everything again from the
//! start: all history × the live episodes, each time. Now the fold's whole
//! state is kept at the last few sleeps, and a late fact folds again from
//! the newest checkpoint before it.
//!
//! A checkpoint is the fold after the facts before it, named by how many
//! there were and a digest chained over them in order (each fact with its
//! record, position and time): it is used only while the facts before it
//! are exactly the same, so the result is the fold from scratch, bit for
//! bit, whatever the order of arrival. It is a local cache, never synced
//! (like the index), kept on disk when the node gives a directory (its
//! files also name the tuning, the encoder and the build: another's are
//! discarded); taken at sleeps, where engram's snapshot is exact (its
//! frequency counts are rebuilt from the survivors, as a restore does).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::persist::{digest, fact_bytes};
use crate::synth::splitmix;
use crate::{Applied, At, Fact, Fold, tuning_tag};

/// Checkpoints kept: the last few sleeps (a sleep every twenty hours; a
/// fact synced later than that folds from the start).
pub const CHECKPOINTS: usize = 3;
/// And the most they take on disk, together.
pub const CHECKPOINT_BYTES: u64 = 256 << 20;

struct Checkpoint {
  /// Facts folded into it.
  n: usize,
  /// The chain digest of those facts.
  chain: u64,
  size: u64,
  /// Its bytes, when there is no directory.
  bytes: Option<Vec<u8>>,
}

/// Every fact folded, in order, the fold they make, and its checkpoints.
pub struct Folding {
  fold: Fold,
  facts: Vec<(At, Fact)>,
  /// Each fact's digest, and the chain over the facts up to it.
  digests: Vec<u64>,
  chain: Vec<u64>,
  dir: Option<PathBuf>,
  salt: u64,
  checkpoints: Vec<Checkpoint>,
  /// The last fold: where it started (a checkpoint's facts, or what was
  /// folded already), and how many facts it folded.
  last: (usize, usize),
}

impl Default for Folding {
  fn default() -> Folding {
    Folding::new()
  }
}

fn order(a: &(At, Fact), b: &(At, Fact)) -> std::cmp::Ordering {
  a.0.key().cmp(&b.0.key())
}

fn link(prev: u64, d: u64) -> u64 {
  splitmix(prev ^ splitmix(d))
}

impl Folding {
  /// With checkpoints in memory only.
  pub fn new() -> Folding {
    Folding {
      fold: Fold::new(),
      facts: Vec::new(),
      digests: Vec::new(),
      chain: Vec::new(),
      dir: None,
      salt: tuning_tag(),
      checkpoints: Vec::new(),
      last: (0, 0),
    }
  }

  /// With checkpoints in `dir`, for this `build` (a checkpoint holds what
  /// this build's fold made, so another build's is discarded): those
  /// already there are used when the facts before them are the same.
  pub fn open(dir: &Path, build: &str) -> Folding {
    let mut f = Folding::new();
    f.salt = digest(format!("{:016x}|{build}", tuning_tag()).as_bytes());
    let _ = std::fs::create_dir_all(dir);
    let prefix = format!("{:016x}-", f.salt);
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
      let name = entry.file_name().to_string_lossy().into_owned();
      let parsed = name
        .strip_prefix(&prefix)
        .and_then(|r| r.strip_suffix(".fold"))
        .and_then(|r| r.split_once('-'))
        .and_then(|(n, c)| Some((n.parse().ok()?, u64::from_str_radix(c, 16).ok()?)));
      match (parsed, entry.metadata()) {
        (Some((n, chain)), Ok(m)) if n > 0 => f.checkpoints.push(Checkpoint {
          n,
          chain,
          size: m.len(),
          bytes: None,
        }),
        // Another build's, tuning's or encoder's, or a write cut short.
        _ => {
          let _ = std::fs::remove_file(entry.path());
        }
      }
    }
    f.dir = Some(dir.to_path_buf());
    f.checkpoints.sort_by_key(|c| c.n);
    f.prune();
    f
  }

  /// The fold.
  pub fn fold(&self) -> &Fold {
    &self.fold
  }

  /// Every fact folded, in order.
  pub fn facts(&self) -> &[(At, Fact)] {
    &self.facts
  }

  /// The checkpoints kept: how many facts each holds, oldest first.
  pub fn checkpoints(&self) -> Vec<usize> {
    self.checkpoints.iter().map(|c| c.n).collect()
  }

  /// The last fold: the facts it started from (a checkpoint's, or what was
  /// folded already; 0 from scratch) and how many it folded.
  pub fn last_fold(&self) -> (usize, usize) {
    self.last
  }

  /// Folds facts not folded before, in any order: on top when they all
  /// come after what was folded, otherwise from the newest checkpoint
  /// before the oldest of them (from the start when there is none). What
  /// the observations among them did, by record (those a checkpoint on
  /// disk already held, at a start, are not folded again and say nothing).
  pub fn add(&mut self, mut new: Vec<(At, Fact)>) -> HashMap<String, Vec<Applied>> {
    if new.is_empty() {
      return HashMap::new();
    }
    new.sort_by(order);
    let fresh: BTreeSet<String> = new.iter().map(|(at, _)| at.record.clone()).collect();
    let digests: Vec<u64> = new
      .iter()
      .map(|(at, f)| digest(&fact_bytes(at, f)))
      .collect();
    let j = self.facts.partition_point(|f| f.0.key() < new[0].0.key());
    if j == self.facts.len() {
      self.facts.extend(new);
      self.digests.extend(digests);
    } else {
      let tail = self.facts.split_off(j);
      let tail_digests = self.digests.split_off(j);
      let mut a = tail.into_iter().zip(tail_digests).peekable();
      let mut b = new.into_iter().zip(digests).peekable();
      loop {
        let take_a = match (a.peek(), b.peek()) {
          (Some(x), Some(y)) => order(&x.0, &y.0).is_le(),
          (Some(_), None) => true,
          (None, Some(_)) => false,
          (None, None) => break,
        };
        let (f, d) = if take_a { a.next() } else { b.next() }.expect("peeked");
        self.facts.push(f);
        self.digests.push(d);
      }
    }
    self.rechain(j);
    self.refold(j, &fresh)
  }

  /// Keeps only the facts `keep` allows, folding again (from the newest
  /// checkpoint before the first that went) when any went.
  pub fn retain(&mut self, mut keep: impl FnMut(&At, &Fact) -> bool) {
    let Some(j) = self.facts.iter().position(|(at, f)| !keep(at, f)) else {
      return;
    };
    let tail = self.facts.split_off(j);
    let tail_digests = self.digests.split_off(j);
    // `keep` already said no to the first.
    for ((at, f), d) in tail.into_iter().zip(tail_digests).skip(1) {
      if keep(&at, &f) {
        self.facts.push((at, f));
        self.digests.push(d);
      }
    }
    self.rechain(j);
    self.refold(j, &BTreeSet::new());
  }

  /// The chain again from fact `j` on.
  fn rechain(&mut self, j: usize) {
    self.chain.truncate(j);
    let mut prev = j.checked_sub(1).map_or(0, |i| self.chain[i]);
    for d in &self.digests[j..] {
      prev = link(prev, *d);
      self.chain.push(prev);
    }
  }

  /// Whether a checkpoint holds exactly the facts now before its point.
  fn valid(&self, c: &Checkpoint) -> bool {
    c.n <= self.facts.len() && self.chain[c.n - 1] == c.chain
  }

  fn path(&self, n: usize, chain: u64) -> Option<PathBuf> {
    let name = format!("{:016x}-{n:010}-{chain:016x}.fold", self.salt);
    self.dir.as_ref().map(|d| d.join(name))
  }

  fn load(&self, c: &Checkpoint) -> Option<Fold> {
    let fold = match &c.bytes {
      Some(b) => Fold::from_bytes(b),
      None => Fold::from_bytes(&std::fs::read(self.path(c.n, c.chain)?).ok()?),
    }?;
    (fold.len() == c.n).then_some(fold)
  }

  fn drop_checkpoint(&mut self, i: usize) {
    let c = self.checkpoints.remove(i);
    if let Some(p) = self.path(c.n, c.chain) {
      let _ = std::fs::remove_file(p);
    }
  }

  /// At most `CHECKPOINTS`, the newest, within `CHECKPOINT_BYTES`.
  fn prune(&mut self) {
    while self.checkpoints.len() > CHECKPOINTS
      || (self.checkpoints.len() > 1
        && self.checkpoints.iter().map(|c| c.size).sum::<u64>() > CHECKPOINT_BYTES)
    {
      self.drop_checkpoint(0);
    }
  }

  fn save(&mut self) {
    let n = self.fold.len();
    let chain = self.chain[n - 1];
    let bytes = self.fold.to_bytes();
    let size = bytes.len() as u64;
    let bytes = match self.path(n, chain) {
      Some(p) => {
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, &bytes)
          .and_then(|()| std::fs::rename(&tmp, &p))
          .is_err()
        {
          let _ = std::fs::remove_file(&tmp);
          return;
        }
        None
      }
      None => Some(bytes),
    };
    self.checkpoints.push(Checkpoint {
      n,
      chain,
      size,
      bytes,
    });
    self.checkpoints.sort_by_key(|c| c.n);
    self.prune();
  }

  /// Folds again from where what is folded still holds (the facts before
  /// `j` unchanged), a checkpoint, or the start.
  fn refold(&mut self, j: usize, fresh: &BTreeSet<String>) -> HashMap<String, Vec<Applied>> {
    // Checkpoints over facts that changed go.
    let mut i = 0;
    while i < self.checkpoints.len() {
      if self.valid(&self.checkpoints[i]) {
        i += 1;
      } else {
        self.drop_checkpoint(i);
      }
    }
    let mut start = if self.fold.len() <= j {
      self.fold.len()
    } else {
      self.fold = Fold::new();
      0
    };
    while let Some(c) = self.checkpoints.last().filter(|c| c.n > start) {
      match self.load(c) {
        Some(fold) => {
          start = fold.len();
          self.fold = fold;
          break;
        }
        None => self.drop_checkpoint(self.checkpoints.len() - 1),
      }
    }
    // The last few sleeps get checkpoints, those newer than any kept.
    let newest = self.checkpoints.last().map_or(0, |c| c.n);
    let mut sleeps: Vec<usize> = (start..self.facts.len())
      .filter(|&i| self.facts[i].1 == Fact::Sleep && i + 1 > newest)
      .collect();
    sleeps.drain(..sleeps.len().saturating_sub(CHECKPOINTS));
    let mut out: HashMap<String, Vec<Applied>> = HashMap::new();
    // `save` reads the fold and the chain, not the facts.
    let facts = std::mem::take(&mut self.facts);
    for (i, (at, f)) in facts.iter().enumerate().skip(start) {
      if let Some(a) = self.fold.apply(at, f)
        && fresh.contains(&at.record)
      {
        out.entry(at.record.clone()).or_default().push(a);
      }
      if sleeps.first() == Some(&i) {
        sleeps.remove(0);
        self.save();
      }
    }
    self.facts = facts;
    self.last = (start, self.facts.len() - start);
    out
  }
}
