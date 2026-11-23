//! The fold in bytes, for checkpoints (`folding.rs`): canonical (maps in
//! key order), so two folds with the same state write the same bytes,
//! which is how the tests compare them.

use std::collections::{BTreeSet, HashMap};

use engram::IdentityStore;

use crate::{At, Entry, Fact, Fold, Kind, Mem, center, configure};

const MAGIC: &[u8; 4] = b"SFLD";
const VERSION: u16 = 1;

#[derive(Default)]
struct W(Vec<u8>);

impl W {
  fn u8(&mut self, v: u8) {
    self.0.push(v);
  }
  fn u32(&mut self, v: u32) {
    self.0.extend_from_slice(&v.to_le_bytes());
  }
  fn u64(&mut self, v: u64) {
    self.0.extend_from_slice(&v.to_le_bytes());
  }
  fn i64(&mut self, v: i64) {
    self.0.extend_from_slice(&v.to_le_bytes());
  }
  fn f32(&mut self, v: f32) {
    self.u32(v.to_bits());
  }
  fn len(&mut self, n: usize) {
    self.u64(n as u64);
  }
  fn bytes(&mut self, b: &[u8]) {
    self.len(b.len());
    self.0.extend_from_slice(b);
  }
  fn str(&mut self, s: &str) {
    self.bytes(s.as_bytes());
  }
  fn i8s(&mut self, v: &[i8]) {
    self.len(v.len());
    self.0.extend(v.iter().map(|&x| x as u8));
  }
  fn opt(&mut self, v: Option<u64>) {
    match v {
      None => self.u8(0),
      Some(v) => {
        self.u8(1);
        self.u64(v);
      }
    }
  }
  fn handles(&mut self, v: impl ExactSizeIterator<Item = u64>) {
    self.len(v.len());
    for h in v {
      self.u64(h);
    }
  }
  fn strings<'a>(&mut self, v: impl ExactSizeIterator<Item = &'a String>) {
    self.len(v.len());
    for s in v {
      self.str(s);
    }
  }
}

struct R<'a>(&'a [u8]);

impl<'a> R<'a> {
  fn take(&mut self, n: usize) -> Option<&'a [u8]> {
    if self.0.len() < n {
      return None;
    }
    let (a, b) = self.0.split_at(n);
    self.0 = b;
    Some(a)
  }
  fn u8(&mut self) -> Option<u8> {
    Some(self.take(1)?[0])
  }
  fn u32(&mut self) -> Option<u32> {
    Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
  }
  fn u64(&mut self) -> Option<u64> {
    Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
  }
  fn i64(&mut self) -> Option<i64> {
    Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
  }
  fn f32(&mut self) -> Option<f32> {
    Some(f32::from_bits(self.u32()?))
  }
  fn len(&mut self) -> Option<usize> {
    let n = usize::try_from(self.u64()?).ok()?;
    // A length can never exceed what is left: corrupt bytes fail here
    // instead of allocating.
    (n <= self.0.len()).then_some(n)
  }
  fn bytes(&mut self) -> Option<&'a [u8]> {
    let n = self.len()?;
    self.take(n)
  }
  fn str(&mut self) -> Option<String> {
    String::from_utf8(self.bytes()?.to_vec()).ok()
  }
  fn i8s(&mut self) -> Option<Vec<i8>> {
    Some(self.bytes()?.iter().map(|&b| b as i8).collect())
  }
  fn opt(&mut self) -> Option<Option<u64>> {
    match self.u8()? {
      0 => Some(None),
      1 => Some(Some(self.u64()?)),
      _ => None,
    }
  }
  fn handles(&mut self) -> Option<Vec<u64>> {
    let n = self.len()?;
    (0..n).map(|_| self.u64()).collect()
  }
  fn strings(&mut self) -> Option<BTreeSet<String>> {
    let n = self.len()?;
    (0..n).map(|_| self.str()).collect()
  }
}

fn kind_byte(k: Kind) -> u8 {
  match k {
    Kind::New => 0,
    Kind::Link => 1,
    Kind::Strengthen => 2,
    Kind::Skip => 3,
    Kind::Gist => 4,
  }
}

fn kind_of(b: u8) -> Option<Kind> {
  Some(match b {
    0 => Kind::New,
    1 => Kind::Link,
    2 => Kind::Strengthen,
    3 => Kind::Skip,
    4 => Kind::Gist,
    _ => return None,
  })
}

fn sorted<V>(m: &HashMap<u64, V>) -> Vec<(u64, &V)> {
  let mut v: Vec<(u64, &V)> = m.iter().map(|(k, v)| (*k, v)).collect();
  v.sort_unstable_by_key(|(k, _)| *k);
  v
}

fn vectors(w: &mut W, m: &HashMap<u64, Vec<i8>>) {
  w.len(m.len());
  for (h, v) in sorted(m) {
    w.u64(h);
    w.i8s(v);
  }
}

fn read_vectors(r: &mut R) -> Option<HashMap<u64, Vec<i8>>> {
  let n = r.len()?;
  (0..n).map(|_| Some((r.u64()?, r.i8s()?))).collect()
}

impl Fold {
  /// The whole fold state, canonical.
  pub fn to_bytes(&self) -> Vec<u8> {
    let mut w = W::default();
    w.0.extend_from_slice(MAGIC);
    w.u32(u32::from(VERSION));
    w.bytes(&self.mem.snapshot(center().tag_u64));
    vectors(&mut w, &self.vectors);
    vectors(&mut w, &self.parked);
    w.len(self.merged.len());
    for (h, m) in sorted(&self.merged) {
      w.u64(h);
      w.handles(m.iter().copied());
    }
    let mut threads: Vec<(&String, &BTreeSet<u64>)> = self.by_thread.iter().collect();
    threads.sort_unstable_by_key(|(t, _)| *t);
    w.len(threads.len());
    for (t, hs) in threads {
      w.str(t);
      w.handles(hs.iter().copied());
    }
    w.len(self.log.len());
    for (h, e) in sorted(&self.log) {
      w.u64(h);
      w.str(&e.place);
      w.u8(kind_byte(e.kind));
      w.f32(e.novelty);
      w.opt(e.target);
      w.opt(e.resolved);
      w.u8(u8::from(e.forgotten));
      w.i64(e.secs);
    }
    w.len(self.shown.len());
    for (h, times) in sorted(&self.shown) {
      w.u64(h);
      w.len(times.len());
      for t in times {
        w.i64(*t);
      }
    }
    w.strings(self.off.iter());
    w.strings(self.switched.iter());
    w.strings(self.recalling.iter());
    w.len(self.gists.len());
    for (h, text) in sorted(&self.gists) {
      w.u64(h);
      w.str(text);
    }
    match &self.last {
      None => w.u8(0),
      Some((r, p)) => {
        w.u8(1);
        w.str(r);
        w.u32(*p);
      }
    }
    w.len(self.folded);
    w.0
  }

  /// A fold back from `to_bytes`; `None` when the bytes are not one (or
  /// were written for another encoder).
  pub fn from_bytes(bytes: &[u8]) -> Option<Fold> {
    let mut r = R(bytes);
    if r.take(4)? != MAGIC || r.u32()? != u32::from(VERSION) {
      return None;
    }
    let mut mem = Mem::restore(r.bytes()?, center().tag_u64, IdentityStore::new())?;
    configure(&mut mem);
    let vectors = read_vectors(&mut r)?;
    let parked = read_vectors(&mut r)?;
    let n = r.len()?;
    let merged = (0..n)
      .map(|_| Some((r.u64()?, r.handles()?)))
      .collect::<Option<HashMap<u64, Vec<u64>>>>()?;
    let n = r.len()?;
    let by_thread = (0..n)
      .map(|_| Some((r.str()?, r.handles()?.into_iter().collect())))
      .collect::<Option<HashMap<String, BTreeSet<u64>>>>()?;
    let n = r.len()?;
    let log = (0..n)
      .map(|_| {
        let h = r.u64()?;
        Some((
          h,
          Entry {
            place: r.str()?,
            kind: kind_of(r.u8()?)?,
            novelty: r.f32()?,
            target: r.opt()?,
            resolved: r.opt()?,
            forgotten: r.u8()? != 0,
            secs: r.i64()?,
          },
        ))
      })
      .collect::<Option<HashMap<u64, Entry>>>()?;
    let n = r.len()?;
    let shown = (0..n)
      .map(|_| {
        let h = r.u64()?;
        let k = r.len()?;
        Some((h, (0..k).map(|_| r.i64()).collect::<Option<Vec<i64>>>()?))
      })
      .collect::<Option<HashMap<u64, Vec<i64>>>>()?;
    let off = r.strings()?;
    let switched = r.strings()?;
    let recalling = r.strings()?;
    let n = r.len()?;
    let gists = (0..n)
      .map(|_| Some((r.u64()?, r.str()?)))
      .collect::<Option<HashMap<u64, String>>>()?;
    let last = match r.u8()? {
      0 => None,
      1 => Some((r.str()?, r.u32()?)),
      _ => return None,
    };
    let folded = usize::try_from(r.u64()?).ok()?;
    if !r.0.is_empty() {
      return None;
    }
    Some(Fold {
      mem,
      vectors,
      parked,
      merged,
      by_thread,
      log,
      shown,
      off,
      switched,
      recalling,
      gists,
      last,
      folded,
    })
  }
}

/// A fact where it is, in bytes: what a checkpoint's chain digests.
pub(crate) fn fact_bytes(at: &At, f: &Fact) -> Vec<u8> {
  let mut w = W::default();
  w.str(&at.record);
  w.u32(at.pos);
  w.i64(at.secs);
  match f {
    Fact::Observe {
      place,
      embedding,
      demand,
      own,
      supersedes,
    } => {
      w.u8(0);
      w.str(place);
      w.i8s(embedding);
      w.f32(*demand);
      w.u8(u8::from(*own));
      w.u8(u8::from(*supersedes));
    }
    Fact::Credit { shown, used } => {
      w.u8(1);
      w.handles(shown.iter().copied());
      w.handles(used.iter().copied());
    }
    Fact::Penalize { handles } => {
      w.u8(2);
      w.handles(handles.iter().copied());
    }
    Fact::Sleep => w.u8(3),
    Fact::Gist {
      text,
      embedding,
      kept,
      dropped,
    } => {
      w.u8(4);
      w.str(text);
      w.i8s(embedding);
      w.handles(kept.iter().copied());
      w.handles(dropped.iter().copied());
    }
    Fact::Remember { thread, on, recall } => {
      w.u8(5);
      w.str(thread);
      w.u8(u8::from(*on));
      w.u8(u8::from(*recall));
    }
  }
  w.0
}

/// A 64-bit digest of bytes (word at a time: fast, stable on every
/// platform; local caches only, never synced).
pub(crate) fn digest(bytes: &[u8]) -> u64 {
  let mut h = 0x243f_6a88_85a3_08d3u64 ^ bytes.len() as u64;
  let (chunks, rest) = bytes.as_chunks::<8>();
  for c in chunks {
    h = crate::synth::splitmix(h ^ u64::from_le_bytes(*c));
  }
  let mut tail = [0u8; 8];
  tail[..rest.len()].copy_from_slice(rest);
  crate::synth::splitmix(h ^ u64::from_le_bytes(tail))
}
