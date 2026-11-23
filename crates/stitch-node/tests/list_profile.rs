//! What a client's refresh round reads (bytes this process read, per
//! request), on a real node's data: `STITCH_PROFILE_DATA` names a copy of
//! an index with its record roots, never the live one (the node started
//! here writes to them). To make one from `~/.stitch` into `$P`: copy
//! `home`, `projects` and `folders` under `$P/roots/`, `sqlite3
//! ~/.stitch/index.sqlite ".backup '$P/index.sqlite'"`, then `UPDATE roots
//! SET dir = replace(dir, '<~/.stitch>/', '$P/roots/')` in the copy.
//! Ignored unless asked for: `STITCH_PROFILE_DATA=$P cargo test --release
//! -p stitch-node --test list_profile -- --ignored --nocapture`.

// `/proc/self/io`: Linux alone (built for iOS too, by `just check-apple`).
#![cfg(target_os = "linux")]

use std::time::Instant;

use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{Config, ProviderConfig};

fn rchar() -> u64 {
  std::fs::read_to_string("/proc/self/io")
    .unwrap()
    .lines()
    .find_map(|l| l.strip_prefix("rchar: "))
    .unwrap()
    .parse()
    .unwrap()
}

fn threads(limit: Option<u32>, forest: bool, place: Option<&str>) -> Request {
  Request::Threads {
    status: None,
    since: None,
    limit,
    thread: None,
    place: place.map(str::to_owned),
    forest: forest.then_some(true),
  }
}

fn live(what: &str) -> Request {
  Request::Live {
    what: what.into(),
    query: None,
  }
}

async fn measure(c: &mut Client, name: &str, req: &Request, n: usize) {
  // Once to warm what is cached per step (transcripts), then measured.
  let _ = c.request(req).await;
  let (r0, t0) = (rchar(), Instant::now());
  let mut bytes = 0;
  for _ in 0..n {
    if let Ok(Reply::Lines(l)) = c.request(req).await {
      bytes = l.iter().map(String::len).sum::<usize>();
    }
  }
  let mb = (rchar() - r0) as f64 / n as f64 / 1e6;
  let ms = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
  println!("{name:28} {mb:9.2} MB read {ms:8.1} ms  reply {bytes} B");
}

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_refresh_reads() {
  let Ok(src) = std::env::var("STITCH_PROFILE_DATA") else {
    return;
  };
  let src = std::path::PathBuf::from(src);
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  std::fs::create_dir_all(&data).unwrap();
  std::fs::copy(src.join("index.sqlite"), data.join("index.sqlite")).unwrap();
  let mut c = Config {
    node: Some("gamer".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    ..Config::default()
  };
  c.providers.insert(
    "anthropic".into(),
    ProviderConfig {
      kind: "anthropic".into(),
      base_url: Some("http://127.0.0.1:9".into()),
      ..ProviderConfig::default()
    },
  );
  c.save(&data).unwrap();
  let _node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  for limit in [1, 300] {
    measure(
      &mut c,
      &format!("Threads limit={limit}"),
      &threads(Some(limit), false, None),
      2,
    )
    .await;
  }
  measure(
    &mut c,
    "Threads forest",
    &threads(Some(5000), true, None),
    1,
  )
  .await;
  measure(
    &mut c,
    "Threads place=stitch",
    &threads(Some(500), false, Some("stitch")),
    1,
  )
  .await;
  for w in ["attention", "jobs", "schedules", "loose-ends", "places"] {
    measure(&mut c, &format!("Live {w}"), &live(w), 2).await;
  }
}
