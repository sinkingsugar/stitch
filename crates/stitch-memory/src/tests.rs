use super::*;
use crate::synth::{near, vector};

fn place(thread: u32, m: u64) -> String {
  format!("home:t{thread}#m{m}")
}

fn observe(p: &str, v: &[f32]) -> Fact {
  Fact::Observe {
    place: p.to_owned(),
    embedding: pack(v),
    demand: 0.0,
    own: false,
    supersedes: false,
  }
}

fn at(record: &str, i: u32) -> At {
  At::new(record, i, 0)
}

const DAY: i64 = 86_400;

#[test]
fn the_center_and_codes_are_what_the_fit_made() {
  let c = center();
  assert_eq!(c.dim, 768);
  assert_eq!(c.doc_center.len(), 768);
  assert_eq!(c.query_center.len(), 768);
  let v = vector(1);
  assert_eq!(unpack(&pack(&v)), v);
  assert_eq!(handle("home:t1#m2"), handle("home:t1#m2"));
  assert_ne!(handle("home:t1#m2"), handle("home:t1#m3"));
  for len in [0, 1, 2, 3, 767, 768] {
    let bytes: Vec<i8> = (0..len).map(|i: i32| (i * 37 % 255 - 127) as i8).collect();
    assert_eq!(from_base64(&to_base64(&bytes)), Some(bytes));
  }
  assert_eq!(from_base64("not base64!"), None);
  assert_eq!(thread_of("home:t1#m2:a0"), Some("t1"));
  assert_eq!(thread_of(&gist_place(7)), None);
}

#[test]
fn every_order_of_arrival_folds_to_the_same_memory() {
  let facts: Vec<(At, Fact)> = (0..80u64)
    .map(|i| {
      let fact = match i % 20 {
        19 => Fact::Credit {
          shown: vec![handle(&place(1, i - 3)), handle(&place(1, i - 5))],
          used: vec![handle(&place(1, i - 3))],
        },
        17 => Fact::Penalize {
          handles: vec![handle(&place(1, i - 2))],
        },
        13 => Fact::Sleep,
        11 => Fact::Remember {
          thread: "t2".into(),
          on: i > 40,
          recall: i % 3 == 0,
        },
        _ => observe(&place(1 + (i % 2) as u32, i), &vector(i % 25)),
      };
      (
        At::new(format!("2026-10-12T00{i:04}Z-mbp-aaaa"), 0, i as i64 * 3600),
        fact,
      )
    })
    .collect();
  let mut in_order = Fold::new();
  for (a, f) in &facts {
    assert!(in_order.follows(a));
    in_order.apply(a, f);
  }
  let mut shuffled = facts.clone();
  shuffled.reverse();
  shuffled.swap(3, 40);
  let rebuilt = Fold::rebuild(shuffled);
  assert_eq!(rebuilt.len(), in_order.len());
  assert_eq!(rebuilt.episodes(), in_order.episodes());
  for i in 0..80 {
    let h = handle(&place(1 + (i % 2) as u32, i));
    assert_eq!(rebuilt.stats(h), in_order.stats(h), "handle {i}");
    assert_eq!(rebuilt.resolve(h), in_order.resolve(h));
  }
  let cue = vector(7);
  assert_eq!(
    rebuilt.recall(&cue, &|_| true),
    in_order.recall(&cue, &|_| true)
  );
  assert_eq!(rebuilt.bands(0), in_order.bands(0));
  // Something older than what was folded cannot go on top.
  assert!(!in_order.follows(&at("2026-10-12T000001Z-mbp-aaaa", 0)));
}

#[test]
fn recall_finds_what_is_close_and_nothing_else() {
  let mut fold = Fold::new();
  let topic = vector(100);
  for h in 0..30u64 {
    fold.apply(
      &at(&format!("r{h:03}"), 0),
      &observe(&place(1, h), &vector(1000 + h)),
    );
  }
  let target = place(2, 0);
  fold.apply(&at("r100", 0), &observe(&target, &near(&topic, 5, 0.3)));
  let th = handle(&target);
  let asked = fold.recall(&near(&topic, 9, 0.3), &|_| true);
  assert_eq!(asked.first().map(|h| h.handle), Some(th), "{asked:?}");
  // Each memory is within the relevance floor, or linked to one that is.
  for h in &asked {
    let linked = fold.entry(h.handle).and_then(|e| e.target) == Some(th)
      || fold.entry(th).and_then(|e| e.target) == Some(h.handle);
    assert!(
      1.0 - h.cosine <= DELIBERATE_FLOOR_DISTANCE || linked,
      "{h:?}"
    );
  }
  // Unrelated cues bring nothing back: an off-topic store returns nothing.
  assert!(fold.recall(&vector(424_242), &|_| true).is_empty());
  // A hidden handle (the current thread's) is never returned.
  assert!(
    fold
      .recall(&near(&topic, 9, 0.3), &|h| h != th)
      .iter()
      .all(|h| h.handle != th)
  );
  assert!(!fold.eligible(th, Some("t2")));
  assert!(fold.eligible(th, Some("t1")));
  // The cortex filters before its limit: hiding the closest still finds
  // the next.
  let keep_all = fold.cortex(&near(&topic, 9, 0.3), 1.0, 1, &|_| true);
  let hidden = fold.cortex(&near(&topic, 9, 0.3), 1.0, 1, &|h| h != th);
  assert_eq!(keep_all[0].0, th);
  assert_eq!(hidden.len(), 1);
  assert_ne!(hidden[0].0, th);
  // Asking does not change the memory.
  let before = fold.recall(&near(&topic, 9, 0.3), &|_| true);
  assert_eq!(fold.recall(&near(&topic, 9, 0.3), &|_| true), before);
}

#[test]
fn the_note_ceiling_follows_novelty() {
  assert_eq!(ambient_ceiling(0.2), (AMBIENT_STRICT_DISTANCE, false));
  assert_eq!(ambient_ceiling(0.8), (AMBIENT_MIDDLE_DISTANCE, false));
  assert_eq!(ambient_ceiling(0.99), (AMBIENT_WANDER_DISTANCE, true));
  let mut fold = Fold::new();
  let v = vector(3);
  fold.apply(&at("r1", 0), &observe(&place(1, 1), &v));
  // A near-duplicate cue is established territory; an unrelated one new.
  let familiar = fold.peek(&place(2, 1), &near(&v, 4, 0.02), 0.0);
  assert!(familiar.novelty <= ESTABLISHED_NOVELTY, "{familiar:?}");
  let novel = fold.peek(&place(2, 2), &vector(999), 0.0);
  assert!(novel.novelty > familiar.novelty);
  // Peeking changes nothing.
  assert_eq!(fold.len(), 1);
  assert!(!fold.knows(handle(&place(2, 1))));
}

#[test]
fn a_newer_number_supersedes_and_a_sibling_keeps_its_own_memory() {
  let mut fold = Fold::new();
  for h in 0..20u64 {
    fold.apply(
      &at(&format!("a{h:02}"), 0),
      &observe(&place(9, h), &vector(500 + h)),
    );
  }
  let dose = vector(42);
  let old = place(1, 1);
  fold.apply(&at("b1", 0), &observe(&old, &dose));
  // The same fact, updated: a Strengthen that supersedes.
  let new = place(3, 7);
  let probe = fold.probe(&[(new.clone(), near(&dose, 43, 0.02), 0.0)], |_| false);
  assert_eq!(probe[0].0.kind, Kind::Strengthen);
  assert_eq!(probe[0].0.target, Some(handle(&old)));
  let a = fold
    .apply(
      &at("b2", 0),
      &Fact::Observe {
        place: new.clone(),
        embedding: pack(&near(&dose, 43, 0.02)),
        demand: 0.0,
        own: false,
        supersedes: true,
      },
    )
    .expect("applied");
  assert_eq!(a, probe[0].0);
  assert_eq!(fold.resolve(handle(&old)), Some(new.as_str()));
  // Recalling the old memory into the newer one's thread shows nothing.
  assert!(!fold.eligible(handle(&old), Some("t3")));
  // A sibling the gate would absorb keeps a memory of its own.
  let sib = place(1, 2);
  let probe = fold.probe(&[(sib.clone(), near(&dose, 44, 0.02), 0.0)], |a| {
    a.target == Some(handle(&old))
  });
  assert!(probe[0].1, "{probe:?}");
  assert_eq!(probe[0].0.kind, Kind::Link);
  fold.apply(
    &at("b3", 0),
    &Fact::Observe {
      place: sib.clone(),
      embedding: pack(&near(&dose, 44, 0.02)),
      demand: 0.0,
      own: true,
      supersedes: false,
    },
  );
  assert!(fold.stats(handle(&sib)).is_some());
  assert_eq!(fold.entry(handle(&sib)).map(|e| e.kind), Some(Kind::Link));
  // The same place twice (two nodes healing one hole) counts once.
  let n = fold.episodes();
  assert!(
    fold
      .apply(&at("b4", 0), &observe(&sib, &vector(77)))
      .is_none()
  );
  assert_eq!(fold.episodes(), n);
}

#[test]
fn credit_dismissal_and_the_sleep_audit() {
  let mut fold = Fold::new();
  for h in 0..10u64 {
    fold.apply(
      &at(&format!("r{h}"), 0),
      &observe(&place(1, h), &vector(h + 50)),
    );
  }
  let (helped, freeloader, dismissed) = (
    handle(&place(1, 4)),
    handle(&place(1, 5)),
    handle(&place(1, 6)),
  );
  assert_eq!(fold.stats(helped).map(|s| s.used), Some(false));
  // Shown three times, helped once; the freeloader never.
  for i in 0..3 {
    fold.apply(
      &At::new(format!("s{i}"), 0, i * DAY),
      &Fact::Credit {
        shown: vec![helped, freeloader],
        used: if i == 0 { vec![helped] } else { Vec::new() },
      },
    );
  }
  assert_eq!(fold.stats(helped).map(|s| s.used), Some(true));
  assert_eq!(fold.shown(freeloader), 3);
  fold.apply(
    &At::new("t0", 0, 3 * DAY),
    &Fact::Penalize {
      handles: vec![dismissed],
    },
  );
  let s = |h| fold.stats(h).expect("alive").strength;
  assert!(s(dismissed) < s(handle(&place(1, 7))));
  let before = s(handle(&place(1, 7)));
  fold.apply(&At::new("u0", 0, 4 * DAY), &Fact::Sleep);
  let s = |h| fold.stats(h).expect("alive").strength;
  // The audit knocked the freeloader before the decay; the one that
  // helped was strengthened and spared.
  assert!(s(freeloader) < s(handle(&place(1, 7))));
  assert!(s(helped) > s(handle(&place(1, 7))));
  assert!(s(handle(&place(1, 7))) < before);
  // Never reinforced, a memory is forgotten after enough sleeps, and is
  // never recalled again.
  for i in 1..12 {
    fold.apply(&At::new(format!("u{i:02}"), 0, (4 + i) * DAY), &Fact::Sleep);
  }
  assert!(fold.stats(freeloader).is_none());
  assert!(fold.resolve(freeloader).is_none());
  assert!(!fold.eligible(freeloader, None));
  let (_, forgotten) = fold.strengths();
  assert!(forgotten >= 1);
}

#[test]
fn quiet_clusters_become_gists() {
  let mut fold = Fold::new();
  let topic = vector(7000);
  // A cluster of linked memories, a fortnight old.
  for i in 0..7u64 {
    fold.apply(
      &At::new(format!("c{i}"), 0, i as i64),
      &observe(&place(4, i), &near(&topic, 7100 + i, 0.45)),
    );
  }
  let linked = (0..7u64)
    .filter(|i| fold.entry(handle(&place(4, *i))).map(|e| e.kind) == Some(Kind::Link))
    .count();
  assert!(linked >= 4, "the cluster links up ({linked})");
  assert!(fold.gist_due(3 * DAY).is_empty(), "not quiet yet");
  let due = fold.gist_due(14 * DAY);
  assert!(due.len() >= GIST_MIN_MEMBERS, "{due:?}");
  let (kept, dropped) = (due[..due.len() - 1].to_vec(), vec![due[due.len() - 1]]);
  let text = "The aeroponics rig: misting every fifteen minutes, reservoir weekly.";
  fold.apply(
    &At::new("g1", 0, 14 * DAY),
    &Fact::Gist {
      text: text.into(),
      embedding: pack(&topic),
      kept: kept.clone(),
      dropped: dropped.clone(),
    },
  );
  let gh = handle(&format!("gist:{text}"));
  assert_eq!(fold.gist(gh), Some(text));
  for h in &kept {
    assert_eq!(fold.resolve(*h), Some(gist_place(gh).as_str()));
    assert!(fold.stats(*h).is_none());
  }
  assert!(fold.resolve(dropped[0]).is_none());
  assert!(fold.stats(gh).is_some());
  assert!(fold.eligible(gh, Some("t4")));
  // A cluster already condensed is not due again.
  assert!(fold.gist_due(15 * DAY).is_empty());
}

#[test]
fn a_thread_out_of_memory_is_never_recalled() {
  let mut fold = Fold::new();
  let v = vector(11);
  fold.apply(&at("r1", 0), &observe(&place(5, 1), &v));
  let h = handle(&place(5, 1));
  assert!(fold.eligible(h, None));
  assert!(!fold.is_switched("t5"));
  fold.apply(
    &at("r2", 0),
    &Fact::Remember {
      thread: "t5".into(),
      on: false,
      recall: false,
    },
  );
  assert!(fold.is_off("t5"));
  assert!(!fold.may_recall("t5"));
  assert!(!fold.eligible(h, None));
  fold.apply(
    &at("r3", 0),
    &Fact::Remember {
      thread: "t5".into(),
      on: true,
      recall: false,
    },
  );
  assert!(fold.may_recall("t5"), "in memory, it recalls");
  assert!(fold.eligible(h, None));
  // Switched back on, still switched: a place's default no longer applies.
  assert!(fold.is_switched("t5"));
}

#[test]
fn a_thread_out_of_memory_may_still_recall_when_told() {
  let mut fold = Fold::new();
  let v = vector(12);
  fold.apply(&at("r1", 0), &observe(&place(6, 1), &v));
  let h = handle(&place(6, 1));
  let off = |recall| Fact::Remember {
    thread: "t6".into(),
    on: false,
    recall,
  };
  // Out of memory, recalling (a spawned thread's default): nothing of it
  // is shown to others, but it searches what others left.
  fold.apply(&at("r2", 0), &off(true));
  assert!(fold.is_off("t6"));
  assert!(fold.may_recall("t6"));
  assert!(!fold.eligible(h, None), "still never shown to others");
  // `/memory off` afterwards: no recall either.
  fold.apply(&at("r3", 0), &off(false));
  assert!(!fold.may_recall("t6"));
  // A thread nobody switched recalls.
  assert!(fold.may_recall("t7"));
}

/// A synthetic history in batches of records, in an order of arrival
/// shuffled by `seed`: the newest records mostly come in order, some
/// late.
fn arrivals(facts: &[(At, Fact)], seed: u64) -> Vec<Vec<(At, Fact)>> {
  let mut records: Vec<Vec<(At, Fact)>> = Vec::new();
  for (at, f) in facts {
    match records.last_mut() {
      Some(r) if r[0].0.record == at.record => r.push((at.clone(), f.clone())),
      _ => records.push(vec![(at.clone(), f.clone())]),
    }
  }
  let mut x = seed;
  let mut late: Vec<Vec<(At, Fact)>> = Vec::new();
  let mut batches: Vec<Vec<(At, Fact)>> = Vec::new();
  for r in records {
    x = crate::synth::splitmix(x);
    if x.is_multiple_of(7) {
      // Held back: another node's, synced later.
      late.push(r);
    } else {
      batches.push(r);
    }
    if x.is_multiple_of(11) && !late.is_empty() {
      batches.push(late.drain(..).flatten().collect());
    }
  }
  batches.push(late.into_iter().flatten().collect());
  batches.retain(|b| !b.is_empty());
  batches
}

#[test]
fn checkpoints_fold_bit_for_bit_in_any_order() {
  let (facts, made) = crate::synth::history(1_400, 3);
  let whole = Fold::rebuild(facts.clone());
  assert!(
    whole.to_bytes() == made.to_bytes(),
    "in order, from scratch"
  );
  let sleeps = facts.iter().filter(|(_, f)| *f == Fact::Sleep).count();
  assert!(sleeps >= 4, "{sleeps} sleeps");
  for seed in [1, 2, 3] {
    let mut folding = Folding::new();
    let mut from_checkpoint = 0;
    for batch in arrivals(&facts, seed) {
      let late = !folding.fold().follows(&batch[0].0);
      folding.add(batch);
      if late && folding.last_fold().0 > 0 {
        from_checkpoint += 1;
      }
      assert!(folding.checkpoints().len() <= crate::folding::CHECKPOINTS);
    }
    assert!(from_checkpoint > 0, "late facts fold from a checkpoint");
    assert!(folding.facts() == &facts[..]);
    assert!(folding.fold().to_bytes() == whole.to_bytes(), "seed {seed}");
  }
}

#[test]
fn a_checkpoint_comes_back_byte_for_byte() {
  let (facts, fold) = crate::synth::history(700, 5);
  let bytes = fold.to_bytes();
  let back = Fold::from_bytes(&bytes).expect("reads back");
  assert!(back.to_bytes() == bytes);
  assert!(Fold::from_bytes(&bytes[..bytes.len() - 1]).is_none());
  // Restored at a sleep, it folds on as the original does.
  let at = facts.iter().rposition(|(_, f)| *f == Fact::Sleep).unwrap() + 1;
  let mut a = Fold::rebuild(facts[..at].iter().cloned());
  let mut b = Fold::from_bytes(&a.to_bytes()).unwrap();
  let (more, _) = crate::synth::history(1_000, 5);
  for (t, f) in &more[at..] {
    assert_eq!(a.apply(t, f), b.apply(t, f));
  }
  assert!(a.to_bytes() == b.to_bytes());
}

#[test]
fn checkpoints_on_disk_serve_a_start_and_go_with_another_build() {
  let dir = tempfile::tempdir().unwrap();
  let (facts, _) = crate::synth::history(1_100, 9);
  let whole = Fold::rebuild(facts.clone());
  {
    let mut folding = Folding::open(dir.path(), "build-a");
    folding.add(facts.clone());
    assert_eq!(folding.checkpoints().len(), crate::folding::CHECKPOINTS);
  }
  // A start: everything is new, and the newest checkpoint holds most.
  let mut folding = Folding::open(dir.path(), "build-a");
  let newest = *folding.checkpoints().last().unwrap();
  folding.add(facts.clone());
  assert_eq!(folding.last_fold(), (newest, facts.len() - newest));
  assert!(folding.fold().to_bytes() == whole.to_bytes());
  // A thread deleted: its observations go, folded again from before them.
  let gone = facts
    .iter()
    .rev()
    .find_map(|(_, f)| match f {
      Fact::Observe { place, .. } => thread_of(place).map(str::to_owned),
      _ => None,
    })
    .unwrap();
  let keep = |_: &At, f: &Fact| !matches!(f, Fact::Observe { place, .. } if thread_of(place) == Some(gone.as_str()));
  folding.retain(keep);
  let kept: Vec<(At, Fact)> = facts.iter().filter(|(a, f)| keep(a, f)).cloned().collect();
  assert!(folding.last_fold().0 > 0, "from a checkpoint");
  assert!(folding.fold().to_bytes() == Fold::rebuild(kept).to_bytes());
  // Another build discards them.
  let other = Folding::open(dir.path(), "build-b");
  assert!(other.checkpoints().is_empty());
  assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn the_cortex_follows_what_is_remembered() {
  let (facts, fold) = crate::synth::history(1_400, 4);
  let (cortex, log) = fold.sizes();
  let (_, forgotten) = fold.strengths();
  assert!(forgotten > 0);
  // Nothing forgotten is scanned; every scanned memory is eligible.
  for (at, f) in &facts {
    if let Fact::Observe { place, .. } = f {
      let h = handle(place);
      let e = fold.entry(h).unwrap();
      if e.forgotten {
        assert_eq!(fold.distance(&vector(1), h), None, "{at:?}");
      }
    }
  }
  assert!(cortex + forgotten <= log, "{cortex} + {forgotten} > {log}");
  // A Strengthen goes with the memory it merged into.
  let mut f = Fold::new();
  let v = vector(31);
  f.apply(&at("a", 0), &observe(&place(1, 1), &v));
  f.apply(&at("b", 0), &observe(&place(2, 1), &near(&v, 32, 0.02)));
  let (t, s) = (handle(&place(1, 1)), handle(&place(2, 1)));
  assert_eq!(f.entry(s).map(|e| e.kind), Some(Kind::Strengthen));
  assert_eq!(f.sizes().0, 2);
  for i in 0..40 {
    f.apply(&At::new(format!("c{i:02}"), 0, i * DAY), &Fact::Sleep);
  }
  assert!(f.entry(t).unwrap().forgotten);
  assert!(f.entry(s).unwrap().forgotten, "merged, forgotten with it");
  assert!(!f.eligible(s, None));
  assert_eq!(f.sizes().0, 0);
  // A thread out of memory waits aside, and comes back.
  let mut f = Fold::new();
  f.apply(&at("a", 0), &observe(&place(5, 1), &v));
  let h = handle(&place(5, 1));
  let off = |on| Fact::Remember {
    thread: "t5".into(),
    on,
    recall: false,
  };
  f.apply(&at("b", 0), &off(false));
  assert!(f.cortex(&v, 1.0, 5, &|_| true).is_empty());
  assert!(f.distance(&v, h).is_some(), "kept aside");
  f.apply(&at("c", 0), &off(true));
  assert_eq!(f.cortex(&v, 1.0, 5, &|_| true)[0].0, h);
}
