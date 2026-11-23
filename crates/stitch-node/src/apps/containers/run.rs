//! Running a declared command (design §2.23): the whole invocation is
//! Stitch's, the inputs copied in read-only, the outputs read back as
//! hostile once the container is gone, then written into the app's data
//! folder, images also attached to the reply.

use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::build::{L_APP, L_NAME, L_NODE, L_RUN};
use super::engine::{self, Podman};
use super::spec::{self, Arg, Command};
use super::{Kind, app_key, node_key};
use crate::apps::threads::Owner;
use crate::node::Shared;

/// A file argument's size, at most.
const MAX_INPUT: u64 = 256 * 1024 * 1024;
/// stdout and stderr kept, each.
const KEEP_OUTPUT: usize = 16 * 1024;
/// Images attached to the reply: at most this many, each this big.
const MAX_IMAGES: usize = 8;
const MAX_IMAGE: u64 = 10 * 1024 * 1024;
/// The container's user: a subordinate ID on the host, owning nothing of
/// the user's.
pub const USER: &str = "1000:1000";

/// One GPU command at a time on the node.
fn gpu() -> &'static tokio::sync::Semaphore {
  static G: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
  G.get_or_init(|| tokio::sync::Semaphore::new(1))
}

/// A file a run made, accepted.
#[derive(Clone, Debug, PartialEq)]
pub struct Made {
  /// Its path in the data folder.
  pub rel: String,
  pub bytes: u64,
  /// An image's type, when it is one that decodes.
  pub image: Option<&'static str>,
}

/// What a run gave.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
  pub exit: Option<i32>,
  pub seconds: u64,
  pub stdout: String,
  pub stderr: String,
  /// Why it was stopped, if it was (its time, its outputs' size).
  pub stopped: Option<String>,
  /// Its folder in the data folder.
  pub folder: Option<String>,
  pub made: Vec<Made>,
  /// What was left out, and why.
  pub skipped: Vec<String>,
}

/// Text from a container as the model and terminals may see it: control
/// characters (escapes) out, but for newlines and tabs.
pub fn clean_text(t: &str) -> String {
  t.chars()
    .map(|c| {
      if c.is_control() && c != '\n' && c != '\t' {
        '·'
      } else {
        c
      }
    })
    .collect()
}

fn cut(b: &[u8]) -> String {
  let t = String::from_utf8_lossy(b);
  let t = if t.len() > KEEP_OUTPUT {
    let mut i = t.len() - KEEP_OUTPUT;
    while !t.is_char_boundary(i) {
      i += 1;
    }
    format!("[… cut]\n{}", &t[i..])
  } else {
    t.into_owned()
  };
  clean_text(&t)
}

/// A file of the data folder by its relative path: every part a plain
/// name, none a link, the file regular. Its path.
pub fn file_in(root: &Path, rel: &str) -> Result<PathBuf, String> {
  if !spec::plain_rel(rel) {
    return Err(format!(
      "{rel}: a plain path in the app's data folder (no `..`, no hidden parts)"
    ));
  }
  let mut p = root.to_owned();
  let parts: Vec<&str> = rel.split('/').collect();
  for (i, c) in parts.iter().enumerate() {
    p.push(c);
    let m = std::fs::symlink_metadata(&p).map_err(|_| format!("{rel}: not there"))?;
    if m.file_type().is_symlink() {
      return Err(format!("{rel}: {c} is a link; links are not followed"));
    }
    let last = i + 1 == parts.len();
    if last && !m.file_type().is_file() {
      return Err(format!("{rel}: not a regular file"));
    }
    if !last && !m.is_dir() {
      return Err(format!("{rel}: {c} is not a folder"));
    }
  }
  Ok(p)
}

/// Copies `src` (opened without following a link when `nofollow`) into
/// `dst`, at most `max` bytes: its SHA-256.
fn copy_in(src: &Path, dst: &Path, max: u64, nofollow: bool) -> Result<String, String> {
  let mut o = std::fs::OpenOptions::new();
  o.read(true);
  o.custom_flags(libc::O_NONBLOCK | if nofollow { libc::O_NOFOLLOW } else { 0 });
  let mut f = o.open(src).map_err(|e| format!("{}: {e}", src.display()))?;
  let m = f.metadata().map_err(|e| e.to_string())?;
  if !m.is_file() {
    return Err(format!("{}: not a regular file", src.display()));
  }
  if m.len() > max {
    return Err(format!("{}: over {} MiB", src.display(), max >> 20));
  }
  let mut bytes = Vec::with_capacity(m.len() as usize);
  (&mut f)
    .take(max + 1)
    .read_to_end(&mut bytes)
    .map_err(|e| e.to_string())?;
  if bytes.len() as u64 > max {
    return Err(format!("{}: over {} MiB", src.display(), max >> 20));
  }
  if let Some(p) = dst.parent() {
    std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
  }
  std::fs::write(dst, &bytes).map_err(|e| format!("{}: {e}", dst.display()))?;
  std::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o444))
    .map_err(|e| e.to_string())?;
  Ok(spec::sha256(&bytes))
}

/// The argv of a command with its arguments, each placeholder one whole
/// argument; the files to copy in (data folder path → path under `/in`).
/// A command's argv and its files to copy in.
pub type Argv = (Vec<String>, Vec<(String, String)>);

pub fn argv(cmd: &Command, given: &serde_json::Map<String, Value>) -> Result<Argv, String> {
  for k in given.keys() {
    if !cmd.args.contains_key(k) {
      return Err(format!("no argument `{k}`; it takes {}", names(cmd)));
    }
  }
  let mut copies = Vec::new();
  let mut out = Vec::new();
  for a in &cmd.run {
    let Some(n) = spec::placeholder(a) else {
      out.push(a.clone());
      continue;
    };
    let kind = &cmd.args[n];
    let v = given.get(n);
    let s = match kind {
      Arg::Out { fresh } => {
        if v.is_some() {
          return Err(format!(
            "`{n}` is the output folder, made by the node: give nothing"
          ));
        }
        if *fresh {
          "/out/result".to_owned()
        } else {
          "/out".to_owned()
        }
      }
      Arg::File { dir, ext } => {
        let rel = v
          .and_then(Value::as_str)
          .ok_or_else(|| format!("`{n}`: a file of the app's data folder"))?;
        if !spec::plain_rel(rel) {
          return Err(format!("`{n}`: {rel}: a plain path in the data folder"));
        }
        if !dir.is_empty() && !rel.starts_with(&format!("{dir}/")) {
          return Err(format!("`{n}`: {rel} is not under {dir}/"));
        }
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let e = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        if !e.as_ref().is_some_and(|e| ext.contains(e)) {
          return Err(format!("`{n}`: {rel} is not a .{} file", ext.join("/.")));
        }
        let inside = format!("/in/args/{n}/{name}");
        copies.push((rel.to_owned(), inside.clone()));
        inside
      }
      Arg::Enum { values } => {
        let s = v.and_then(Value::as_str).unwrap_or("");
        if !values.iter().any(|x| x == s) {
          return Err(format!("`{n}`: one of {}", values.join(", ")));
        }
        s.to_owned()
      }
      Arg::Int { min, max } => {
        let i = v
          .and_then(Value::as_i64)
          .ok_or_else(|| format!("`{n}`: a whole number"))?;
        if i < *min || i > *max {
          return Err(format!("`{n}`: from {min} to {max}"));
        }
        i.to_string()
      }
      Arg::Slug => {
        let s = v.and_then(Value::as_str).unwrap_or("");
        if s.is_empty()
          || s.len() > 64
          || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
          || s.starts_with('-')
        {
          return Err(format!(
            "`{n}`: letters, digits, - and _ (not first), at most 64"
          ));
        }
        s.to_owned()
      }
    };
    out.push(s);
  }
  Ok((out, copies))
}

fn names(cmd: &Command) -> String {
  if cmd.args.is_empty() {
    "none".into()
  } else {
    cmd.args.keys().cloned().collect::<Vec<_>>().join(", ")
  }
}

/// Removes a run's container if it is still there when dropped (a
/// cancelled turn drops the future mid-run).
struct Reaper {
  podman: Podman,
  name: String,
  armed: bool,
}

impl Drop for Reaper {
  fn drop(&mut self) {
    if self.armed {
      let (p, n) = (self.podman.program.clone(), self.name.clone());
      std::thread::spawn(move || {
        let _ = std::process::Command::new(p)
          .args(["rm", "-f", "-t", "0", &n])
          .stdin(std::process::Stdio::null())
          .stdout(std::process::Stdio::null())
          .stderr(std::process::Stdio::null())
          .status();
      });
    }
  }
}

/// Removes the container `name` and waits until no container of the run
/// is left.
pub(crate) async fn remove(podman: &Podman, name: &str, run: &str) -> Result<(), String> {
  let _ = podman
    .output(&["kill", "--signal", "KILL", name], Duration::from_secs(30))
    .await;
  let _ = podman
    .output(&["rm", "-f", "-t", "0", name], Duration::from_secs(60))
    .await;
  for _ in 0..50 {
    let (_, out, _) = podman
      .output(
        &[
          "ps",
          "-a",
          "-q",
          "--filter",
          &format!("label={L_RUN}={run}"),
        ],
        Duration::from_secs(30),
      )
      .await?;
    if out.trim().is_empty() {
      return Ok(());
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
  }
  Err(format!("the container {name} is still there"))
}

/// Stops every container of an app on this node: how many.
pub(crate) async fn stop_app(s: &Shared, key: &str) -> usize {
  let Some(podman) = Podman::find() else {
    return 0;
  };
  let node = node_key(&s.data);
  let Ok((_, out, _)) = podman
    .output(
      &[
        "ps",
        "-q",
        "--filter",
        &format!("label={L_NODE}={node}"),
        "--filter",
        &format!("label={L_APP}={key}"),
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
  ids.len()
}

/// The outputs' size and count as the container's user sees them (its
/// folders may be closed to the node while it runs).
async fn measure(podman: &Podman, out: &Path) -> Option<(u64, u64)> {
  let dir = out.display().to_string();
  let (code, text, _) = podman
    .output(
      &[
        "unshare",
        "sh",
        "-c",
        "du -s -B1 -- \"$1\"; du -s --inodes -- \"$1\"",
        "sh",
        &dir,
      ],
      Duration::from_secs(20),
    )
    .await
    .ok()?;
  if code != 0 && text.is_empty() {
    return None;
  }
  let mut nums = text
    .lines()
    .filter_map(|l| l.split_whitespace().next()?.parse::<u64>().ok());
  Some((nums.next()?, nums.next()?))
}

/// Everything about a run that does not depend on the node: the podman
/// argv for `image`, the command's `argv`, the run's folders.
#[allow(clippy::too_many_arguments)]
pub fn podman_args(
  cmd: &Command,
  image: &str,
  argv: &[String],
  name: &str,
  labels: &[String],
  input: &Path,
  output: &Path,
  seccomp: &Path,
  cache: Option<(&str, &str)>,
  gpu: Option<&Path>,
) -> Result<Vec<String>, String> {
  for p in [input, output, seccomp] {
    let t = p.display().to_string();
    if t.contains([':', ',', '\n']) {
      return Err(format!("{t}: a path podman cannot mount (`:` or `,`)"));
    }
  }
  let mem = format!("{}m", cmd.memory_mb.unwrap_or(spec::MEMORY_MB));
  let timeout = cmd.timeout.unwrap_or(spec::TIMEOUT);
  let mut a: Vec<String> = [
    "run",
    "--rm",
    "--init",
    "--pull=never",
    "--network=none",
    "--read-only",
    "--read-only-tmpfs=false",
    "--tmpfs=/tmp:rw,nosuid,nodev,size=512m,mode=1777",
    "--tmpfs=/run:rw,nosuid,nodev,noexec,size=16m,mode=0755",
    "--tmpfs=/home/app:rw,nosuid,nodev,size=512m,mode=1777",
    "--tmpfs=/dev/shm:rw,nosuid,nodev,noexec,size=256m,mode=1777",
    "--cap-drop=all",
    "--security-opt=no-new-privileges",
    "--pid=private",
    "--ipc=private",
    "--uts=private",
    "--cgroupns=private",
    "--log-driver=none",
    "--ulimit=core=0:0",
    "--no-hosts",
    "--hostname=app",
    "--env=HOME=/home/app",
    "--env=TMPDIR=/tmp",
    "--env=LANG=C.UTF-8",
    "--env=PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    "--workdir=/home/app",
  ]
  .iter()
  .map(|s| s.to_string())
  .collect();
  a.push(format!("--security-opt=seccomp={}", seccomp.display()));
  a.push(format!("--user={USER}"));
  a.push(format!("--memory={mem}"));
  a.push(format!("--memory-swap={mem}"));
  a.push(format!("--pids-limit={}", cmd.pids.unwrap_or(spec::PIDS)));
  a.push(format!("--cpus={}", cmd.cpus.unwrap_or(spec::CPUS)));
  a.push(format!("--timeout={}", timeout + 5));
  a.push(format!("--name={name}"));
  for l in labels {
    a.push(format!("--label={l}"));
  }
  a.push(format!(
    "--volume={}:/in:ro,noexec,nosuid,nodev",
    input.display()
  ));
  a.push(format!(
    "--volume={}:/out:rw,noexec,nosuid,nodev",
    output.display()
  ));
  if let Some((vol, path)) = cache {
    a.push(format!("--volume={vol}:{path}:rw,noexec,nosuid,nodev"));
  }
  if let Some(dir) = gpu {
    a.push(format!("--cdi-spec-dir={}", dir.display()));
    a.push("--device=nvidia.com/gpu=all".into());
  }
  a.push(format!(
    "--entrypoint={}",
    serde_json::to_string(&[&argv[0]]).expect("json")
  ));
  a.push(image.to_owned());
  a.extend(argv[1..].iter().cloned());
  Ok(a)
}

/// The image built for an app's granted run, its build grant standing
/// with the same pins.
async fn image(s: &Arc<Shared>, owner: &Owner, pins: &spec::Pins) -> Result<String, String> {
  let b = super::granted(s, owner, Kind::Build).ok_or_else(|| {
    format!(
      "app \"{}\" has no build grant, so no image to run: the user grants it with `stitch app \
       allow-container`",
      owner.name
    )
  })?;
  if b.spec.pins.as_ref() != Some(pins) {
    return Err(format!(
      "app \"{}\"'s build grant is for other pins than its run grant: the user grants both again",
      owner.name
    ));
  }
  super::build::ensure(s, owner, false).await
}

/// The CDI spec of this node's GPU commands: generated with the system's
/// NVIDIA toolkit (root's, at least the version with the fixes), its hooks
/// only those Stitch keeps. Its directory.
pub(crate) async fn cdi(dir: PathBuf) -> Result<PathBuf, String> {
  let ctk = engine::root_owned(Path::new("/usr/bin/nvidia-ctk")).map_err(|e| {
    format!("GPU commands need the NVIDIA Container Toolkit installed by the system: {e}")
  })?;
  let out = tokio::process::Command::new(&ctk)
    .arg("--version")
    .env_clear()
    .output()
    .await
    .map_err(|e| format!("nvidia-ctk: {e}"))?;
  let v = engine::nvidia_ctk_version(&String::from_utf8_lossy(&out.stdout))
    .ok_or("nvidia-ctk: its version does not read")?;
  if v < spec::MIN_NVIDIA_CTK {
    return Err(format!(
      "the NVIDIA Container Toolkit here is {}.{}.{}: GPU commands need {}.{}.{} or later (the \
       fixes for CVE-2024-0132 and CVE-2025-23266)",
      v.0,
      v.1,
      v.2,
      spec::MIN_NVIDIA_CTK.0,
      spec::MIN_NVIDIA_CTK.1,
      spec::MIN_NVIDIA_CTK.2
    ));
  }
  std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
  let path = dir.join("nvidia.json");
  let mut c = tokio::process::Command::new(&ctk);
  c.env_clear().env("PATH", "/usr/bin:/bin");
  c.args([
    "cdi",
    "generate",
    "--quiet",
    "--format",
    "json",
    "--disable-hook",
    "all",
  ]);
  for h in engine::CDI_HOOKS {
    c.args(["--enable-hook", h]);
  }
  c.arg("--output").arg(&path);
  let st = c.output().await.map_err(|e| format!("nvidia-ctk: {e}"))?;
  if !st.status.success() {
    return Err(format!(
      "nvidia-ctk cdi generate failed: {}",
      String::from_utf8_lossy(&st.stderr).trim()
    ));
  }
  let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
  let v: Value = serde_json::from_str(&text).map_err(|e| format!("the CDI spec: {e}"))?;
  engine::check_cdi(&v)?;
  Ok(dir)
}

/// Runs command `name` of the app's run grant with `args`, its outputs
/// into `data` (the app's data folder) under `out/`, within `quota`
/// bytes for the folder.
pub(crate) async fn run(
  s: &Arc<Shared>,
  owner: &Owner,
  data: &Path,
  quota: u64,
  name: &str,
  args: &serde_json::Map<String, Value>,
) -> Result<Outcome, String> {
  let g = super::granted(s, owner, Kind::Run).ok_or_else(|| {
    format!(
      "app \"{}\" has no run grant: the user grants its commands with `stitch app \
       allow-container`",
      owner.name
    )
  })?;
  let cmd = g.spec.commands.get(name).ok_or_else(|| {
    format!(
      "no command {name} in app \"{}\"'s grant; it has {}",
      owner.name,
      g.spec
        .commands
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(", ")
    )
  })?;
  let (argv, copies) = argv(cmd, args)?;
  launch(s, owner, &g.spec, cmd, name, &argv, &copies, data, quota).await
}

/// Runs a command line of the app's thread in its image (the shell
/// grant): `/bin/sh -c <line>`, isolated as a command is, the data
/// folder's `work/` copied in at `/in/work`, outputs from `/out`.
pub(crate) async fn shell(
  s: &Arc<Shared>,
  owner: &Owner,
  data: &Path,
  quota: u64,
  line: &str,
) -> Result<Outcome, String> {
  let g = super::granted(s, owner, Kind::Shell).ok_or_else(|| {
    format!(
      "app \"{}\" has no shell grant: the user grants it with `stitch app allow-container`",
      owner.name
    )
  })?;
  let sh = g.spec.shell.clone().unwrap_or_default();
  if line.len() > 16 * 1024 || line.contains('\0') {
    return Err("a command line of at most 16 KiB".into());
  }
  let cmd = Command {
    run: vec!["/bin/sh".into(), "-c".into(), line.to_owned()],
    timeout: sh.timeout,
    memory_mb: sh.memory_mb,
    ..Default::default()
  };
  let mut cmd = cmd;
  cmd.cpus = Some(spec::CPUS);
  cmd.pids = Some(spec::PIDS);
  cmd.max_files = Some(spec::OUT_FILES);
  cmd.max_mb = Some(spec::OUT_MB);
  // The work folder's files, links and hidden parts left out.
  let mut copies = Vec::new();
  let work = data.join("work");
  if std::fs::symlink_metadata(&work).is_ok_and(|m| m.is_dir()) {
    let mut total = 0u64;
    for e in walkdir::WalkDir::new(&work)
      .follow_links(false)
      .max_depth(16)
      .into_iter()
      .flatten()
    {
      if !e.file_type().is_file() {
        continue;
      }
      let Ok(rel) = e.path().strip_prefix(data) else {
        continue;
      };
      let rel = rel.to_string_lossy().into_owned();
      if !spec::plain_rel(&rel) {
        continue;
      }
      total += e.metadata().map(|m| m.len()).unwrap_or(0);
      if total > MAX_INPUT || copies.len() >= 2000 {
        return Err("work/ holds more than 256 MiB or 2000 files to copy in".into());
      }
      copies.push((rel.clone(), format!("/in/{rel}")));
    }
  }
  let argv = cmd.run.clone();
  launch(
    s, owner, &g.spec, &cmd, "shell", &argv, &copies, data, quota,
  )
  .await
}

#[allow(clippy::too_many_arguments)]
async fn launch(
  s: &Arc<Shared>,
  owner: &Owner,
  c: &spec::ContainerNeeds,
  cmd: &Command,
  name: &str,
  argv: &[String],
  copies: &[(String, String)],
  data: &Path,
  quota: u64,
) -> Result<Outcome, String> {
  let key = app_key(owner);
  let daily = c.daily_minutes.unwrap_or(spec::DAILY_MINUTES) * 60;
  let used = super::used_today(s, &key);
  if used >= daily {
    return Err(format!(
      "app \"{}\" used its {} container minutes today",
      owner.name,
      daily / 60
    ));
  }
  let (podman, _) = engine::engine().await?;
  let pins = c.pins.clone().ok_or("the grant has no pins")?;
  let image = image(s, owner, &pins).await?;
  let _gpu = if cmd.gpu {
    Some(gpu().acquire().await.map_err(|e| e.to_string())?)
  } else {
    None
  };
  let cdi_dir = if cmd.gpu {
    Some(cdi(super::dir(s).join("cdi")).await?)
  } else {
    None
  };
  let id = format!(
    "{}-{}",
    jiff::Zoned::now().strftime("%Y%m%d-%H%M%S"),
    &blake3::hash(format!("{:?}{}", Instant::now(), name).as_bytes()).to_hex()[..6]
  );
  let root = super::build::scratch(s).join(&id);
  let (input, output) = (root.join("in"), root.join("out"));
  let r = run_in(
    s,
    owner,
    &podman,
    cmd,
    name,
    &id,
    &image,
    argv,
    copies,
    &input,
    &output,
    data,
    quota,
    &pins,
    cdi_dir.as_deref(),
    c,
  )
  .await;
  // The run's files: the node's, but owned by the container's user while
  // it ran.
  let _ = podman
    .output(
      &[
        "unshare",
        "chown",
        "-R",
        "-h",
        "0:0",
        &root.display().to_string(),
      ],
      Duration::from_secs(60),
    )
    .await;
  let _ = std::fs::remove_dir_all(&root);
  if let Ok(o) = &r {
    super::count(s, &key, o.seconds.max(1));
  }
  r
}

#[allow(clippy::too_many_arguments)]
async fn run_in(
  s: &Arc<Shared>,
  owner: &Owner,
  podman: &Podman,
  cmd: &Command,
  name: &str,
  id: &str,
  image: &str,
  argv: &[String],
  copies: &[(String, String)],
  input: &Path,
  output: &Path,
  data: &Path,
  quota: u64,
  pins: &spec::Pins,
  cdi_dir: Option<&Path>,
  c: &spec::ContainerNeeds,
) -> Result<Outcome, String> {
  std::fs::create_dir_all(input).map_err(|e| e.to_string())?;
  std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
  for d in [input, output, input.parent().unwrap_or(input)] {
    let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o755));
  }
  // The arguments' files, from the data folder, links refused.
  for (rel, inside) in copies {
    let src = file_in(data, rel)?;
    let dst = input.join(inside.trim_start_matches("/in/"));
    copy_in(&src, &dst, MAX_INPUT, true)?;
  }
  // The declared inputs, each checked against its pin as copied.
  for (n, i) in &cmd.inputs {
    let from = crate::place::expand_home(&i.from);
    if crate::apps::threads::holds_data(&s.data, &from) {
      return Err(format!(
        "input {n}: {} holds the node's data",
        from.display()
      ));
    }
    for (f, want) in &i.files {
      let got = copy_in(&from.join(f), &input.join(n).join(f), MAX_INPUT, false)?;
      if &got != want {
        return Err(format!(
          "input {n}: {f} has changed since it was pinned (sha256 {got}, pinned {want}): \
           nothing ran; the app pins it again and the user grants the change"
        ));
      }
    }
  }
  // The output folder, the container user's.
  let (code, _, err) = podman
    .output(
      &[
        "unshare",
        "chown",
        "1000:1000",
        &output.display().to_string(),
      ],
      Duration::from_secs(30),
    )
    .await?;
  if code != 0 {
    return Err(format!("the output folder: {}", err.trim()));
  }
  let seccomp = engine::write_profile(&super::dir(s)).map_err(|e| e.to_string())?;
  let node = node_key(&s.data);
  let key = app_key(owner);
  let cname = format!("stitch-run-{id}");
  let labels = vec![
    format!("{L_NODE}={node}"),
    format!("{L_APP}={key}"),
    format!("{L_NAME}={}", owner.name.replace(['\n', '\r'], " ")),
    format!("{L_RUN}={id}"),
    format!("{}={}", super::build::L_PINS, pins.id()),
  ];
  let cache = match &c.cache {
    Some(cc) => Some((
      super::storage::cache_volume(s, &key).await?,
      cc.path.clone(),
    )),
    None => None,
  };
  let args = podman_args(
    cmd,
    image,
    argv,
    &cname,
    &labels,
    input,
    output,
    &seccomp,
    cache.as_ref().map(|(v, p)| (v.as_str(), p.as_str())),
    cdi_dir,
  )?;
  let mut reaper = Reaper {
    podman: podman.clone(),
    name: cname.clone(),
    armed: true,
  };
  let o = execute(podman, cmd, &args, output).await?;
  remove(podman, &cname, id).await?;
  reaper.armed = false;
  if let (Some(cc), Some((vol, _))) = (&c.cache, &cache) {
    super::storage::bound_cache(podman, vol, cc.mb).await;
  }
  // Read back as hostile, now that nothing writes.
  let (code, _, err) = podman
    .output(
      &[
        "unshare",
        "chown",
        "-R",
        "-h",
        "0:0",
        &output.display().to_string(),
      ],
      Duration::from_secs(60),
    )
    .await?;
  if code != 0 {
    return Err(format!(
      "the outputs could not be taken back: {}",
      err.trim()
    ));
  }
  let mut o = o;
  let max_files = cmd.max_files.unwrap_or(spec::OUT_FILES) as usize;
  let max_bytes = cmd.max_mb.unwrap_or(spec::OUT_MB) << 20;
  let (files, skipped) = collect(output, max_files, max_bytes);
  o.skipped.extend(skipped);
  // Into the data folder, within its quota.
  let used = super::super::files::usage(data).0;
  let adding: u64 = files.iter().map(|(_, b)| b.len() as u64).sum();
  if used + adding > quota {
    o.skipped.push(format!(
      "all outputs: the data folder would pass its quota ({} MiB used, {} MiB more, quota {} \
       MiB)",
      used >> 20,
      adding >> 20,
      quota >> 20
    ));
    return Ok(o);
  }
  if !files.is_empty() {
    let folder = format!("out/{name}-{id}");
    let base = super::super::files::make_dir(data, &folder)?;
    for (rel, bytes) in files {
      let dst = base.join(&rel);
      if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
      }
      std::fs::write(&dst, &bytes).map_err(|e| format!("{}: {e}", dst.display()))?;
      let image = crate::apps::threads::sniff(&bytes)
        .filter(|t| *t != "image/gif")
        .filter(|_| (bytes.len() as u64) <= MAX_IMAGE)
        .filter(|_| crate::apps::threads::decodes(&bytes).is_ok());
      o.made.push(Made {
        rel: format!("{folder}/{rel}"),
        bytes: bytes.len() as u64,
        image,
      });
    }
    o.folder = Some(folder);
  }
  Ok(o)
}

/// Runs podman's argv to its end or its limits: the command's time, its
/// outputs' size and count (measured as it runs).
pub(crate) async fn execute(
  podman: &Podman,
  cmd: &Command,
  args: &[String],
  output: &Path,
) -> Result<Outcome, String> {
  use tokio::io::AsyncReadExt;
  let mut c = podman.command();
  c.args(args);
  c.stdout(std::process::Stdio::piped());
  c.stderr(std::process::Stdio::piped());
  let started = Instant::now();
  let mut child = c.spawn().map_err(|e| format!("podman run: {e}"))?;
  let (mut so, mut se) = (
    child.stdout.take().expect("piped"),
    child.stderr.take().expect("piped"),
  );
  let keep = |buf: &mut Vec<u8>, chunk: &[u8]| {
    buf.extend_from_slice(chunk);
    if buf.len() > 4 * KEEP_OUTPUT {
      buf.drain(..buf.len() - 2 * KEEP_OUTPUT);
    }
  };
  let timeout = Duration::from_secs(cmd.timeout.unwrap_or(spec::TIMEOUT));
  let max_bytes = cmd.max_mb.unwrap_or(spec::OUT_MB) << 20;
  let max_files = cmd.max_files.unwrap_or(spec::OUT_FILES);
  let (mut out, mut err) = (Vec::new(), Vec::new());
  let (mut a, mut b) = (vec![0u8; 8192], vec![0u8; 8192]);
  let (mut o_done, mut e_done) = (false, false);
  let mut stopped = None;
  let mut status = None;
  let mut tick = tokio::time::interval(Duration::from_millis(500));
  let deadline = tokio::time::sleep(timeout);
  tokio::pin!(deadline);
  loop {
    tokio::select! {
      n = so.read(&mut a), if !o_done => match n {
        Ok(0) | Err(_) => o_done = true,
        Ok(n) => keep(&mut out, &a[..n]),
      },
      n = se.read(&mut b), if !e_done => match n {
        Ok(0) | Err(_) => e_done = true,
        Ok(n) => keep(&mut err, &b[..n]),
      },
      st = child.wait(), if status.is_none() => {
        status = Some(st.map_err(|e| e.to_string())?);
      }
      _ = &mut deadline => {
        stopped = Some(format!("its {} s were up: the container was killed", timeout.as_secs()));
        break;
      }
      _ = tick.tick() => {
        if let Some((bytes, inodes)) = measure(podman, output).await
          && (bytes > max_bytes + (1 << 20) || inodes > max_files + 64)
        {
          stopped = Some(format!(
            "its outputs passed their caps ({} MiB, {} files; at most {} MiB, {} files): the \
             container was killed",
            bytes >> 20, inodes, max_bytes >> 20, max_files
          ));
          break;
        }
      }
    }
    if status.is_some() && o_done && e_done {
      break;
    }
  }
  if stopped.is_some() {
    let _ = child.start_kill();
  }
  let exit = status.and_then(|s| s.code());
  Ok(Outcome {
    exit,
    seconds: started.elapsed().as_secs(),
    stdout: cut(&out),
    stderr: cut(&err),
    stopped,
    ..Default::default()
  })
}

/// A name as a file of the outputs may have.
fn good_name(n: &str) -> bool {
  !n.is_empty()
    && n.len() <= 255
    && !n.starts_with('.')
    && !n.chars().any(|c| c.is_control() || c == '\\')
}

/// The outputs read back, as hostile: no link followed, regular files
/// only, at most `max_files` and `max_bytes` together. The files (path in
/// the output folder → bytes) and what was left out.
pub fn collect(
  root: &Path,
  max_files: usize,
  max_bytes: u64,
) -> (Vec<(String, Vec<u8>)>, Vec<String>) {
  let mut files = Vec::new();
  let mut skipped = Vec::new();
  let mut total = 0u64;
  let mut stack = vec![(root.to_owned(), String::new(), 0usize)];
  while let Some((dir, rel, depth)) = stack.pop() {
    // The node owns it now: opened for reading if closed.
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    let Ok(rd) = std::fs::read_dir(&dir) else {
      skipped.push(format!("{rel}/: not readable"));
      continue;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
      let name = e.file_name().to_string_lossy().into_owned();
      let path = e.path();
      let r = if rel.is_empty() {
        name.clone()
      } else {
        format!("{rel}/{name}")
      };
      if !good_name(&name) {
        skipped.push(format!(
          "{r}: not a name taken (hidden, control characters)"
        ));
        continue;
      }
      let Ok(m) = std::fs::symlink_metadata(&path) else {
        continue;
      };
      let ft = m.file_type();
      if ft.is_symlink() {
        skipped.push(format!("{r}: a link, not followed"));
        continue;
      }
      if ft.is_dir() {
        if depth >= 8 {
          skipped.push(format!("{r}/: deeper than 8 folders"));
        } else {
          stack.push((path, r, depth + 1));
        }
        continue;
      }
      if !ft.is_file() {
        skipped.push(format!(
          "{r}: not a regular file (a FIFO, socket or device)"
        ));
        continue;
      }
      if files.len() >= max_files {
        skipped.push(format!("{r}: past the {max_files} files"));
        continue;
      }
      if total + m.len() > max_bytes {
        skipped.push(format!("{r}: past the {} MiB of outputs", max_bytes >> 20));
        continue;
      }
      // Opened without following a link, not blocking; the same file as
      // looked at.
      let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path);
      let Ok(f) = f else {
        skipped.push(format!("{r}: not readable"));
        continue;
      };
      let Ok(fm) = f.metadata() else { continue };
      if !fm.is_file() || fm.ino() != m.ino() || fm.dev() != m.dev() {
        skipped.push(format!("{r}: changed while read"));
        continue;
      }
      let mut bytes = Vec::new();
      if f
        .take(max_bytes - total + 1)
        .read_to_end(&mut bytes)
        .is_err()
      {
        skipped.push(format!("{r}: not readable"));
        continue;
      }
      if total + bytes.len() as u64 > max_bytes {
        skipped.push(format!("{r}: past the {} MiB of outputs", max_bytes >> 20));
        continue;
      }
      total += bytes.len() as u64;
      files.push((r, bytes));
    }
  }
  files.sort_by(|a, b| a.0.cmp(&b.0));
  (files, skipped)
}

/// The tool's result: what the model reads, and the images to attach.
pub fn said(name: &str, o: &Outcome) -> String {
  let mut t = format!(
    "{name}: {} in {} s.",
    match (&o.stopped, o.exit) {
      (Some(why), _) => format!("stopped ({why})"),
      (None, Some(0)) => "done (exit 0)".to_owned(),
      (None, Some(c)) => format!("failed (exit {c})"),
      (None, None) => "ended by a signal".to_owned(),
    },
    o.seconds
  );
  if let Some(f) = &o.folder {
    t.push_str(&format!("\nIts files, in the app's data folder, {f}/:"));
    for m in &o.made {
      t.push_str(&format!(
        "\n  {} ({} bytes{})",
        m.rel,
        m.bytes,
        if m.image.is_some() {
          ", image, attached"
        } else {
          ""
        }
      ));
    }
  } else {
    t.push_str("\nNo files.");
  }
  for sk in &o.skipped {
    t.push_str(&format!("\nLeft out: {sk}"));
  }
  if !o.stdout.trim().is_empty() {
    t.push_str(&format!("\n--- stdout ---\n{}", o.stdout.trim_end()));
  }
  if !o.stderr.trim().is_empty() {
    t.push_str(&format!("\n--- stderr ---\n{}", o.stderr.trim_end()));
  }
  t
}

/// The images of an outcome to attach (at most `MAX_IMAGES`).
pub fn images(o: &Outcome) -> Vec<&Made> {
  o.made
    .iter()
    .filter(|m| m.image.is_some())
    .take(MAX_IMAGES)
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::os::unix::fs::symlink;

  fn cmd() -> Command {
    let mut c = Command {
      run: vec![
        "/opt/b".into(),
        "--".into(),
        "{recipe}".into(),
        "{n}".into(),
        "{out}".into(),
      ],
      ..Default::default()
    };
    c.args.insert(
      "recipe".into(),
      Arg::File {
        dir: "projects".into(),
        ext: vec!["json".into()],
      },
    );
    c.args.insert("n".into(), Arg::Int { min: 1, max: 9 });
    c.args.insert("out".into(), Arg::Out { fresh: true });
    c
  }

  fn args(v: Value) -> serde_json::Map<String, Value> {
    v.as_object().unwrap().clone()
  }

  #[test]
  fn arguments_typed_and_whole() {
    let c = cmd();
    let (got, copies) = argv(
      &c,
      &args(serde_json::json!({"recipe": "projects/skirt/recipe.json", "n": 3})),
    )
    .unwrap();
    assert_eq!(
      got,
      vec![
        "/opt/b",
        "--",
        "/in/args/recipe/recipe.json",
        "3",
        "/out/result"
      ]
    );
    assert_eq!(
      copies,
      vec![(
        "projects/skirt/recipe.json".into(),
        "/in/args/recipe/recipe.json".into()
      )]
    );
    for bad in [
      serde_json::json!({"recipe": "../x.json", "n": 3}),
      serde_json::json!({"recipe": "projects/../../x.json", "n": 3}),
      serde_json::json!({"recipe": "other/x.json", "n": 3}),
      serde_json::json!({"recipe": "projects/x.sh", "n": 3}),
      serde_json::json!({"recipe": "projects/x.json", "n": 10}),
      serde_json::json!({"recipe": "projects/x.json", "n": "3; rm -rf /"}),
      serde_json::json!({"recipe": "projects/x.json", "n": 3, "out": "/etc"}),
      serde_json::json!({"recipe": "projects/x.json", "n": 3, "extra": "--python-expr"}),
      serde_json::json!({"recipe": "projects/.hidden/x.json", "n": 3}),
    ] {
      assert!(argv(&c, &args(bad.clone())).is_err(), "{bad}");
    }
  }

  #[test]
  fn data_files_without_links() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    std::fs::create_dir_all(root.join("projects/a")).unwrap();
    std::fs::write(root.join("projects/a/r.json"), "{}").unwrap();
    std::fs::write(d.path().join("secret"), "s").unwrap();
    symlink(d.path().join("secret"), root.join("projects/a/link.json")).unwrap();
    symlink(root.join("projects/a"), root.join("projects/b")).unwrap();
    assert!(file_in(root, "projects/a/r.json").is_ok());
    assert!(file_in(root, "projects/a/link.json").is_err());
    assert!(file_in(root, "projects/b/r.json").is_err());
    assert!(file_in(root, "projects/a").is_err());
    assert!(file_in(root, "projects/../secret").is_err());
  }

  #[test]
  fn outputs_read_as_hostile() {
    let d = tempfile::tempdir().unwrap();
    let out = d.path().join("out");
    std::fs::create_dir_all(out.join("sub/deeper")).unwrap();
    std::fs::write(out.join("a.png"), b"x").unwrap();
    std::fs::write(out.join("sub/deeper/b.glb"), b"yy").unwrap();
    std::fs::write(d.path().join("secret"), b"KEY").unwrap();
    symlink(d.path().join("secret"), out.join("leak.txt")).unwrap();
    symlink("/", out.join("rootdir")).unwrap();
    std::fs::write(out.join(".hidden"), b"h").unwrap();
    assert!(
      std::process::Command::new("mkfifo")
        .arg(out.join("pipe"))
        .status()
        .unwrap()
        .success()
    );
    // A folder closed to its owner is opened again (the node owns it).
    std::fs::create_dir(out.join("closed")).unwrap();
    std::fs::write(out.join("closed/c.txt"), b"c").unwrap();
    std::fs::set_permissions(out.join("closed"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let (files, skipped) = collect(&out, 10, 1 << 20);
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["a.png", "closed/c.txt", "sub/deeper/b.glb"]);
    let said = skipped.join("\n");
    assert!(said.contains("leak.txt: a link"), "{said}");
    assert!(said.contains("rootdir: a link"), "{said}");
    assert!(said.contains("pipe: not a regular file"), "{said}");
    assert!(said.contains(".hidden"), "{said}");
    assert!(!files.iter().any(|(_, b)| b == b"KEY"));
    // Caps.
    let (files, skipped) = collect(&out, 1, 1 << 20);
    assert_eq!(files.len(), 1);
    assert!(skipped.iter().any(|s| s.contains("past the 1 files")));
    std::fs::write(out.join("big.bin"), vec![0u8; 3000]).unwrap();
    let (files, _) = collect(&out, 10, 2048);
    assert!(!files.iter().any(|(n, _)| n == "big.bin"));
  }

  #[test]
  fn the_invocation_is_all_stitchs() {
    let c = cmd();
    let a = podman_args(
      &c,
      "abc123",
      &["/opt/b".into(), "--".into(), "x".into()],
      "stitch-run-1",
      &["stitch.run=1".into()],
      Path::new("/d/in"),
      Path::new("/d/out"),
      Path::new("/d/seccomp.json"),
      None,
      None,
    )
    .unwrap();
    for must in [
      "--rm",
      "--init",
      "--pull=never",
      "--network=none",
      "--read-only",
      "--cap-drop=all",
      "--security-opt=no-new-privileges",
      "--pid=private",
      "--ipc=private",
      "--log-driver=none",
      "--user=1000:1000",
      "--security-opt=seccomp=/d/seccomp.json",
      "--volume=/d/in:/in:ro,noexec,nosuid,nodev",
      "--volume=/d/out:/out:rw,noexec,nosuid,nodev",
      "--entrypoint=[\"/opt/b\"]",
    ] {
      assert!(a.iter().any(|x| x == must), "{must} missing: {a:?}");
    }
    // The image, then the arguments, each whole.
    let i = a.iter().position(|x| x == "abc123").unwrap();
    assert_eq!(&a[i + 1..], &["--", "x"]);
    assert!(
      !a.iter()
        .any(|x| x.contains("keep-id") || x.contains("privileged"))
    );
    // A path podman would split is refused.
    assert!(
      podman_args(
        &c,
        "i",
        &["/b".into()],
        "n",
        &[],
        Path::new("/d/a:b"),
        Path::new("/d/out"),
        Path::new("/s"),
        None,
        None
      )
      .is_err()
    );
  }
}
