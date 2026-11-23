//! Loose ends' judgement by the decision model against the quick model's
//! (design §2.29), on the user's real conversations: `just
//! loose-ends-eval` (the `gemini` key from the node's secrets; the data
//! directory `STITCH_EVAL_DATA`, `~/.stitch` by default, read only: the
//! index is built anew in a temporary directory). Every judgement the
//! quick model wrote (`loose-end` sets) is asked again of the decision
//! model over the conversation as it was at the step judged. Not in CI.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use futures_util::StreamExt;
use serde_json::{Value, json};
use stitch_providers::{Gemini, Provider};
use stitch_records::{NodeName, RecordId, kinds};
use stitch_store::{BlobStore, Index, RecordRoot};

use super::*;

/// A record ID's second (`2026-11-14T001932Z-…`).
fn id_time(id: &RecordId) -> Option<Timestamp> {
  let head = id.as_str().split('-').take(3).collect::<Vec<_>>().join("-");
  jiff::civil::DateTime::strptime("%Y-%m-%dT%H%M%SZ", &head)
    .ok()?
    .to_zoned(jiff::tz::TimeZone::UTC)
    .ok()
    .map(|z| z.timestamp())
}

struct Case {
  thread: RecordId,
  title: String,
  step: Timestamp,
  old: Verdict,
  old_line: String,
  head: RecordId,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real conversations and the real Decisions API: just loose-ends-eval"]
async fn loose_ends_eval() {
  let key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY (just loose-ends-eval)");
  let data = std::env::var_os("STITCH_EVAL_DATA")
    .map(PathBuf::from)
    .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join(".stitch"));
  let limit: usize = std::env::var("STITCH_EVAL_N")
    .ok()
    .and_then(|n| n.parse().ok())
    .unwrap_or(400);
  let tmp = tempfile::tempdir().unwrap();
  let mut index = Index::open(&tmp.path().join("index.sqlite")).unwrap();
  let mut roots = Vec::new();
  let mut dirs = vec![data.join("home")];
  if let Ok(d) = std::fs::read_dir(data.join("projects")) {
    dirs.extend(d.filter_map(|e| e.ok().map(|e| e.path())));
  }
  for dir in dirs {
    if let Ok(r) = RecordRoot::open(&dir) {
      index.refresh(&r).unwrap();
      roots.push(r);
    }
  }
  let index = Mutex::new(index);
  let blobs = BlobStore::open(&data.join("blobs")).unwrap();
  let node: NodeName = "gamer".parse().unwrap();

  // Every quick-model judgement, and the segment it judged.
  let mut cases = Vec::new();
  {
    let ix = index.lock().unwrap();
    let threads: Vec<_> = ix
      .records(Some(kinds::THREAD))
      .unwrap()
      .into_iter()
      .filter(is_conversation)
      .collect();
    for t in threads {
      let sets = ix.sources(kinds::SET, "target", &t.id, None).unwrap();
      let segs = ix.thread_segments(&t.id).unwrap_or_default();
      let title = ix.title(&t.id).ok().flatten().unwrap_or_default();
      for s in sets {
        if s.fields.get("field").and_then(Value::as_str) != Some(JUDGED) {
          continue;
        }
        let v = &s.fields["value"];
        let model = v.get("model").and_then(Value::as_str).unwrap_or("");
        if model.contains("decision") {
          continue;
        }
        let (Some(old), Some(step)) = (
          v.get("verdict")
            .and_then(Value::as_str)
            .and_then(Verdict::parse),
          v.get("step").and_then(stamp_of),
        ) else {
          continue;
        };
        // The latest segment written by then (its ID's second).
        let floor = Timestamp::from_second(step.as_second()).unwrap();
        let Some(head) = segs
          .iter()
          .filter(|(id, _, _)| id_time(id).is_some_and(|t| t <= floor))
          .map(|(id, _, _)| id.clone())
          .next_back()
        else {
          continue;
        };
        cases.push(Case {
          thread: t.id.clone(),
          title: title.clone(),
          step,
          old,
          old_line: v
            .get("line")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
          head,
        });
      }
    }
  }
  cases.sort_by_key(|c| std::cmp::Reverse(c.step));
  cases.truncate(limit);
  println!("{} judgements by the quick model to compare", cases.len());

  // Their excerpts, as the pass reads them.
  let mut inputs = Vec::new();
  for c in &cases {
    let Some(root) = roots.iter().find(|r| r.read_record(&c.thread).is_ok()) else {
      continue;
    };
    let ctx = ChatContext {
      node: &node,
      root,
      index: &index,
      blobs: &blobs,
    };
    let convo = Convo {
      title: c.title.clone(),
      ..Convo::bare(c.thread.clone(), "gamer".into(), c.step)
    };
    if let Ok(e) = excerpt_at(&ctx, &convo, Some(&c.head)) {
      inputs.push((c, e));
    }
  }

  let provider = Provider::Gemini(Gemini::new(key));
  let profile = crate::decide::profiles::named(crate::decide::profiles::LOOSE_ENDS).unwrap();
  let questions = profile.questions();
  let model = crate::decide::DEFAULT_MODEL.split_once('/').unwrap().1;
  let answered: Vec<_> = futures_util::stream::iter(inputs.iter())
    .map(|(c, e)| {
      let (provider, questions) = (&provider, &questions);
      async move {
        let input = decision_input(&e.text);
        let started = std::time::Instant::now();
        let mut last = String::new();
        for attempt in 0..4 {
          match crate::decide::call_provider(
            provider,
            "gemini",
            model,
            &input,
            questions,
            std::time::Duration::from_secs(10),
          )
          .await
          {
            Ok((a, s)) => return (c, e, Ok((a, s, started.elapsed().as_millis() as u64))),
            Err(err) => last = err,
          }
          tokio::time::sleep(std::time::Duration::from_millis(400 << attempt)).await;
        }
        (c, e, Err(last))
      }
    })
    .buffer_unordered(8)
    .collect()
    .await;

  let mut confusion: BTreeMap<(&str, &str), usize> = BTreeMap::new();
  let (mut n, mut exact, mut waits, mut open) = (0, 0, 0, 0);
  let mut by_t: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
  let (mut usd, mut ms, mut errors) = (0.0, Vec::new(), 0);
  let mut rows = Vec::new();
  let mut disagree = Vec::new();
  // Taken at the profile's thresholds: how many, agreeing, quick-model
  // waits the decision model would have called otherwise.
  let mut profile_taken = (0usize, 0usize, 0usize);
  for (c, e, r) in &answered {
    let Ok((answers, spent, took)) = r else {
      errors += 1;
      continue;
    };
    let d = crate::decide::Decision {
      profile: profile.name.into(),
      answers: answers.clone(),
      thresholds: profile
        .thresholds
        .iter()
        .map(|(k, t)| ((*k).to_owned(), *t))
        .collect(),
      calibrated: true,
      model: spent.model.clone(),
      ms: *took,
      cached: false,
      fell_back: None,
      spent: Some(spent.clone()),
    };
    let Some((choice, p)) = d.choice("verdict") else {
      errors += 1;
      continue;
    };
    let Some(new) = Verdict::parse(choice) else {
      errors += 1;
      continue;
    };
    n += 1;
    usd += spent.usd.unwrap_or(0.0);
    ms.push(*took);
    *confusion.entry((c.old.as_str(), new.as_str())).or_default() += 1;
    let same = c.old == new;
    exact += usize::from(same);
    waits += usize::from((c.old == Verdict::Waiting) == (new == Verdict::Waiting));
    open += usize::from(c.old.open() == new.open());
    for t in [50u32, 60, 70, 80, 90] {
      let e = by_t.entry(t).or_default();
      if p >= f64::from(t) / 100.0 {
        e.0 += 1;
        e.1 += usize::from(same);
      }
    }
    if d.passes("verdict") {
      profile_taken.0 += 1;
      profile_taken.1 += usize::from(same);
      profile_taken.2 += usize::from(c.old == Verdict::Waiting && new != Verdict::Waiting);
    }
    let line = line_of(new, &e.words);
    if !same && disagree.len() < 25 {
      disagree.push(format!(
        "  {} [{}] quick {} → decision {} {p:.2}\n    quick: {}\n    words: {}",
        c.thread,
        c.title.chars().take(50).collect::<String>(),
        c.old.as_str(),
        new.as_str(),
        c.old_line,
        line
      ));
    }
    rows.push(json!({
      "thread": c.thread.to_string(), "step": c.step.to_string(),
      "old": c.old.as_str(), "new": new.as_str(), "p": p,
      "probabilities": d.answers_json(4)["verdict"]["probabilities"],
      "old_line": c.old_line, "line": line, "ms": took,
    }));
  }
  ms.sort_unstable();
  let pct = |a: usize| 100.0 * a as f64 / n.max(1) as f64;
  println!("\nanswered {n}, errors {errors}");
  println!("exact agreement: {exact}/{n} ({:.1}%)", pct(exact));
  println!(
    "waits on the user (waiting-on-you vs the rest): {waits}/{n} ({:.1}%)",
    pct(waits)
  );
  println!(
    "open (waiting or continue) vs closed (done or stale): {open}/{n} ({:.1}%)",
    pct(open)
  );
  println!("\nby threshold on the choice's probability (taken / agreement among taken):");
  for (t, (k, a)) in &by_t {
    println!(
      "  ≥ 0.{t}: {k}/{n} taken ({:.1}%), {a}/{k} agree ({:.1}%)",
      pct(*k),
      100.0 * *a as f64 / (*k).max(1) as f64
    );
  }
  println!(
    "  the profile's ({:?}): {}/{n} taken ({:.1}%), {} agree ({:.1}%), {} of the quick model's \
     waits called otherwise",
    profile.thresholds,
    profile_taken.0,
    pct(profile_taken.0),
    profile_taken.1,
    100.0 * profile_taken.1 as f64 / profile_taken.0.max(1) as f64,
    profile_taken.2
  );
  println!("\nconfusion (quick → decision):");
  for ((o, d), k) in &confusion {
    println!("  {o:>15} → {d:<15} {k}");
  }
  println!(
    "\ncost ${usd:.4} for {n} ({:.6} a call); median {} ms, p90 {} ms",
    usd / n.max(1) as f64,
    ms.get(ms.len() / 2).unwrap_or(&0),
    ms.get(ms.len() * 9 / 10).unwrap_or(&0)
  );
  println!("\nsome disagreements:\n{}", disagree.join("\n"));
  let out =
    std::env::var("STITCH_EVAL_OUT").unwrap_or_else(|_| "/tmp/decide-eval/loose-ends.jsonl".into());
  if let Some(dir) = std::path::Path::new(&out).parent() {
    let _ = std::fs::create_dir_all(dir);
  }
  let text: String = rows.iter().map(|r| format!("{r}\n")).collect();
  std::fs::write(&out, text).unwrap();
  println!("rows in {out}");
}
