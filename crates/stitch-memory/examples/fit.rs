//! Fits engram to EmbeddingGemma 2 as Stitch runs it (llama.cpp, Q8_0,
//! int8 storage): the frozen centering artifact, the anisotropy health
//! check, and calibrated jaccard bands, with engram-fit's own math.
//!
//! STITCH_EMBED_MODEL=…/embeddinggemma-2-Q8_0.gguf \
//!   cargo run --release -p stitch-memory --example fit -- assets/embeddinggemma-2.center.json

use engram_fit::{
  PairLabel, apply_center, build_artifact, calibrate, codes_from, default_pack,
  int8_unit_round_trip, pairwise_jaccard_stats, parse_pairs, verdict,
};
use stitch_llama::{Embedder, Options};
use stitch_memory::{DOC_PREFIX, ENGINE, K, MODEL, QUERY_PREFIX};

fn embed(e: &mut Embedder, prefix: &str, texts: &[String]) -> Vec<Vec<f32>> {
  let mut out: Vec<Vec<f32>> = texts
    .iter()
    .map(|t| e.embed(&format!("{prefix}{t}")).expect("embeds"))
    .collect();
  // What production gates on: int8 round-tripped values.
  int8_unit_round_trip(&mut out);
  out
}

fn main() {
  let model = std::env::var("STITCH_EMBED_MODEL").expect("set STITCH_EMBED_MODEL");
  let out = std::env::args().nth(1).expect("the artifact's path");
  let mut e = Embedder::load(model.as_ref(), None, &Options::default()).expect("loads");

  let pack = default_pack();
  eprintln!(
    "embedding {} sentences twice (doc + query prefixes)…",
    pack.len()
  );
  let doc = embed(&mut e, DOC_PREFIX, &pack);
  let query = embed(&mut e, QUERY_PREFIX, &pack);
  let art = build_artifact(ENGINE, MODEL, DOC_PREFIX, QUERY_PREFIX, &doc, &query).unwrap();
  std::fs::write(&out, serde_json::to_string(&art).unwrap()).unwrap();
  println!(
    "wrote {out}: dim {}, {} samples, tag {:#018x}",
    art.dim, art.samples, art.tag_u64
  );

  let raw = pairwise_jaccard_stats(&codes_from(&doc, K));
  let mut centered = doc.clone();
  apply_center(&mut centered, &art.doc_center);
  let cen = pairwise_jaccard_stats(&codes_from(&centered, K));
  println!(
    "health (k={K}): raw mean {:.3}, centered mean {:.3} p95 {:.3} max {:.3}",
    raw.mean, cen.mean, cen.p95, cen.max
  );
  println!("verdict: {}", verdict(cen.mean));

  // engram-fit's labelled pairs, read at run time: engram is a git
  // dependency, so its assets are not beside this checkout unless it is
  // `just dev-link`ed (`ENGRAM_PAIRS` names the file otherwise).
  let pairs_path = std::env::var("ENGRAM_PAIRS").unwrap_or_else(|_| {
    concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../../../engram/tools/engram-fit/assets/pairs.tsv"
    )
    .to_string()
  });
  let pairs_text = std::fs::read_to_string(&pairs_path).unwrap_or_else(|e| {
    panic!("{pairs_path}: {e} (set ENGRAM_PAIRS to engram-fit's assets/pairs.tsv)")
  });
  let pairs = parse_pairs(&pairs_text).unwrap();
  let texts: Vec<String> = pairs
    .iter()
    .flat_map(|p| [p.a.clone(), p.b.clone()])
    .collect();
  let mut embs = embed(&mut e, DOC_PREFIX, &texts);
  apply_center(&mut embs, &art.doc_center);
  let codes = codes_from(&embs, K);
  let (mut dup, mut related, mut unrelated) = (Vec::new(), Vec::new(), Vec::new());
  for (i, p) in pairs.iter().enumerate() {
    let j = codes[2 * i].jaccard(&codes[2 * i + 1]);
    match p.label {
      PairLabel::Dup => dup.push(j),
      PairLabel::Related => related.push(j),
      PairLabel::Unrelated => unrelated.push(j),
    }
  }
  let cal = calibrate(dup, related, unrelated);
  for (name, s) in [
    ("dup", &cal.dup),
    ("related", &cal.related),
    ("unrelated", &cal.unrelated),
  ] {
    println!(
      "{name:9} mean {:.3} p50 {:.3} p95 {:.3} max {:.3} ({} pairs)",
      s.mean, s.p50, s.p95, s.max, s.pairs
    );
  }
  println!(
    "suggested: dup {:.3}, link {:.3}, recall_anchor {:.3}",
    cal.suggested_dup, cal.suggested_link, cal.suggested_anchor
  );

  // Cosine of a query-role cue against stored (doc-role) content, raw
  // int8 vectors: the cortex check recall verifies candidates with.
  let a_texts: Vec<String> = pairs.iter().map(|p| p.a.clone()).collect();
  let b_texts: Vec<String> = pairs.iter().map(|p| p.b.clone()).collect();
  let qa = embed(&mut e, QUERY_PREFIX, &a_texts);
  let db = embed(&mut e, DOC_PREFIX, &b_texts);
  let (mut cd, mut cr, mut cu) = (Vec::new(), Vec::new(), Vec::new());
  for (i, p) in pairs.iter().enumerate() {
    let c: f32 = qa[i].iter().zip(&db[i]).map(|(x, y)| x * y).sum();
    match p.label {
      PairLabel::Dup => cd.push(c),
      PairLabel::Related => cr.push(c),
      PairLabel::Unrelated => cu.push(c),
    }
  }
  for (name, mut v) in [("dup", cd), ("related", cr), ("unrelated", cu)] {
    v.sort_by(f32::total_cmp);
    let q = |f: f32| v[((v.len() - 1) as f32 * f) as usize];
    println!(
      "cosine query→doc {name:9} min {:.3} p25 {:.3} p50 {:.3} p75 {:.3} max {:.3}",
      q(0.0),
      q(0.25),
      q(0.5),
      q(0.75),
      q(1.0)
    );
  }
}
