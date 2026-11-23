//! Transfers field-calibrated thresholds from an earlier encoder to the one
//! Stitch runs. edge talk tuned its memory on months of real traffic with
//! EmbeddingGemma-300m; every distance and jaccard band it settled on is a
//! position in THAT model's distribution. This re-embeds the same messages
//! with EmbeddingGemma 2 and maps each threshold to the value at the same
//! quantile of the new model's distribution, over pairs dense around the
//! thresholds (each sampled message's nearest neighbours under the old
//! model, plus random pairs).
//!
//! Input: a TSV of `id \t sender \t base64 int8 old embedding \t text`
//! (doc-prefixed, as the old store kept them). The old model is run too,
//! for query-prefixed cues.
//!
//! STITCH_EMBED_MODEL=…/embeddinggemma-2-Q8_0.gguf cargo run --release \
//!   -p stitch-memory --example transfer -- field.tsv old.gguf old.center.json cache-dir

use std::path::Path;

use engram_fit::{apply_center, codes_from, int8_unit_round_trip};
use serde::Deserialize;
use stitch_llama::{Embedder, Options};
use stitch_memory::{DOC_PREFIX, K, QUERY_PREFIX};

#[derive(Deserialize)]
struct OldCenter {
  doc_center: Vec<f32>,
  query_center: Vec<f32>,
  doc_prefix: String,
  query_prefix: String,
}

struct Rng(u64);

impl Rng {
  fn next(&mut self) -> u64 {
    self.0 = self
      .0
      .wrapping_mul(6_364_136_223_846_793_005)
      .wrapping_add(1_442_695_040_888_963_407);
    self.0 >> 33
  }
  fn below(&mut self, n: usize) -> usize {
    (self.next() % n as u64) as usize
  }
}

fn b64(s: &str) -> Vec<u8> {
  const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let (mut bits, mut n, mut out) = (0u32, 0, Vec::new());
  for c in s.bytes().filter(|&c| c != b'=') {
    bits = (bits << 6) | A.iter().position(|&x| x == c).expect("base64") as u32;
    n += 6;
    if n >= 8 {
      n -= 8;
      out.push((bits >> n) as u8);
    }
  }
  out
}

fn cos(a: &[f32], b: &[f32]) -> f32 {
  let (mut d, mut na, mut nb) = (0.0, 0.0, 0.0);
  for (x, y) in a.iter().zip(b) {
    d += x * y;
    na += x * x;
    nb += y * y;
  }
  d / (na * nb).sqrt().max(1e-9)
}

/// Embeds `texts` with `prefix`, int8 round-tripped; cached in `cache`.
fn embed_cached(model: &Path, prefix: &str, texts: &[&str], cache: &Path) -> Vec<Vec<f32>> {
  if let Ok(bytes) = std::fs::read(cache)
    && bytes.len() == texts.len() * 768
  {
    return bytes
      .chunks(768)
      .map(|c| c.iter().map(|&b| f32::from(b as i8) / 127.0).collect())
      .collect();
  }
  let mut e = Embedder::load(model, None, &Options::default()).expect("loads");
  let mut out: Vec<Vec<f32>> = texts
    .iter()
    .enumerate()
    .map(|(i, t)| {
      if i % 500 == 0 {
        eprintln!("  {}/{} {}", i, texts.len(), model.display());
      }
      e.embed(&format!("{prefix}{t}")).expect("embeds")
    })
    .collect();
  int8_unit_round_trip(&mut out);
  let bytes: Vec<u8> = out
    .iter()
    .flat_map(|v| v.iter().map(|x| (x * 127.0).round() as i8 as u8))
    .collect();
  std::fs::write(cache, bytes).expect("cache written");
  out
}

/// The value at the quantile of `new` where `t` sits in `old`.
fn map(old: &[f32], new: &[f32], t: f32) -> (f32, f32) {
  let mut o = old.to_vec();
  let mut n = new.to_vec();
  o.sort_by(f32::total_cmp);
  n.sort_by(f32::total_cmp);
  let q = o.partition_point(|x| *x <= t) as f32 / o.len() as f32;
  let i = ((q * n.len() as f32) as usize).min(n.len() - 1);
  (q, n[i])
}

fn spearman(a: &[f32], b: &[f32]) -> f32 {
  let rank = |v: &[f32]| {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&i, &j| v[i].total_cmp(&v[j]));
    let mut r = vec![0.0f32; v.len()];
    for (k, &i) in idx.iter().enumerate() {
      r[i] = k as f32;
    }
    r
  };
  let (ra, rb) = (rank(a), rank(b));
  let n = a.len() as f32;
  let mean = (n - 1.0) / 2.0;
  let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
  for i in 0..a.len() {
    let (x, y) = (ra[i] - mean, rb[i] - mean);
    num += x * y;
    da += x * x;
    db += y * y;
  }
  num / (da * db).sqrt()
}

fn main() {
  let args: Vec<String> = std::env::args().collect();
  let (tsv, old_model, old_center, cache) =
    (&args[1], Path::new(&args[2]), &args[3], Path::new(&args[4]));
  let new_model = std::env::var("STITCH_EMBED_MODEL").expect("set STITCH_EMBED_MODEL");
  let new_model = Path::new(&new_model);
  let oc: OldCenter = serde_json::from_str(&std::fs::read_to_string(old_center).unwrap()).unwrap();
  let nc = stitch_memory::center();

  let text = std::fs::read_to_string(tsv).unwrap();
  let rows: Vec<(&str, Vec<f32>, &str)> = text
    .lines()
    .filter_map(|l| {
      let mut f = l.splitn(4, '\t');
      let (_, sender, emb, t) = (f.next()?, f.next()?, f.next()?, f.next()?);
      let v: Vec<f32> = b64(emb)
        .iter()
        .map(|&b| f32::from(b as i8) / 127.0)
        .collect();
      (v.len() == 768).then_some((sender, v, t))
    })
    .collect();
  eprintln!("{} messages", rows.len());
  let texts: Vec<&str> = rows.iter().map(|r| r.2).collect();
  let old_doc: Vec<Vec<f32>> = rows.iter().map(|r| r.1.clone()).collect();
  let new_doc = embed_cached(new_model, DOC_PREFIX, &texts, &cache.join("new_doc.i8"));

  // Doc → doc (ambient: the cue is the user's message, embedded as stored).
  let mut rng = Rng(0x5eed);
  let n = rows.len();
  let (mut oc_, mut nc_) = (Vec::new(), Vec::new());
  let mut pairs = Vec::new();
  for _ in 0..1500 {
    let a = rng.below(n);
    let mut near: Vec<(usize, f32)> = (0..n)
      .filter(|&b| b != a)
      .map(|b| (b, cos(&old_doc[a], &old_doc[b])))
      .collect();
    near.sort_by(|x, y| y.1.total_cmp(&x.1));
    for &(b, _) in near.iter().take(40) {
      pairs.push((a, b));
    }
    for _ in 0..20 {
      let b = rng.below(n);
      if b != a {
        pairs.push((a, b));
      }
    }
  }
  for &(a, b) in &pairs {
    oc_.push(1.0 - cos(&old_doc[a], &old_doc[b]));
    nc_.push(1.0 - cos(&new_doc[a], &new_doc[b]));
  }
  println!(
    "doc→doc cosine distance: {} pairs, spearman {:.3}",
    pairs.len(),
    spearman(&oc_, &nc_)
  );
  for (name, t) in [
    ("ambient strict (established topic)", 0.55),
    ("fusion / middle", 0.60),
    ("ambient wander (novel topic)", 0.70),
  ] {
    let (q, v) = map(&oc_, &nc_, t);
    println!(
      "  {name:36} old {t:.3} (q {q:.3}) → new {v:.3}  (cosine {:.3})",
      1.0 - v
    );
  }

  // Jaccard on centered codes: the gate's and recall's bands.
  let mut oj_doc = old_doc.clone();
  apply_center(&mut oj_doc, &oc.doc_center);
  let mut nj_doc = new_doc.clone();
  apply_center(&mut nj_doc, &nc.doc_center);
  let (ocodes, ncodes) = (codes_from(&oj_doc, K), codes_from(&nj_doc, K));
  let (mut oj, mut nj) = (Vec::new(), Vec::new());
  for &(a, b) in &pairs {
    oj.push(ocodes[a].jaccard(&ocodes[b]));
    nj.push(ncodes[a].jaccard(&ncodes[b]));
  }
  println!(
    "doc→doc jaccard (k={K}): spearman {:.3}",
    spearman(&oj, &nj)
  );
  for (name, t) in [
    ("dup (strengthen)", 0.30),
    ("surface threshold", 0.18),
    ("recall anchor", 0.07),
    ("link / recall floor", 0.05),
  ] {
    let (q, v) = map(&oj, &nj, t);
    println!("  {name:36} old {t:.3} (q {q:.3}) → new {v:.3}");
  }

  // Query → doc (deliberate recall: a cue against stored content).
  let users: Vec<usize> = (0..n).filter(|&i| rows[i].0 == "user").collect();
  let mut qs: Vec<usize> = Vec::new();
  for _ in 0..800 {
    qs.push(users[rng.below(users.len())]);
  }
  qs.sort_unstable();
  qs.dedup();
  let qtexts: Vec<&str> = qs.iter().map(|&i| texts[i]).collect();
  let old_q = embed_cached(
    old_model,
    &oc.query_prefix,
    &qtexts,
    &cache.join("old_query.i8"),
  );
  let new_q = embed_cached(
    new_model,
    QUERY_PREFIX,
    &qtexts,
    &cache.join("new_query.i8"),
  );
  let _ = &oc.doc_prefix;
  let (mut oq, mut nq) = (Vec::new(), Vec::new());
  for (k, &a) in qs.iter().enumerate() {
    let mut near: Vec<(usize, f32)> = (0..n)
      .filter(|&b| b != a)
      .map(|b| (b, cos(&old_q[k], &old_doc[b])))
      .collect();
    near.sort_by(|x, y| y.1.total_cmp(&x.1));
    let mut bs: Vec<usize> = near.iter().take(40).map(|x| x.0).collect();
    for _ in 0..20 {
      bs.push(rng.below(n));
    }
    for b in bs.into_iter().filter(|&b| b != a) {
      oq.push(1.0 - cos(&old_q[k], &old_doc[b]));
      nq.push(1.0 - cos(&new_q[k], &new_doc[b]));
    }
  }
  println!(
    "query→doc cosine distance: {} pairs, spearman {:.3}",
    oq.len(),
    spearman(&oq, &nq)
  );
  for (name, t) in [("fusion (deliberate)", 0.60), ("deliberate floor", 0.65)] {
    let (q, v) = map(&oq, &nq, t);
    println!(
      "  {name:36} old {t:.3} (q {q:.3}) → new {v:.3}  (cosine {:.3})",
      1.0 - v
    );
  }
  // The old model's query center is unused: cosine runs on raw vectors.
  let _ = &oc.query_center;
}
