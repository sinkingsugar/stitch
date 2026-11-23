//! Building an app's image (design §2.23): only under the build grant,
//! from the granted context's blobs (never the live directory), one build
//! at a time on the node, at most once an hour per app and pins unless
//! the user asked, with the build's network kept off the host's own
//! services, labelled so only Stitch's are ever removed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::engine::{self, Podman};
use super::spec::{ContainerNeeds, Pins};
use super::{Kind, app_key, node_key};
use crate::apps::threads::Owner;
use crate::node::Shared;

/// The labels every image and container of Stitch's carries.
pub const L_NODE: &str = "stitch.node";
pub const L_APP: &str = "stitch.app";
pub const L_NAME: &str = "stitch.app_name";
pub const L_PINS: &str = "stitch.pins";
pub const L_RUN: &str = "stitch.run";

/// The build's network: pasta, the gateway not mapped to the host's
/// loopback, and no address mapped to the host's own (podman maps
/// 169.254.1.2, `host.containers.internal`, to it by default, which
/// reaches every service listening on all the host's addresses: found by
/// the live tests on gamer).
pub const BUILD_NETWORK: &str = "pasta:--no-map-gw,--map-guest-addr,none";

/// An app's image's name.
pub fn image_name(key: &str, pins: &Pins) -> String {
  format!("localhost/stitch-app-{key}:{}", pins.id())
}

/// One build at a time on the node.
fn building() -> &'static tokio::sync::Semaphore {
  static B: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
  B.get_or_init(|| tokio::sync::Semaphore::new(1))
}

/// When each app and pins (by key) was last built or tried, and how it failed.
type Tried = Mutex<HashMap<(String, String), (Instant, Option<String>)>>;

/// When each app and pins was last built (or tried), on this node.
fn tried() -> &'static Tried {
  static T: OnceLock<Tried> = OnceLock::new();
  T.get_or_init(Default::default)
}

const RETRY: Duration = Duration::from_secs(3600);

/// The image built here for an app's pins, by ID, when there is one.
pub async fn image_for(podman: &Podman, node: &str, key: &str, pins: &Pins) -> Option<String> {
  let (code, out, _) = podman
    .output(
      &[
        "images",
        "--noheading",
        "--no-trunc",
        "--filter",
        &format!("label={L_NODE}={node}"),
        "--filter",
        &format!("label={L_APP}={key}"),
        "--filter",
        &format!("label={L_PINS}={}", pins.id()),
        "--format",
        "{{.ID}}",
      ],
      Duration::from_secs(30),
    )
    .await
    .ok()?;
  if code != 0 {
    return None;
  }
  out
    .lines()
    .map(str::trim)
    .find(|l| !l.is_empty())
    .map(|l| l.trim_start_matches("sha256:").to_owned())
}

/// The image for the app's granted build: built if it is not here yet
/// (`by_user`: the user's grant started it, so no hourly limit). Its ID.
pub(crate) async fn ensure(
  s: &Arc<Shared>,
  owner: &Owner,
  by_user: bool,
) -> Result<String, String> {
  let g = super::granted(s, owner, Kind::Build).ok_or_else(|| {
    format!(
      "app \"{}\" has no build grant: the user grants it with `stitch app allow-container`",
      owner.name
    )
  })?;
  let pins = g.spec.pins.clone().ok_or("the grant has no pins")?;
  let (podman, _) = engine::engine().await?;
  let node = node_key(&s.data);
  let key = app_key(owner);
  if let Some(id) = image_for(&podman, &node, &key, &pins).await {
    return Ok(id);
  }
  let k = (key.clone(), pins.id());
  if !by_user
    && let Some((at, err)) = tried().lock().expect("tried lock").get(&k)
    && at.elapsed() < RETRY
  {
    return Err(format!(
      "app \"{}\"'s image was built or tried {} minutes ago{}; it is tried again at most once \
       an hour (or when the user grants the build again)",
      owner.name,
      at.elapsed().as_secs() / 60,
      err
        .as_ref()
        .map(|e| format!(" and failed: {e}"))
        .unwrap_or_default()
    ));
  }
  tried()
    .lock()
    .expect("tried lock")
    .insert(k.clone(), (Instant::now(), None));
  let _slot = building().acquire().await.map_err(|e| e.to_string())?;
  // Built meanwhile by another call.
  if let Some(id) = image_for(&podman, &node, &key, &pins).await {
    return Ok(id);
  }
  let r = build(s, &podman, owner, &node, &key, &g.spec, &pins).await;
  if let Err(e) = &r {
    tried()
      .lock()
      .expect("tried lock")
      .insert(k, (Instant::now(), Some(e.chars().take(300).collect())));
  }
  r
}

/// Writes the granted context from its blobs into `dir` (fresh), each
/// file by its plain path.
fn write_context(s: &Shared, pins: &Pins, dir: &Path) -> Result<(), String> {
  if dir.exists() {
    std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
  }
  std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
  for (rel, blob) in &pins.files {
    if !super::spec::plain_rel(rel) {
      return Err(format!("{rel}: not a plain path"));
    }
    let h: stitch_records::BlobHash = blob.parse().map_err(|_| format!("{rel}: no blob"))?;
    let src = s.state().blobs.path(&h);
    let dst = dir.join(rel);
    if let Some(p) = dst.parent() {
      std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    std::fs::copy(&src, &dst).map_err(|e| format!("{rel} (blob {h}): {e}"))?;
  }
  let cf = std::fs::read(dir.join("Containerfile")).map_err(|e| format!("Containerfile: {e}"))?;
  if super::spec::sha256(&cf) != pins.containerfile {
    return Err("the Containerfile is not the one granted".into());
  }
  Ok(())
}

/// The base images not in podman's storage yet: the ones Stitch pulls, and
/// so may remove.
async fn absent_bases(podman: &Podman, pins: &Pins) -> Vec<String> {
  let mut out = Vec::new();
  for b in &pins.bases {
    let present = matches!(
      podman
        .output(&["image", "exists", b], Duration::from_secs(20))
        .await,
      Ok((0, _, _))
    );
    if !present {
      out.push(b.clone());
    }
  }
  out
}

async fn build(
  s: &Arc<Shared>,
  podman: &Podman,
  owner: &Owner,
  node: &str,
  key: &str,
  spec: &ContainerNeeds,
  pins: &Pins,
) -> Result<String, String> {
  let root = super::dir(s).join("build");
  let ctx = root.join(format!("{key}-{}", pins.id()));
  write_context(s, pins, &ctx)?;
  let pulled = absent_bases(podman, pins).await;
  let mem = format!(
    "{}m",
    spec.build.memory_mb.unwrap_or(super::spec::BUILD_MEMORY_MB)
  );
  let network = if spec.build.network == Some(false) {
    "none".to_owned()
  } else {
    // The gateway not mapped to the host: the host's loopback stays out of
    // reach (verified on gamer, design §2.23).
    BUILD_NETWORK.to_owned()
  };
  let tag = image_name(key, pins);
  let labels = [
    format!("{L_NODE}={node}"),
    format!("{L_APP}={key}"),
    format!("{L_NAME}={}", owner.name.replace(['\n', '\r'], " ")),
    format!("{L_PINS}={}", pins.id()),
  ];
  let args = build_args(&ctx, &tag, &labels, &mem, &network);
  let minutes = spec.build.minutes.unwrap_or(super::spec::BUILD_MINUTES);
  let log_path = root.join(format!("{key}.log"));
  let mut cmd = podman.command();
  cmd.args(&args);
  cmd.stdout(std::process::Stdio::piped());
  cmd.stderr(std::process::Stdio::piped());
  cmd.process_group(0);
  let started = Instant::now();
  let mut child = cmd.spawn().map_err(|e| format!("podman build: {e}"))?;
  let pid = child.id();
  let (mut so, mut se) = (child.stdout.take(), child.stderr.take());
  let read = async move {
    use tokio::io::AsyncReadExt;
    let mut log = Vec::new();
    let (mut a, mut b) = (vec![0u8; 8192], vec![0u8; 8192]);
    let (mut o_done, mut e_done) = (so.is_none(), se.is_none());
    while !(o_done && e_done) {
      tokio::select! {
        n = async { so.as_mut().unwrap().read(&mut a).await }, if !o_done => match n {
          Ok(0) | Err(_) => o_done = true,
          Ok(n) => log.extend_from_slice(&a[..n]),
        },
        n = async { se.as_mut().unwrap().read(&mut b).await }, if !e_done => match n {
          Ok(0) | Err(_) => e_done = true,
          Ok(n) => log.extend_from_slice(&b[..n]),
        },
      }
      // The last 256 KiB.
      if log.len() > 512 * 1024 {
        log.drain(..log.len() - 256 * 1024);
      }
    }
    log
  };
  let limit = Duration::from_secs(minutes * 60);
  let (status, log) = match tokio::time::timeout(limit, async {
    let log = read.await;
    (child.wait().await, log)
  })
  .await
  {
    Ok((st, log)) => (st.map_err(|e| e.to_string()), log),
    Err(_) => {
      // Asked to stop first, so podman removes its build containers.
      if let Some(p) = pid {
        crate::mcp::kill_group(p, libc::SIGTERM);
      }
      tokio::time::sleep(Duration::from_secs(10)).await;
      if let Some(p) = pid {
        crate::mcp::kill_group(p, libc::SIGKILL);
      }
      let _ = std::fs::remove_dir_all(&ctx);
      return Err(format!(
        "the build took longer than its {minutes} minutes: stopped"
      ));
    }
  };
  let _ = std::fs::write(&log_path, &log);
  let _ = std::fs::remove_dir_all(&ctx);
  let tail = |log: &[u8]| {
    let t = String::from_utf8_lossy(&log[log.len().saturating_sub(2000)..]).into_owned();
    crate::apps::containers::run::clean_text(&t)
  };
  match status {
    Ok(st) if st.success() => {}
    Ok(st) => {
      return Err(format!(
        "the build failed ({st}); its log is {}; the end:\n{}",
        log_path.display(),
        tail(&log)
      ));
    }
    Err(e) => return Err(format!("podman build: {e}")),
  }
  let id = image_for(podman, node, key, pins)
    .await
    .ok_or("the build ended but its image is not here")?;
  super::storage::record_pulled(s, &pulled);
  super::storage::after_build(s, podman, owner, key, &id, spec).await?;
  eprintln!(
    "app \"{}\": image {} built in {}s",
    owner.name,
    &id[..12.min(id.len())],
    started.elapsed().as_secs()
  );
  Ok(id)
}

/// `podman build`'s argv: the context and nothing else, no secrets or
/// SSH agent, no intermediate images kept, the memory bounded, the
/// network as said.
pub(crate) fn build_args(
  ctx: &Path,
  tag: &str,
  labels: &[String],
  mem: &str,
  network: &str,
) -> Vec<String> {
  let mut args: Vec<String> = vec![
    "build".into(),
    "--file".into(),
    ctx.join("Containerfile").display().to_string(),
    "--tag".into(),
    tag.to_owned(),
    "--layers=false".into(),
    "--force-rm".into(),
    "--pull=missing".into(),
    "--memory".into(),
    mem.to_owned(),
    "--memory-swap".into(),
    mem.to_owned(),
    "--network".into(),
    network.to_owned(),
    "--no-hosts".into(),
    "--jobs".into(),
    "1".into(),
  ];
  for l in labels {
    args.push("--label".into());
    args.push(l.clone());
  }
  args.push(ctx.display().to_string());
  args
}

/// Where a run's or build's files are kept a moment (the node's).
pub(crate) fn scratch(s: &Shared) -> PathBuf {
  super::dir(s).join("runs")
}
