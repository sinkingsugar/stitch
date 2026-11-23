//! What app containers keep on disk (design §2.23): images in the user's
//! podman storage, an optional cache volume per app; only what Stitch
//! labelled with this node's key is ever removed, a base image only when
//! Stitch pulled it.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::build::{L_APP, L_NAME, L_NODE, L_PINS};
use super::engine::Podman;
use super::node_key;
use super::spec::ContainerNeeds;
use crate::apps::threads::Owner;
use crate::node::Shared;

/// What the node keeps of its builds: the base images it pulled.
#[derive(Default, Serialize, Deserialize)]
struct State {
  #[serde(default)]
  pulled: Vec<String>,
}

fn state_path(s: &Shared) -> std::path::PathBuf {
  super::dir(s).join("state.json")
}

fn read_state(s: &Shared) -> State {
  std::fs::read(state_path(s))
    .ok()
    .and_then(|b| serde_json::from_slice(&b).ok())
    .unwrap_or_default()
}

fn write_state(s: &Shared, st: &State) {
  let _ = std::fs::create_dir_all(super::dir(s));
  let _ = std::fs::write(
    state_path(s),
    serde_json::to_vec_pretty(st).unwrap_or_default(),
  );
}

/// Base images a build pulled (absent before it): Stitch's to remove.
pub(crate) fn record_pulled(s: &Shared, bases: &[String]) {
  if bases.is_empty() {
    return;
  }
  let mut st = read_state(s);
  for b in bases {
    if !st.pulled.contains(b) {
      st.pulled.push(b.clone());
    }
  }
  write_state(s, &st);
}

/// An image of Stitch's.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
  pub id: String,
  pub app: String,
  pub name: String,
  pub pins: String,
  pub created: i64,
  pub size: u64,
}

/// This node's images, every app's (or one's).
pub async fn images(podman: &Podman, node: &str, app: Option<&str>) -> Result<Vec<Image>, String> {
  let mut args = vec![
    "images".to_owned(),
    "--no-trunc".into(),
    "--filter".into(),
    format!("label={L_NODE}={node}"),
  ];
  if let Some(a) = app {
    args.push("--filter".into());
    args.push(format!("label={L_APP}={a}"));
  }
  args.push("--format".into());
  args.push("json".into());
  let refs: Vec<&str> = args.iter().map(String::as_str).collect();
  let (code, out, err) = podman.output(&refs, Duration::from_secs(60)).await?;
  if code != 0 {
    return Err(format!("podman images: {}", err.trim()));
  }
  let v: Value = serde_json::from_str(out.trim()).unwrap_or(Value::Array(Vec::new()));
  let mut list = Vec::new();
  for i in v.as_array().into_iter().flatten() {
    let label = |k: &str| {
      i.pointer(&format!("/Labels/{}", k.replace('/', "~1")))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
    };
    let id = i["Id"]
      .as_str()
      .unwrap_or("")
      .trim_start_matches("sha256:")
      .to_owned();
    if id.is_empty() || list.iter().any(|x: &Image| x.id == id) {
      continue;
    }
    list.push(Image {
      id,
      app: label(L_APP),
      name: label(L_NAME),
      pins: label(L_PINS),
      created: i["Created"].as_i64().unwrap_or(0),
      size: i["Size"].as_u64().unwrap_or(0),
    });
  }
  list.sort_by_key(|i| std::cmp::Reverse(i.created));
  Ok(list)
}

/// After a build: the app's images within its disk budget (the new one
/// removed when it passes it alone), the current and previous kept, older
/// ones removed.
pub(crate) async fn after_build(
  _s: &Arc<Shared>,
  podman: &Podman,
  _owner: &Owner,
  key: &str,
  id: &str,
  spec: &ContainerNeeds,
) -> Result<(), String> {
  let node = node_key(&_s.data);
  let list = images(podman, &node, Some(key)).await?;
  let budget = spec.build.disk_mb.unwrap_or(super::spec::DISK_MB) << 20;
  let new = list.iter().find(|i| i.id == id);
  if let Some(n) = new
    && n.size > budget
  {
    let _ = podman
      .output(&["rmi", "-f", id], Duration::from_secs(120))
      .await;
    return Err(format!(
      "the image is {} MiB, past the app's {} MiB (container.build.disk_mb): removed",
      n.size >> 20,
      budget >> 20
    ));
  }
  // The new one first, then the newest other: kept. The rest removed.
  let mut kept = vec![id.to_owned()];
  if let Some(prev) = list.iter().find(|i| i.id != id) {
    kept.push(prev.id.clone());
  }
  let mut total: u64 = list
    .iter()
    .filter(|i| kept.contains(&i.id))
    .map(|i| i.size)
    .sum();
  // Over the budget with both: only the new one.
  if total > budget && kept.len() > 1 {
    kept.truncate(1);
    total = new.map_or(0, |n| n.size);
  }
  let _ = total;
  for i in list.iter().filter(|i| !kept.contains(&i.id)) {
    let _ = podman
      .output(&["rmi", "-f", &i.id], Duration::from_secs(120))
      .await;
  }
  Ok(())
}

/// The app's cache volume (made, labelled, when it is not there): its name.
pub(crate) async fn cache_volume(s: &Shared, key: &str) -> Result<String, String> {
  let name = format!("stitch-app-{key}-cache");
  let podman = Podman::find().ok_or("no podman")?;
  if !matches!(
    podman
      .output(&["volume", "exists", &name], Duration::from_secs(20))
      .await,
    Ok((0, _, _))
  ) {
    let node = node_key(&s.data);
    let (code, _, err) = podman
      .output(
        &[
          "volume",
          "create",
          "--label",
          &format!("{L_NODE}={node}"),
          "--label",
          &format!("{L_APP}={key}"),
          &name,
        ],
        Duration::from_secs(30),
      )
      .await?;
    if code != 0 {
      return Err(format!("the cache volume: {}", err.trim()));
    }
    // Its files are the container user's.
    let mount = volume_dir(&podman, &name).await?;
    let _ = podman
      .output(
        &["unshare", "chown", "1000:1000", &mount],
        Duration::from_secs(30),
      )
      .await;
  }
  Ok(name)
}

async fn volume_dir(podman: &Podman, name: &str) -> Result<String, String> {
  let (code, out, err) = podman
    .output(
      &["volume", "inspect", "--format", "{{.Mountpoint}}", name],
      Duration::from_secs(30),
    )
    .await?;
  if code != 0 {
    return Err(err.trim().to_owned());
  }
  Ok(out.trim().to_owned())
}

/// A volume's size as the container's user sees it.
async fn volume_size(podman: &Podman, name: &str) -> Option<u64> {
  let dir = volume_dir(podman, name).await.ok()?;
  let (_, out, _) = podman
    .output(
      &["unshare", "du", "-s", "-B1", "--", &dir],
      Duration::from_secs(60),
    )
    .await
    .ok()?;
  out.split_whitespace().next()?.parse().ok()
}

/// The cache emptied when past its size.
pub(crate) async fn bound_cache(podman: &Podman, name: &str, mb: u64) {
  if let Some(size) = volume_size(podman, name).await
    && size > mb << 20
    && let Ok(dir) = volume_dir(podman, name).await
  {
    let _ = podman
      .output(
        &["unshare", "find", &dir, "-mindepth", "1", "-delete"],
        Duration::from_secs(120),
      )
      .await;
  }
}

/// At the node's start: containers of this node's left by a crash (conmon
/// keeps them alive), removed; never another's.
#[cfg(target_os = "linux")]
pub(crate) async fn clean_at_start(s: &Shared) -> usize {
  // Only a node that ever ran an app's container asks podman (a podman
  // call at every start, on every node, raced the start's own sockets
  // in the folders test).
  if !super::dir(s).exists() {
    return 0;
  }
  let Some(podman) = Podman::find() else {
    return 0;
  };
  let node = node_key(&s.data);
  let Ok((_, out, _)) = podman
    .output(
      &[
        "ps",
        "-a",
        "-q",
        "--filter",
        &format!("label={L_NODE}={node}"),
      ],
      Duration::from_secs(30),
    )
    .await
  else {
    return 0;
  };
  let ids: Vec<&str> = out.split_whitespace().collect();
  for id in &ids {
    let _ = podman
      .output(&["rm", "-f", "-t", "0", id], Duration::from_secs(60))
      .await;
  }
  let _ = std::fs::remove_dir_all(super::build::scratch(s));
  ids.len()
}

/// `stitch app storage`: each app's images, cache and data folder, or one
/// app's removed (its images and cache; its data folder stays, the
/// user's).
pub(crate) async fn report(s: &Arc<Shared>, remove: Option<&str>) -> Result<Vec<String>, String> {
  let podman = Podman::find().ok_or("no podman on this node: no app containers here")?;
  let node = node_key(&s.data);
  let list = images(&podman, &node, None).await?;
  let mut by: BTreeMap<String, Vec<Image>> = BTreeMap::new();
  for i in list {
    by.entry(i.app.clone()).or_default().push(i);
  }
  let mut out = Vec::new();
  if let Some(name) = remove {
    let keys: Vec<String> = by
      .iter()
      .filter(|(k, v)| *k == name || v.iter().any(|i| i.name == name))
      .map(|(k, _)| k.clone())
      .collect();
    if keys.is_empty() {
      return Err(format!("no images of an app \"{name}\" on this node"));
    }
    for k in keys {
      let n = super::run::stop_app(s, &k).await;
      let mut freed = 0;
      for i in &by[&k] {
        if matches!(
          podman
            .output(&["rmi", "-f", &i.id], Duration::from_secs(120))
            .await,
          Ok((0, _, _))
        ) {
          freed += i.size;
        }
      }
      let vol = format!("stitch-app-{k}-cache");
      let _ = podman
        .output(&["volume", "rm", "-f", &vol], Duration::from_secs(60))
        .await;
      out.push(format!(
        "app \"{}\": {} image(s) removed ({} MiB){}, cache removed; rebuilt at its next run \
         (within its build grant)",
        by[&k].first().map(|i| i.name.as_str()).unwrap_or(&k),
        by[&k].len(),
        freed >> 20,
        if n > 0 {
          format!(", {n} running stopped")
        } else {
          String::new()
        }
      ));
    }
    // Bases Stitch pulled, when no image of its own uses them any more.
    let st = read_state(s);
    let left = images(&podman, &node, None).await.unwrap_or_default();
    let mut keep = Vec::new();
    for b in st.pulled {
      if left.is_empty()
        && matches!(
          podman.output(&["rmi", &b], Duration::from_secs(60)).await,
          Ok((0, _, _))
        )
      {
        out.push(format!("base image {b} removed (Stitch pulled it)"));
      } else {
        keep.push(b);
      }
    }
    write_state(s, &ReadState { pulled: keep }.into());
    return Ok(out);
  }
  if by.is_empty() {
    out.push("No app images on this node.".into());
  }
  let mut total = 0u64;
  for (k, imgs) in &by {
    let size: u64 = imgs.iter().map(|i| i.size).sum();
    total += size;
    let cache = volume_size(&podman, &format!("stitch-app-{k}-cache")).await;
    out.push(format!(
      "{} ({k}): {} image(s), {} MiB{}",
      imgs.first().map(|i| i.name.as_str()).unwrap_or("?"),
      imgs.len(),
      size >> 20,
      cache
        .map(|c| format!("; cache {} MiB", c >> 20))
        .unwrap_or_default()
    ));
    for i in imgs {
      out.push(format!(
        "  image {} pins {} {} MiB",
        &i.id[..12.min(i.id.len())],
        i.pins,
        i.size >> 20
      ));
    }
  }
  let st = read_state(s);
  if !st.pulled.is_empty() {
    out.push(format!(
      "Base images Stitch pulled: {}",
      st.pulled.join(", ")
    ));
  }
  out.push(format!(
    "Images share layers, so sizes overlap; together at most {} MiB. Podman's storage: {}",
    total >> 20,
    super::engine::engine()
      .await
      .map(|(_, i)| i.graph_root.display().to_string())
      .unwrap_or_else(|e| e)
  ));
  Ok(out)
}

struct ReadState {
  pulled: Vec<String>,
}

impl From<ReadState> for State {
  fn from(r: ReadState) -> State {
    State { pulled: r.pulled }
  }
}
