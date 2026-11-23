//! The container engine (design §2.23): rootless podman, checked before
//! every build and run, called by its CLI with an argv and a cleared
//! environment, never through its socket or a shell; and the seccomp
//! profile Stitch passes.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

/// What `podman info` says that matters here.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
  pub version: String,
  pub rootless: bool,
  pub remote: bool,
  pub cgroup_v2: bool,
  pub controllers: Vec<String>,
  pub seccomp: bool,
  pub graph_root: PathBuf,
}

impl Info {
  /// From `podman info --format json`.
  pub fn parse(json: &str) -> Result<Info, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("podman info: {e}"))?;
    let host = v.get("host").ok_or("podman info: no host")?;
    let s = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("").to_owned();
    Ok(Info {
      version: s(v.pointer("/version/Version")),
      rootless: host
        .pointer("/security/rootless")
        .and_then(Value::as_bool)
        .unwrap_or(false),
      remote: host
        .get("serviceIsRemote")
        .and_then(Value::as_bool)
        .unwrap_or(false),
      cgroup_v2: s(host.get("cgroupVersion")) == "v2",
      controllers: host
        .get("cgroupControllers")
        .and_then(Value::as_array)
        .map(|a| {
          a.iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
        })
        .unwrap_or_default(),
      seccomp: host
        .pointer("/security/seccompEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(false),
      graph_root: PathBuf::from(s(v.pointer("/store/graphRoot"))),
    })
  }

  /// Why it cannot run an app's containers, if it cannot.
  pub fn refusal(&self) -> Option<String> {
    if self.remote {
      return Some(
        "podman here is a remote client (CONTAINER_HOST or a connection): app containers run \
         only on a local rootless podman"
          .into(),
      );
    }
    if !self.rootless {
      return Some(
        "podman here runs as root: app containers run only in rootless podman (a rootful \
         engine is root-equivalent)"
          .into(),
      );
    }
    if !self.cgroup_v2 {
      return Some(
        "this machine has no cgroup v2: rootless podman cannot enforce memory and process \
         limits, so app containers are refused"
          .into(),
      );
    }
    let missing: Vec<&str> = ["memory", "pids", "cpu"]
      .into_iter()
      .filter(|c| !self.controllers.iter().any(|x| x == c))
      .collect();
    if !missing.is_empty() {
      return Some(format!(
        "the cgroup controllers {} are not delegated to this user (systemd's \
         user@.service Delegate=): their limits would be ignored, so app containers are refused",
        missing.join(", ")
      ));
    }
    if !self.seccomp {
      return Some("podman here has no seccomp: app containers are refused".into());
    }
    None
  }
}

/// The variables podman is run with; everything else is cleared, so a
/// `CONTAINER_HOST`, `DOCKER_HOST` or connection setting never reaches it.
const KEPT: &[&str] = &["PATH", "HOME", "XDG_RUNTIME_DIR", "USER", "LOGNAME", "LANG"];

/// The podman CLI.
#[derive(Clone, Debug)]
pub struct Podman {
  pub program: PathBuf,
}

impl Podman {
  /// The `podman` on the PATH (never `docker`), when there is one.
  pub fn find() -> Option<Podman> {
    if cfg!(not(target_os = "linux")) {
      return None;
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
      .map(|d| d.join("podman"))
      .find(|p| p.is_file())
      .map(|program| Podman { program })
  }

  /// A command of it, with the cleared environment.
  pub fn command(&self) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(&self.program);
    c.env_clear();
    for k in KEPT {
      if let Some(v) = std::env::var_os(k) {
        c.env(k, v);
      }
    }
    c.stdin(std::process::Stdio::null());
    c.kill_on_drop(true);
    c
  }

  /// Runs it to the end within `limit`: its exit status, stdout, stderr
  /// (each cut at 1 MiB).
  pub async fn output(
    &self,
    args: &[&str],
    limit: Duration,
  ) -> Result<(i32, String, String), String> {
    let mut c = self.command();
    c.args(args);
    let out = tokio::time::timeout(limit, c.output())
      .await
      .map_err(|_| format!("podman {} took longer than {limit:?}", args.join(" ")))?
      .map_err(|e| format!("podman: {e}"))?;
    let cut = |b: &[u8]| String::from_utf8_lossy(&b[..b.len().min(1 << 20)]).into_owned();
    Ok((
      out.status.code().unwrap_or(-1),
      cut(&out.stdout),
      cut(&out.stderr),
    ))
  }

  /// `podman info`, checked: the engine, or why app containers are
  /// refused here.
  pub async fn info(&self) -> Result<Info, String> {
    let (code, out, err) = self
      .output(&["info", "--format", "json"], Duration::from_secs(30))
      .await?;
    if code != 0 {
      return Err(format!("podman info failed: {}", err.trim()));
    }
    let info = Info::parse(&out)?;
    match info.refusal() {
      Some(why) => Err(why),
      None => Ok(info),
    }
  }
}

/// The engine as last checked (a minute), so each run does not ask again.
pub(crate) struct Checked {
  at: Option<(Instant, Result<Info, String>)>,
}

impl Checked {
  pub(crate) const fn new() -> Checked {
    Checked { at: None }
  }
}

static CHECKED: tokio::sync::Mutex<Checked> = tokio::sync::Mutex::const_new(Checked::new());

/// The podman to use, checked (rootless, local, cgroup v2 with its
/// controllers, seccomp), or why app containers cannot run here.
pub async fn engine() -> Result<(Podman, Info), String> {
  if cfg!(not(target_os = "linux")) {
    return Err(
      "app containers run only on Linux for now (on macOS they would need a podman machine VM \
       without shared folders: not built)"
        .into(),
    );
  }
  let podman = Podman::find().ok_or(
    "no podman here: app containers need rootless podman (install podman, with subordinate IDs \
     for this user in /etc/subuid and /etc/subgid)",
  )?;
  let mut c = CHECKED.lock().await;
  if let Some((at, r)) = &c.at
    && at.elapsed() < Duration::from_secs(60)
  {
    return r.clone().map(|i| (podman.clone(), i));
  }
  let r = podman.info().await;
  c.at = Some((Instant::now(), r.clone()));
  r.map(|i| (podman, i))
}

/// Podman's default seccomp profile (containers-common 0.69, Apache-2.0),
/// which Stitch makes stricter (`profile`).
const DEFAULT_SECCOMP: &str = include_str!("seccomp-default.json");

/// What Stitch's profile takes away from podman's default: nested
/// namespaces (podman's default allows `unshare -r`, verified on gamer),
/// mounts, keyrings, tracing, BPF, perf, userfaultfd and io_uring.
pub const DENIED: &[&str] = &[
  "unshare",
  "setns",
  "mount",
  "umount",
  "umount2",
  "pivot_root",
  "fsopen",
  "fsconfig",
  "fsmount",
  "fspick",
  "move_mount",
  "open_tree",
  "mount_setattr",
  "keyctl",
  "add_key",
  "request_key",
  "ptrace",
  "process_vm_readv",
  "process_vm_writev",
  "pidfd_getfd",
  "name_to_handle_at",
  "open_by_handle_at",
  "bpf",
  "perf_event_open",
  "userfaultfd",
  "io_uring_setup",
  "io_uring_enter",
  "io_uring_register",
  "clone",
  "clone3",
  "chroot",
  "lookup_dcookie",
  "kexec_load",
  "kexec_file_load",
  "init_module",
  "finit_module",
  "delete_module",
  "quotactl",
  "acct",
];

/// The namespace flags `clone` may not carry: CLONE_NEWNS, NEWCGROUP,
/// NEWUTS, NEWIPC, NEWUSER, NEWPID, NEWNET (and NEWTIME).
pub const CLONE_NS: u64 = 0x0002_0000
  | 0x0200_0000
  | 0x0400_0000
  | 0x0800_0000
  | 0x1000_0000
  | 0x2000_0000
  | 0x4000_0000
  | 0x80;

/// Stitch's seccomp profile: podman's default with `DENIED` taken out of
/// every rule that allows, `clone` allowed only without namespace flags,
/// `clone3` answering ENOSYS (its flags are behind a pointer; libcs fall
/// back to `clone`).
pub fn profile() -> Value {
  let mut p: Value = serde_json::from_str(DEFAULT_SECCOMP).expect("the default profile parses");
  let rules = p["syscalls"].as_array_mut().expect("syscalls");
  for r in rules.iter_mut() {
    if r["action"] == "SCMP_ACT_ALLOW"
      && let Some(names) = r["names"].as_array_mut()
    {
      names.retain(|n| !DENIED.contains(&n.as_str().unwrap_or("")));
    }
  }
  rules.retain(|r| {
    r["action"] != "SCMP_ACT_ALLOW" || r["names"].as_array().is_some_and(|n| !n.is_empty())
  });
  rules.push(serde_json::json!({
    "names": ["clone"],
    "action": "SCMP_ACT_ALLOW",
    "args": [{"index": 0, "value": CLONE_NS, "valueTwo": 0, "op": "SCMP_CMP_MASKED_EQ"}],
    "comment": "stitch: clone without namespace flags",
    "includes": {}, "excludes": {}
  }));
  rules.push(serde_json::json!({
    "names": ["clone3"],
    "action": "SCMP_ACT_ERRNO",
    "errnoRet": 38,
    "comment": "stitch: clone3 answers ENOSYS, so libcs use clone, whose flags are checked",
    "includes": {}, "excludes": {}
  }));
  p
}

/// The profile written where podman reads it (the node's containers
/// directory), its path.
pub fn write_profile(dir: &Path) -> std::io::Result<PathBuf> {
  std::fs::create_dir_all(dir)?;
  let path = dir.join("seccomp.json");
  let text = serde_json::to_vec_pretty(&profile()).expect("json");
  if std::fs::read(&path).ok().as_deref() != Some(&text[..]) {
    let tmp = dir.join(".seccomp.json.tmp");
    std::fs::write(&tmp, &text)?;
    std::fs::rename(&tmp, &path)?;
  }
  Ok(path)
}

/// `nvidia-ctk --version`'s version, as numbers.
pub fn nvidia_ctk_version(out: &str) -> Option<(u32, u32, u32)> {
  let v = out
    .split_whitespace()
    .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()) && w.contains('.'))?;
  let mut it = v.split(['.', '-', '+']).map(|n| n.parse::<u32>().ok());
  Some((it.next()??, it.next()??, it.next().flatten().unwrap_or(0)))
}

/// The hooks Stitch's CDI spec keeps: the libraries' links and cache,
/// and the hardening that stops the driver making device nodes.
pub const CDI_HOOKS: &[&str] = &[
  "create-symlinks",
  "update-ldcache",
  "disable-device-node-modification",
];

/// Whether a program is the system's: owned by root, writable by root
/// alone, its directory likewise.
pub fn root_owned(path: &Path) -> Result<PathBuf, String> {
  use std::os::unix::fs::MetadataExt;
  let canon = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
  for p in [canon.as_path(), canon.parent().unwrap_or(Path::new("/"))] {
    let m = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
    if m.uid() != 0 || m.mode() & 0o022 != 0 {
      return Err(format!(
        "{}: not the system's (owned by root, writable by root alone)",
        p.display()
      ));
    }
  }
  Ok(canon)
}

/// A CDI spec (JSON) checked: its hooks only `CDI_HOOKS`, each run by a
/// root-owned `nvidia-cdi-hook` or `nvidia-ctk`; its hooks' names.
pub fn check_cdi(spec: &Value) -> Result<Vec<String>, String> {
  let mut edits: Vec<&Value> = vec![&spec["containerEdits"]];
  if let Some(devs) = spec["devices"].as_array() {
    edits.extend(devs.iter().map(|d| &d["containerEdits"]));
  }
  let mut names = Vec::new();
  for e in edits {
    for h in e["hooks"].as_array().into_iter().flatten() {
      let path = h["path"].as_str().unwrap_or("");
      let prog = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
      if prog != "nvidia-cdi-hook" && prog != "nvidia-ctk" {
        return Err(format!("CDI hook {path}: not NVIDIA's hook"));
      }
      root_owned(Path::new(path))?;
      let args: Vec<&str> = h["args"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
      let name = args
        .iter()
        .skip(1)
        .find(|a| **a != "hook")
        .copied()
        .unwrap_or("");
      if !CDI_HOOKS.contains(&name) {
        return Err(format!("CDI hook {name}: not one Stitch keeps"));
      }
      if !names.iter().any(|n| n == name) {
        names.push(name.to_owned());
      }
    }
  }
  Ok(names)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn info(rootless: bool, remote: bool, v: &str, ctl: &[&str]) -> String {
    serde_json::json!({
      "host": {"cgroupVersion": v, "cgroupControllers": ctl, "serviceIsRemote": remote,
               "security": {"rootless": rootless, "seccompEnabled": true}},
      "store": {"graphRoot": "/home/u/.local/share/containers/storage"},
      "version": {"Version": "5.0.0"}
    })
    .to_string()
  }

  #[test]
  fn engines_refused() {
    let ok = Info::parse(&info(true, false, "v2", &["cpu", "memory", "pids", "io"])).unwrap();
    assert_eq!(ok.refusal(), None);
    let rootful = Info::parse(&info(false, false, "v2", &["cpu", "memory", "pids"])).unwrap();
    assert!(rootful.refusal().unwrap().contains("root"));
    let remote = Info::parse(&info(true, true, "v2", &["cpu", "memory", "pids"])).unwrap();
    assert!(remote.refusal().unwrap().contains("remote"));
    let v1 = Info::parse(&info(true, false, "v1", &[])).unwrap();
    assert!(v1.refusal().unwrap().contains("cgroup v2"));
    let undelegated = Info::parse(&info(true, false, "v2", &["cpu", "io"])).unwrap();
    let why = undelegated.refusal().unwrap();
    assert!(why.contains("memory") && why.contains("pids"), "{why}");
  }

  #[test]
  fn profile_takes_away() {
    let p = profile();
    let rules = p["syscalls"].as_array().unwrap();
    for r in rules {
      if r["action"] == "SCMP_ACT_ALLOW" {
        for n in r["names"].as_array().unwrap() {
          let n = n.as_str().unwrap();
          if n == "clone" {
            assert_eq!(r["args"][0]["op"], "SCMP_CMP_MASKED_EQ");
            assert_eq!(r["args"][0]["value"], CLONE_NS);
            continue;
          }
          assert!(!DENIED.contains(&n), "{n} still allowed");
        }
      }
    }
    // Still the default's allowances otherwise.
    assert!(rules.iter().any(|r| r["action"] == "SCMP_ACT_ALLOW"
      && r["names"].as_array().unwrap().iter().any(|n| n == "read")));
    assert!(
      rules
        .iter()
        .any(|r| r["names"][0] == "clone3" && r["errnoRet"] == 38)
    );
  }

  #[test]
  fn nvidia_versions() {
    assert_eq!(
      nvidia_ctk_version("NVIDIA Container Toolkit CLI version 1.20.1\ncommit: x"),
      Some((1, 20, 1))
    );
    assert_eq!(nvidia_ctk_version("version 1.16.2-rc.1"), Some((1, 16, 2)));
    assert!(
      nvidia_ctk_version("version 1.16.2").unwrap() < crate::apps::containers::spec::MIN_NVIDIA_CTK
    );
    assert_eq!(nvidia_ctk_version("nothing"), None);
  }

  #[test]
  fn cdi_hooks_checked() {
    let hook = |n: &str| serde_json::json!({"hookName": "createContainer", "path": "/usr/bin/true", "args": ["nvidia-cdi-hook", n]});
    let spec = serde_json::json!({"containerEdits": {"hooks": [hook("create-symlinks")]}});
    // `/usr/bin/true` is root's, but not NVIDIA's hook.
    assert!(check_cdi(&spec).unwrap_err().contains("not NVIDIA"));
    let spec =
      serde_json::json!({"containerEdits": {"hooks": []}, "devices": [{"containerEdits": {}}]});
    assert_eq!(check_cdi(&spec).unwrap(), Vec::<String>::new());
    assert!(root_owned(Path::new("/usr/bin/env")).is_ok());
    assert!(root_owned(&std::env::temp_dir()).is_err());
  }
}
