//! Local integration probe driven by the vault tests: one JSON command per stdin line,
//! one JSON reply per stdout line. Keys never appear on command lines; the harness
//! consumes stdout privately. Not a production CLI: never use with real keys.
use serde_json::{Value, json};
use std::{
  collections::BTreeMap,
  io::{self, BufRead, Write},
};
use stitch_vault::{
  Client, Credential, Device, Kind, Kit, Known, Limits, Membership, RecoverOptions, Role, Signer,
  keyring, sha256,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn image() -> Vec<u8> {
  hex::decode("89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000b49444154789c636000020000050001a5f645400000000049454e44ae426082").unwrap()
}
/// A valid Stitch record (parsed back) with the original image attached.
fn record(node: &str, kind: &str) -> Vec<u8> {
  use stitch_records::{Attachment, Author, BlobHash, Draft, Record};
  let image = image();
  let record = Draft::new(kind, Author::user(node.parse().unwrap()))
    .unwrap()
    .attach(Attachment {
      hash: BlobHash::of(&image),
      media_type: "image/png".into(),
      name: Some("original.png".into()),
      bytes: image.len() as u64,
    })
    .body("# Recovered work must remain inert\n")
    .into_record(
      format!("2026-10-27T120000Z-{node}-a2b3").parse().unwrap(),
      "2026-10-27T12:00:00Z".parse().unwrap(),
    );
  let text = record.to_text();
  Record::parse(&text).unwrap();
  text.into_bytes()
}

struct Writer {
  device: Device,
  client: Client,
  known: Known,
}
/// A real node store (record root and blob store) in a temporary directory.
struct Store {
  dir: std::path::PathBuf,
  root: stitch_store::RecordRoot,
  blobs: stitch_store::BlobStore,
  synced: BTreeMap<String, String>,
  ids: BTreeMap<String, String>,
}
impl Drop for Store {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.dir);
  }
}
struct Vault {
  /// Production-style signed requests (SIGNED_AUTH): no tokens, no bootstrap token.
  signed: bool,
  store: Option<Store>,
  endpoint: String,
  /// Local-prototype bootstrap token: posts the genesis and the first admin only.
  bootstrap: Client,
  kit: Kit,
  /// The first admin device: signs every later statement and posts it itself.
  admin: Option<String>,
  writers: BTreeMap<String, Writer>,
  devices: BTreeMap<String, String>,
}
impl Vault {
  fn vault(&self) -> &str {
    self.kit.vault()
  }
  /// The verified current membership (the next statement extends it).
  async fn membership(&self) -> Result<Membership> {
    let log = self.bootstrap_reader().statements().await?;
    let (m, refused) = Membership::verify(self.vault(), &log);
    if let Some(r) = refused {
      return Err(r.into());
    }
    Ok(m)
  }
  fn bootstrap_reader(&self) -> &Client {
    // Any enrolled credential can read the log; before the first admin, nothing is
    // enrolled, so the first admin's own client is used once it exists.
    self
      .admin
      .as_ref()
      .and_then(|a| self.writers.get(a))
      .map_or(&self.bootstrap, |w| &w.client)
  }
  async fn enroll(&mut self, name: &str, role: Role) -> Result<String> {
    // Admins are admitted by the kit (at once); everyone else by the first admin.
    let by = match (&self.admin, role) {
      (Some(a), Role::Writer | Role::Reader) => Some(a.clone()),
      _ => None,
    };
    self.enroll_by(name, role, by.as_deref(), None).await
  }
  /// Admit a new device, signed by `by` (a device name; None: the kit) and optionally
  /// co-signed by `cosign` (a device name or "kit"), posted with the signer's credential.
  async fn enroll_by(
    &mut self,
    name: &str,
    role: Role,
    by: Option<&str>,
    cosign: Option<&str>,
  ) -> Result<String> {
    let device = Device::generate()?;
    let token = format!("probe-{name}-local-test-credential");
    let client = if self.signed {
      Client::signed(&self.endpoint, self.vault(), device.request_signer()?)?
    } else {
      Client::new(&self.endpoint, self.vault(), token.clone())?
    };
    // The role the log gives it at once (a delayed admin is a writer until promoted).
    let now_role = |state: &Membership| match role {
      Role::Admin if !state.admin_ok(&device.public()) => "writer",
      r => r.as_str(),
    };
    let state = match &self.admin {
      None => {
        let log = self.bootstrap.statements().await.unwrap_or_default();
        Membership::verify(self.vault(), &log).0
      }
      Some(_) => self.membership().await?,
    };
    let cosigner = match cosign {
      Some("kit") => Some(Signer::Kit(&self.kit)),
      Some(n) => Some(Signer::Device(
        &self.writers.get(n).ok_or("unknown cosigner")?.device,
      )),
      None => None,
    };
    match by {
      None => {
        // The kit admits, the bootstrap credential (or the kit's anchor) posts.
        let log = self.bootstrap_reader().statements().await?;
        let (ring, _, _) = keyring(&log, &Credential::Kit(&self.kit))?;
        let s = state.add_cosigned(
          &Signer::Kit(&self.kit),
          cosigner.as_ref(),
          &ring,
          &device.public(),
          &device.seal_public(),
          role,
        )?;
        self.bootstrap.post_statement(&s).await?;
        let mut after = state.clone();
        after.apply(&s)?;
        let enrollment = json!({"token_hash": sha256(token.as_bytes()), "role": now_role(&after)});
        if !self.signed {
          self
            .bootstrap
            .put_device(&device.public(), &enrollment)
            .await?;
        }
        if self.admin.is_none() {
          self.admin = Some(name.into());
        }
      }
      Some(by) => {
        // A device signs, and posts with its own credential.
        let a = self.writers.get(by).ok_or("unknown signer")?;
        let (ring, _, _) = a
          .client
          .keyring(&Credential::Device {
            device: &a.device,
            vault: self.kit.vault(),
          })
          .await?;
        let s = state.add_cosigned(
          &Signer::Device(&a.device),
          cosigner.as_ref(),
          &ring,
          &device.public(),
          &device.seal_public(),
          role,
        )?;
        a.client.post_statement(&s).await?;
        let mut after = state.clone();
        after.apply(&s)?;
        let enrollment = json!({"token_hash": sha256(token.as_bytes()), "role": now_role(&after)});
        if !self.signed {
          a.client.put_device(&device.public(), &enrollment).await?;
        }
      }
    }
    self.devices.insert(name.into(), device.public());
    self.writers.insert(
      name.into(),
      Writer {
        device,
        client,
        known: Default::default(),
      },
    );
    Ok(token)
  }
}

/// One commit: values are text, {"record": kind}, {"png": true} or null (delete).
/// Deterministic pseudo-random bytes (incompressible, like media), optionally with
/// `insert` spliced in at `at`: an edited version of a large file.
fn file(seed: u64, size: usize, edit: Option<(usize, &str)>) -> Vec<u8> {
  let mut state = seed | 1;
  let mut data: Vec<u8> = (0..size)
    .map(|_| {
      state ^= state << 13;
      state ^= state >> 7;
      state ^= state << 17;
      state as u8
    })
    .collect();
  if let Some((at, insert)) = edit {
    data.splice(at..at, insert.bytes());
  }
  data
}
/// One commit: values are text, {"record": kind}, {"png": true}, {"file": {seed, size,
/// at?, insert?}} or null (delete). Returns cursor, sequence, packs and pack bytes uploaded.
async fn put(
  vault: &str,
  name: &str,
  w: &mut Writer,
  entries: &[Value],
) -> Result<(u64, u64, u64, u64)> {
  let (ring, _, _) = w
    .client
    .keyring(&Credential::Device {
      device: &w.device,
      vault,
    })
    .await?;
  let key = ring.current()?;
  let mut changes = vec![];
  for entry in entries {
    let entry_name = entry[0].as_str().ok_or("name")?.to_string();
    let value = &entry[1];
    let change = if value.is_null() {
      None
    } else if value["png"].as_bool() == Some(true) {
      Some((Kind::Blob, image()))
    } else if let Some(kind) = value["record"].as_str() {
      Some((Kind::Record, record(name, kind)))
    } else if value["file"].is_object() {
      let f = &value["file"];
      let edit = f["at"]
        .as_u64()
        .map(|at| (at as usize, f["insert"].as_str().unwrap_or("edit")));
      Some((
        Kind::File,
        file(
          f["seed"].as_u64().unwrap_or(1),
          f["size"].as_u64().ok_or("size")? as usize,
          edit,
        ),
      ))
    } else {
      Some((
        Kind::Record,
        value.as_str().ok_or("value")?.as_bytes().to_vec(),
      ))
    };
    changes.push((entry_name, change));
  }
  let (cursor, commit) = w
    .client
    .commit(key, &mut w.device, &mut w.known, &changes)
    .await?;
  if w.client.publish(&commit).await? != cursor {
    return Err("publication retry was not idempotent".into());
  }
  let bytes = commit.packs.iter().map(|p| p.size).sum();
  Ok((cursor, commit.sequence, commit.packs.len() as u64, bytes))
}

fn printable(bytes: &[u8]) -> Value {
  match std::str::from_utf8(bytes) {
    Ok(s) if !s.contains('\0') && bytes.len() <= 64 => json!(s),
    _ => json!(format!("bytes:{}:{}", bytes.len(), &sha256(bytes)[..16])),
  }
}
/// Recover with the kit (`kit`), or as an enrolled device of this probe (`as`).
async fn recover(cmd: &Value, vault: Option<&Vault>) -> Result<Value> {
  let kit = cmd["kit"].as_str().map(Kit::parse).transpose()?;
  let device = match cmd["as"].as_str() {
    Some(name) => Some(
      &vault
        .ok_or("init first")?
        .writers
        .get(name)
        .ok_or("unknown device")?
        .device,
    ),
    None => None,
  };
  let vault_id = match (&kit, vault) {
    (Some(k), _) => k.vault().to_string(),
    (None, Some(v)) => v.vault().to_string(),
    _ => return Err("kit or as required".into()),
  };
  let credential = match (&kit, device) {
    (_, Some(device)) => Credential::Device {
      device,
      vault: &vault_id,
    },
    (Some(kit), None) => Credential::Kit(kit),
    _ => return Err("kit or as required".into()),
  };
  let endpoint = cmd["endpoint"].as_str().ok_or("endpoint")?;
  let client = match (cmd["signed"].as_bool() == Some(true), device, &kit) {
    // Signed: the credential that decrypts also authenticates.
    (true, Some(device), _) => Client::signed(endpoint, &vault_id, device.request_signer()?)?,
    (true, None, Some(kit)) => Client::signed(endpoint, &vault_id, kit.request_signer()?)?,
    _ => Client::new(
      endpoint,
      &vault_id,
      cmd["token"].as_str().ok_or("token")?.into(),
    )?,
  };
  let pins = cmd["pins"]
    .as_object()
    .map(|o| {
      o.iter()
        .map(|(k, v)| (k.clone(), v.as_u64().unwrap_or(0)))
        .collect()
    })
    .unwrap_or_default();
  let options = RecoverOptions {
    through: cmd["through"].as_u64(),
    minimum_cursor: cmd["minimum_cursor"].as_u64().unwrap_or(0),
    minimum_members: cmd["minimum_members"].as_u64().unwrap_or(0),
    pins,
    limits: limits(&cmd["limits"]),
    checkpoints: cmd["checkpoints"].as_bool().unwrap_or(true),
  };
  // A fresh staging directory per recovery; listed for the harness, then removed. A
  // named one (`staging`) survives a failed run so that `resume` can continue it.
  static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
  let named = cmd["staging"].as_str();
  let staging = std::env::temp_dir().join(match named {
    Some(n) if n.bytes().all(|c| c.is_ascii_alphanumeric()) => {
      format!("stitch-vault-probe-{}-named-{n}", std::process::id())
    }
    Some(_) => return Err("staging name".into()),
    None => format!(
      "stitch-vault-probe-{}-{}",
      std::process::id(),
      NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ),
  });
  let result = if cmd["resume"].as_bool() == Some(true) {
    client.resume(&credential, &options, &staging).await
  } else {
    client.recover(&credential, &options, &staging).await
  };
  let failed = result.is_err();
  let reply = match result {
    Ok(r) => {
      let mut entries = BTreeMap::new();
      for name in r.entries().keys() {
        entries.insert(name.clone(), printable(&r.read(name)?));
      }
      json!({
        "cursor": r.cursor(),
        "complete": r.report().complete(),
        "report": r.report(),
        "entries": entries,
        "staging": listing(&staging)?,
      })
    }
    // The harness matches the reason, so an unrelated failure cannot pass as expected.
    Err(e) => json!({"error": e.to_string()}),
  };
  if staging.exists() && !(failed && named.is_some()) {
    std::fs::remove_dir_all(&staging)?;
  }
  Ok(reply)
}
fn limits(v: &Value) -> Limits {
  let d = if v["phone"].as_bool() == Some(true) {
    Limits::phone()
  } else {
    Limits::default()
  };
  let get = |k: &str, d: u64| v[k].as_u64().unwrap_or(d);
  Limits {
    commits: get("commits", d.commits),
    chunks: get("chunks", d.chunks),
    entries: get("entries", d.entries),
    name_bytes: get("name_bytes", d.name_bytes),
    download_bytes: get("download_bytes", d.download_bytes),
    staged_bytes: get("staged_bytes", d.staged_bytes),
  }
}
/// Relative path → octal mode of everything in the staging directory.
fn listing(root: &std::path::Path) -> Result<BTreeMap<String, String>> {
  use std::os::unix::fs::PermissionsExt;
  let mut out = BTreeMap::new();
  let mut stack = vec![root.to_path_buf()];
  while let Some(dir) = stack.pop() {
    for entry in std::fs::read_dir(&dir)? {
      let path = entry?.path();
      let mode = std::fs::metadata(&path)?.permissions().mode() & 0o777;
      let relative = path.strip_prefix(root)?.to_string_lossy().into_owned();
      if path.is_dir() {
        stack.push(path);
      }
      out.insert(relative, format!("{mode:o}"));
    }
  }
  out.insert(
    ".".into(),
    format!(
      "{:o}",
      std::fs::metadata(root)?.permissions().mode() & 0o777
    ),
  );
  Ok(out)
}

async fn run(vault: &mut Option<Vault>, cmd: &Value) -> Result<Value> {
  match cmd["cmd"].as_str().ok_or("cmd")? {
    "init" => {
      // Every vault starts on the real clock (a service clock is per vault).
      stitch_vault::set_clock_offset(0);
      let endpoint: String = cmd["endpoint"].as_str().ok_or("endpoint")?.into();
      let kit = Kit::generate()?;
      let signed = cmd["signed"].as_bool() == Some(true);
      // Signed: the kit's anchor signs the requests that post the first statements.
      let bootstrap = if signed {
        Client::signed(&endpoint, kit.vault(), kit.request_signer()?)?
      } else {
        Client::new(
          &endpoint,
          kit.vault(),
          cmd["admin"].as_str().ok_or("admin")?.into(),
        )?
      };
      let (genesis, _) = Membership::genesis(&kit)?;
      bootstrap.post_statement(&genesis).await?;
      let mut v = Vault {
        endpoint,
        signed,
        store: None,
        bootstrap,
        kit,
        admin: None,
        writers: BTreeMap::new(),
        devices: BTreeMap::new(),
      };
      let mut tokens = BTreeMap::new();
      for (list, role) in [
        ("admins", Role::Admin),
        ("writers", Role::Writer),
        ("readers", Role::Reader),
      ] {
        for name in cmd[list].as_array().into_iter().flatten() {
          let name = name.as_str().ok_or("name")?;
          tokens.insert(name.to_string(), v.enroll(name, role).await?);
        }
      }
      let reply = json!({"kit": v.kit.text().as_str(), "vault": v.vault(), "tokens": tokens, "devices": v.devices});
      *vault = Some(v);
      Ok(reply)
    }
    "put" => {
      let v = vault.as_mut().ok_or("init first")?;
      let vault_id = v.vault().to_string();
      let name = cmd["writer"].as_str().ok_or("writer")?;
      let w = v.writers.get_mut(name).ok_or("unknown writer")?;
      let (cursor, sequence, packs, bytes) = put(
        &vault_id,
        name,
        w,
        cmd["entries"].as_array().ok_or("entries")?,
      )
      .await?;
      Ok(json!({"cursor": cursor, "sequence": sequence, "packs": packs, "bytes": bytes}))
    }
    "concurrent" => {
      // Several writers at once (different devices): uploads and publications race.
      let v = vault.as_mut().ok_or("init first")?;
      let puts = cmd["puts"].as_array().ok_or("puts")?;
      let mut taken = vec![];
      for p in puts {
        let name = p["writer"].as_str().ok_or("writer")?.to_string();
        let w = v
          .writers
          .remove(&name)
          .ok_or("unknown or repeated writer")?;
        taken.push((name, w, p["entries"].as_array().ok_or("entries")?.clone()));
      }
      let vault_id = v.vault().to_string();
      let vault_id = vault_id.as_str();
      let results = futures_util::future::join_all(
        taken
          .iter_mut()
          .map(|(name, w, entries)| put(vault_id, name, w, entries)),
      )
      .await;
      for (name, w, _) in taken {
        v.writers.insert(name, w);
      }
      let mut cursors = vec![];
      for r in results {
        cursors.push(r?.0);
      }
      Ok(json!({"cursors": cursors}))
    }
    "many" => {
      let v = vault.as_mut().ok_or("init first")?;
      let vault_id = v.vault().to_string();
      let writer = cmd["writer"].as_str().ok_or("writer")?;
      let prefix = cmd["prefix"].as_str().unwrap_or("n");
      let w = v.writers.get_mut(writer).ok_or("unknown writer")?;
      let mut last = (0, 0, 0, 0);
      for i in 0..cmd["count"].as_u64().ok_or("count")? {
        let entry = json!([format!("{prefix}{i}"), format!("{prefix} value {i}")]);
        last = put(&vault_id, writer, w, &[entry]).await?;
      }
      Ok(json!({"cursor": last.0, "sequence": last.1}))
    }
    "revoke" => {
      // The admin device revokes; every revocation rotates (default policy).
      let v = vault.as_ref().ok_or("init first")?;
      let name = cmd["writer"].as_str().ok_or("writer")?;
      let w = v.writers.get(name).ok_or("unknown writer")?;
      let through = cmd["through"].as_u64().unwrap_or(w.device.head().0);
      // `by: "kit"`: the kit signs (a retroactive revocation, below the device's head,
      // needs the kit or two admins).
      if cmd["by"].as_str() == Some("kit") {
        v.bootstrap
          .revoke(
            &Signer::Kit(&v.kit),
            None,
            v.vault(),
            &w.device.public(),
            through,
          )
          .await?;
        return Ok(json!({"through": through}));
      }
      let a = &v.writers[v.admin.as_ref().ok_or("no admin")?];
      a.client
        .revoke(
          &Signer::Device(&a.device),
          None,
          v.vault(),
          &w.device.public(),
          through,
        )
        .await?;
      Ok(json!({"through": through}))
    }
    "checkpoint" => {
      let v = vault.as_ref().ok_or("init first")?;
      let name = match cmd["as"].as_str() {
        Some(n) => n.to_string(),
        None => v.admin.clone().ok_or("no admin")?,
      };
      let a = v.writers.get(&name).ok_or("unknown device")?;
      let credential = Credential::Device {
        device: &a.device,
        vault: v.vault(),
      };
      let none = Default::default();
      let out = a
        .client
        .checkpoint(
          &credential,
          &a.device,
          cmd["repack_below"].as_f64().unwrap_or(0.5),
          &none,
        )
        .await?;
      Ok(serde_json::to_value(out)?)
    }
    "gc" => {
      let v = vault.as_ref().ok_or("init first")?;
      let a = &v.writers[v.admin.as_ref().ok_or("no admin")?];
      Ok(a.client.gc().await?)
    }
    "op" => {
      // One membership statement, signed by `by` (a device name or "kit"), optionally
      // co-signed by `cosign`, posted with the signer's own credential (the kit's via the
      // bootstrap/anchor client).
      let v = vault.as_ref().ok_or("init first")?;
      let signer_of = |name: &str| -> Result<Signer<'_>> {
        Ok(if name == "kit" {
          Signer::Kit(&v.kit)
        } else {
          Signer::Device(&v.writers.get(name).ok_or("unknown device")?.device)
        })
      };
      let by = cmd["by"].as_str().ok_or("by")?;
      let signer = signer_of(by)?;
      let cosigner = cmd["cosign"].as_str().map(signer_of).transpose()?;
      let poster = if by == "kit" {
        &v.bootstrap
      } else {
        &v.writers[by].client
      };
      let target = |k: &str| -> Result<String> {
        Ok(
          v.devices
            .get(cmd[k].as_str().ok_or("target")?)
            .ok_or("unknown target")?
            .clone(),
        )
      };
      // Revocations and freezes default to the target's own head (not retroactive).
      let through = cmd["through"].as_u64().unwrap_or_else(|| {
        cmd["target"]
          .as_str()
          .and_then(|t| v.writers.get(t))
          .map_or(0, |w| w.device.head().0)
      });
      let s = match cmd["op"].as_str().ok_or("op")? {
        "delete" => {
          poster
            .propose(v.vault(), |m| m.delete(&signer, cosigner.as_ref()))
            .await?
        }
        "settings" => {
          let days = cmd["days"].as_u64().unwrap_or(30);
          let two = cmd["two"].as_bool().unwrap_or(false);
          poster
            .propose(v.vault(), |m| {
              m.settings(&signer, cosigner.as_ref(), days, two)
            })
            .await?
        }
        "revoke" => {
          let d = target("target")?;
          poster
            .propose(v.vault(), |m| {
              m.revoke(&signer, cosigner.as_ref(), &d, through)
            })
            .await?
        }
        "cancel" => {
          let seq = cmd["seq"].as_u64().ok_or("seq")?;
          poster
            .propose(v.vault(), |m| m.cancel(&signer, seq))
            .await?
        }
        "freeze" => {
          let d = target("target")?;
          poster
            .propose(v.vault(), |m| m.freeze(&signer, &d, through))
            .await?
        }
        "unfreeze" => {
          let d = target("target")?;
          let credential = match &signer {
            Signer::Kit(k) => Credential::Kit(k),
            Signer::Device(dev) => Credential::Device {
              device: dev,
              vault: v.vault(),
            },
          };
          let (ring, _, _) = poster.keyring(&credential).await?;
          poster
            .propose(v.vault(), |m| m.unfreeze(&signer, &ring, &d))
            .await?
        }
        other => return Err(format!("unknown op {other}").into()),
      };
      Ok(json!({"seq": s.seq, "at": s.at}))
    }
    "flood" => {
      // A stolen admin `by` posts `n` cheap immediate statements (same retention), one
      // after another from one fetched state; stops at the first refusal.
      let v = vault.as_ref().ok_or("init first")?;
      let w = v
        .writers
        .get(cmd["by"].as_str().ok_or("by")?)
        .ok_or("unknown device")?;
      let mut state = v.membership().await?;
      let signer = Signer::Device(&w.device);
      let mut posted = 0u64;
      let mut refused = Value::Null;
      for _ in 0..cmd["n"].as_u64().ok_or("n")? {
        let s = state.settings(&signer, None, state.retention_days, state.approvals >= 2)?;
        if let Err(e) = w.client.post_statement(&s).await {
          refused = json!(e.to_string());
          break;
        }
        state.apply(&s)?;
        posted += 1;
      }
      Ok(json!({"posted": posted, "refused": refused}))
    }
    "signed_header" => {
      // One request's x-vault-auth as device `as` signs it (replay tests).
      let v = vault.as_ref().ok_or("init first")?;
      let w = v
        .writers
        .get(cmd["as"].as_str().ok_or("as")?)
        .ok_or("unknown device")?;
      let header = w.device.request_signer()?.header(
        v.vault(),
        cmd["method"].as_str().unwrap_or("GET"),
        cmd["path"].as_str().ok_or("path")?,
        cmd["digest"].as_str().unwrap_or(""),
      );
      Ok(json!({"header": header}))
    }
    "add" => {
      // Admit a new device signed by `by` (a device name or "kit"), optionally co-signed.
      let v = vault.as_mut().ok_or("init first")?;
      let role = match cmd["role"].as_str().unwrap_or("writer") {
        "admin" => Role::Admin,
        "reader" => Role::Reader,
        _ => Role::Writer,
      };
      let by = cmd["by"].as_str().filter(|b| *b != "kit");
      let name = cmd["name"].as_str().ok_or("name")?;
      let token = v.enroll_by(name, role, by, cmd["cosign"].as_str()).await?;
      let seq = v.membership().await?.seq;
      Ok(json!({"token": token, "seq": seq, "device": v.devices[name]}))
    }
    "pending" => {
      // The pending actions as a device verifies them, and as the service reports them.
      let v = vault.as_ref().ok_or("init first")?;
      let reader = v
        .writers
        .get(cmd["as"].as_str().unwrap_or("adm"))
        .ok_or("unknown device")?;
      let (pending, deleted, retention) = reader.client.pending(v.vault()).await?;
      Ok(json!({"pending": pending, "deleted": deleted, "retention_days": retention}))
    }
    "settings" => {
      let v = vault.as_ref().ok_or("init first")?;
      let a = &v.writers[v.admin.as_ref().ok_or("no admin")?];
      a.client
        .settings(
          &a.device,
          v.vault(),
          cmd["days"].as_u64().unwrap_or(30),
          cmd["two"].as_bool().unwrap_or(false),
        )
        .await?;
      Ok(json!({"ok": true}))
    }
    "clock" => {
      // Move the service clock (TEST_CLOCK only), as the bootstrap token.
      let v = vault.as_ref().ok_or("init first")?;
      let offset = cmd["offset_ms"].as_i64().ok_or("offset_ms")?;
      v.bootstrap.set_clock(offset).await?;
      // Clients (statement times, pending actions, recovery) move with the service.
      stitch_vault::set_clock_offset(offset);
      Ok(json!({"ok": true}))
    }
    "forget" => {
      // Drop a writer's known-chunk cache (after the service refused a reuse).
      let v = vault.as_mut().ok_or("init first")?;
      v.writers
        .get_mut(cmd["writer"].as_str().ok_or("writer")?)
        .ok_or("unknown writer")?
        .known
        .clear();
      Ok(json!({"ok": true}))
    }
    "store" => {
      // A small real store: a thread with its original image (and a transcript), a
      // schedule and an assignment that refer to it, and a record whose blob is absent.
      use stitch_records::{
        Attachment, Author, BlobHash, Draft, Event, Msg, NodeName, Part, Ref, Role as Who, kinds,
      };
      let v = vault.as_mut().ok_or("init first")?;
      let dir =
        std::env::temp_dir().join(format!("stitch-vault-probe-store-{}", std::process::id()));
      let _ = std::fs::remove_dir_all(&dir);
      let root = stitch_store::RecordRoot::init(
        &dir.join("root"),
        "probe-repo".parse()?,
        stitch_store::Role::Home,
      )?;
      let blobs = stitch_store::BlobStore::open(&dir.join("blobs"))?;
      let node: NodeName = "devicea".parse()?;
      let png = image();
      let hash = blobs.put_bytes(&png)?;
      let attach = |hash: BlobHash, name: &str, bytes: u64| Attachment {
        hash,
        media_type: "image/png".into(),
        name: Some(name.into()),
        bytes,
      };
      let thread = root.publish(
        &node,
        Draft::new(kinds::THREAD, Author::user(node.clone()))?
          .attach(attach(hash, "original.png", png.len() as u64))
          .body("# A thread with an original image\n"),
      )?;
      root.publish_segment(
        &node,
        &thread.id,
        None,
        None,
        vec![Event::Msg(Msg {
          id: "m1".into(),
          at: jiff::Timestamp::now(),
          role: Who::User,
          from: None,
          model: None,
          parts: vec![Part::Text {
            text: "hello vault".into(),
          }],
          native: None,
        })],
      )?;
      let schedule = root.publish(
        &node,
        Draft::new(kinds::SCHEDULE, Author::user(node.clone()))?
          .reference("thread", Ref::to(thread.id.clone()))?
          .body("every day at 9\n"),
      )?;
      let assignment = root.publish(
        &node,
        Draft::new(kinds::ASSIGNMENT, Author::user(node.clone()))?
          .reference("thread", Ref::to(thread.id.clone()))?
          .body("run it\n"),
      )?;
      let missing = root.publish(
        &node,
        Draft::new(kinds::THREAD, Author::user(node.clone()))?
          .attach(attach(
            BlobHash::of(b"never stored here"),
            "elsewhere.png",
            17,
          ))
          .body("# Its original is on another device\n"),
      )?;
      let ids = BTreeMap::from([
        ("thread".to_string(), thread.id.to_string()),
        ("schedule".to_string(), schedule.id.to_string()),
        ("assignment".to_string(), assignment.id.to_string()),
        ("missing".to_string(), missing.id.to_string()),
      ]);
      let reply = json!({"ids": ids, "blob": hash.hex()});
      v.store = Some(Store {
        dir,
        root,
        blobs,
        synced: BTreeMap::new(),
        ids,
      });
      Ok(reply)
    }
    "backup" => {
      let v = vault.as_mut().ok_or("init first")?;
      let vault_id = v.vault().to_string();
      let w = v
        .writers
        .get_mut(cmd["writer"].as_str().ok_or("writer")?)
        .ok_or("unknown writer")?;
      let store = v.store.as_mut().ok_or("store first")?;
      if let Some(name) = cmd["purge"].as_str() {
        // Remove a record file, as a purge would.
        let id: stitch_records::RecordId = store.ids[name].parse()?;
        std::fs::remove_file(store.root.path_of(&stitch_store::ItemPath::Record(id)))?;
      }
      let (ring, _, _) = w
        .client
        .keyring(&Credential::Device {
          device: &w.device,
          vault: &vault_id,
        })
        .await?;
      let report = stitch_vault::node::backup(
        &w.client,
        ring.current()?,
        &mut w.device,
        &mut w.known,
        &store.root,
        &store.blobs,
        &mut store.synced,
      )
      .await?;
      Ok(json!({"report": report, "complete": report.complete()}))
    }
    "preview" => {
      // Recover with the kit into staging, then build an inert preview from it.
      let v = vault.as_ref().ok_or("init first")?;
      let client = Client::new(
        cmd["endpoint"].as_str().ok_or("endpoint")?,
        v.vault(),
        cmd["token"].as_str().ok_or("token")?.into(),
      )?;
      let tmp =
        std::env::temp_dir().join(format!("stitch-vault-probe-preview-{}", std::process::id()));
      let _ = std::fs::remove_dir_all(&tmp);
      std::fs::create_dir_all(&tmp)?;
      let recovered = client
        .recover(
          &Credential::Kit(&v.kit),
          &RecoverOptions::default(),
          &tmp.join("staging"),
        )
        .await?;
      let dest = tmp.join("preview");
      let preview = stitch_vault::node::preview(&recovered, &dest, "preview")?;
      // What a node would see: every file, and whether any old ID survived anywhere.
      let mut files = vec![];
      let mut old_ids_left = vec![];
      let olds: Vec<&String> = preview.ids.keys().collect();
      let mut stack = vec![dest.clone()];
      while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)? {
          let path = e?.path();
          if path.is_dir() {
            stack.push(path);
            continue;
          }
          let rel = path.strip_prefix(&dest)?.to_string_lossy().into_owned();
          let text = String::from_utf8_lossy(&std::fs::read(&path)?).into_owned();
          for old in &olds {
            if text.contains(old.as_str()) || rel.contains(old.as_str()) {
              old_ids_left.push(rel.clone());
            }
          }
          files.push(rel);
        }
      }
      files.sort();
      let kinds: Vec<String> = files
        .iter()
        .filter(|f| f.ends_with(".md"))
        .filter_map(|f| {
          stitch_records::Record::parse(&std::fs::read_to_string(dest.join(f)).ok()?).ok()
        })
        .map(|r| r.kind.to_string())
        .collect();
      std::fs::remove_dir_all(&tmp)?;
      Ok(
        json!({"preview": preview, "files": files, "kinds": kinds, "old_ids_left": old_ids_left, "report": recovered.report()}),
      )
    }
    "epochs" => {
      // Which epochs a credential can open: a device (`as`) or the kit.
      let v = vault.as_ref().ok_or("init first")?;
      let log = v.bootstrap_reader().statements().await?;
      let epochs = match cmd["as"].as_str() {
        Some(name) => keyring(
          &log,
          &Credential::Device {
            device: &v.writers.get(name).ok_or("unknown device")?.device,
            vault: v.vault(),
          },
        )?
        .0
        .epochs(),
        None => keyring(&log, &Credential::Kit(&v.kit))?.0.epochs(),
      };
      Ok(json!({"epochs": epochs}))
    }
    "recover" => recover(cmd, vault.as_ref()).await,
    other => Err(format!("unknown command {other}").into()),
  }
}

#[tokio::main]
async fn main() -> Result<()> {
  rustls::crypto::ring::default_provider()
    .install_default()
    .ok();
  let mut vault = None;
  let stdin = io::stdin();
  for line in stdin.lock().lines() {
    let line = line?;
    if line.trim().is_empty() {
      continue;
    }
    let cmd: Value = serde_json::from_str(&line)?;
    let reply = match run(&mut vault, &cmd).await {
      Ok(v) => v,
      Err(e) => json!({"failed": e.to_string()}),
    };
    let mut out = io::stdout().lock();
    writeln!(out, "{reply}")?;
    out.flush()?;
  }
  Ok(())
}
