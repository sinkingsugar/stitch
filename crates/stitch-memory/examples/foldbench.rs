//! What memory's fold costs as history grows (design §2.9, "As built"):
//! synthetic histories shaped like the field's (`stitch_memory::synth`) of
//! 10k, 50k and 200k facts; a full rebuild, folding one late fact, an
//! observe on top, and ambient and asked recall.
//!
//! `cargo run --release -p stitch-memory --example foldbench [sizes…]`

use std::time::{Duration, Instant};

use stitch_memory::synth::{self, near, vector};
use stitch_memory::{Fact, Fold, ambient_ceiling, handle};

fn ms(d: Duration) -> String {
  format!("{:.3} ms", d.as_secs_f64() * 1e3)
}

fn main() {
  let sizes: Vec<usize> = std::env::args()
    .skip(1)
    .filter_map(|a| a.parse().ok())
    .collect();
  let sizes = if sizes.is_empty() {
    vec![10_000, 50_000, 200_000]
  } else {
    sizes
  };
  for n in sizes {
    let t = Instant::now();
    // The history, and 200 more facts to observe on top.
    let (all, _) = synth::history(n + 200, 7);
    let gen_time = t.elapsed();
    let cut = all
      .iter()
      .position(|(at, _)| at.secs >= all[n].0.secs)
      .unwrap_or(all.len());
    let (past, future) = all.split_at(cut);
    let past = past.to_vec();

    // A full rebuild.
    let t = Instant::now();
    let fold = Fold::rebuild(past.iter().cloned());
    let rebuild = t.elapsed();
    let (cortex, log) = fold.sizes();
    let decisions = synth::decisions(&fold);

    // One late fact: a record about a day back (another node's exchange,
    // synced late), the rest folded already.
    let late_at = past.len().saturating_sub(250);
    let late_rec = past[late_at].0.record.clone();
    let (late, rest): (Vec<_>, Vec<_>) = past
      .iter()
      .cloned()
      .partition(|(at, _)| at.record == late_rec);
    let mut folding = stitch_memory::Folding::new();
    folding.add(rest);
    let t = Instant::now();
    folding.add(late);
    let late_time = t.elapsed();
    assert_eq!(folding.fold().len(), fold.len());
    let checkpoint = folding.fold().to_bytes().len();

    // A start: every fact read again, the newest checkpoint on disk.
    let dir = tempfile::tempdir().expect("a temporary directory");
    stitch_memory::Folding::open(dir.path(), "bench").add(past.clone());
    let mut start = stitch_memory::Folding::open(dir.path(), "bench");
    let t = Instant::now();
    start.add(past.clone());
    let start_time = t.elapsed();
    assert_eq!(start.fold().len(), fold.len());

    // Observes on top.
    let mut on_top = Fold::rebuild(past.iter().cloned());
    let observes: Vec<_> = future
      .iter()
      .filter(|(_, f)| matches!(f, Fact::Observe { .. }))
      .cloned()
      .collect();
    let mut k = 0;
    let t = Instant::now();
    for (at, f) in future {
      if matches!(f, Fact::Observe { .. }) {
        k += 1;
      }
      on_top.apply(at, f);
    }
    let observe = t.elapsed() / k.max(1);

    // Ambient recall: the cue peeked (its novelty), the cortex's proposals
    // under its ceiling, eligible ones only; asked recall: the tool's.
    let cues: Vec<Vec<f32>> = observes
      .iter()
      .take(50)
      .map(|(_, f)| match f {
        Fact::Observe { embedding, .. } => near(&stitch_memory::unpack(embedding), 99, 0.2),
        _ => vector(1),
      })
      .collect();
    let t = Instant::now();
    let mut proposed = 0;
    for (i, cue) in cues.iter().enumerate() {
      let a = fold.peek(&format!("home:cue#m{i}"), cue, 0.3);
      let (ceiling, _) = ambient_ceiling(a.novelty);
      proposed += fold
        .cortex(cue, ceiling, stitch_memory::PROPOSER_TOP, &|h| {
          fold.eligible(h, Some("cue"))
        })
        .len();
    }
    let ambient = t.elapsed() / cues.len() as u32;
    let t = Instant::now();
    let mut hits = 0;
    for cue in &cues {
      hits += fold.recall(cue, &|h| fold.eligible(h, None)).len();
    }
    let asked = t.elapsed() / cues.len() as u32;
    let _ = handle("");

    println!(
      "{n} facts ({} folded; generated in {:.1} s): {} episodes, cortex {cortex}, log {log} ({} forgotten); {decisions:?}",
      past.len(),
      gen_time.as_secs_f64(),
      fold.episodes(),
      fold.strengths().1,
    );
    println!(
      "  rebuild {}  late fact {}  start {}  observe {}  ambient {} ({proposed} proposed)  asked {} ({hits} hits)  checkpoint {:.1} MB",
      ms(rebuild),
      ms(late_time),
      ms(start_time),
      ms(observe),
      ms(ambient),
      ms(asked),
      checkpoint as f64 / 1e6,
    );
  }
}
