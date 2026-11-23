//! Test driver for the hosted vault service's live end-to-end test: what the `stitch vault` CLI does not do,
//! done with a throwaway node's own vault device (`<data>/vault/`). Never use it with
//! real keys or a real node's data directory. The kit, when needed, is read from
//! standard input, never from the command line.
//!
//! - `header DATA METHOD PATH [DIGEST]`: one request's `x-vault-auth`, signed by the
//!   node's device (PATH relative to the vault, e.g. `/members`).
//! - `revoke DATA DEVICE [THROUGH]`: the node (an admin) revokes DEVICE as of its
//!   commit THROUGH (0 by default: a reader, or a writer that never published).
//! - `delete DATA` (kit on stdin): the two-admin setting on, then the vault's deletion
//!   signed by the node and co-signed by the kit, so it applies at once.
use serde_json::{Value, json};
use std::{io::Read, path::Path};
use stitch_vault::{Client, Device, Kit, Signer};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn open(data: &str) -> Result<(String, String, Device)> {
  let dir = Path::new(data).join("vault");
  let config: Value = serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)?;
  let device = Device::from_secret_text(&std::fs::read_to_string(dir.join("device.key"))?)?;
  let s = |k: &str| config[k].as_str().map(String::from).ok_or("config.json");
  Ok((s("endpoint")?, s("vault")?, device))
}

#[tokio::main]
async fn main() -> Result<()> {
  let args: Vec<String> = std::env::args().skip(1).collect();
  let arg = |i: usize| args.get(i).map(String::as_str).ok_or("missing argument");
  let (endpoint, vault, device) = open(arg(1)?)?;
  let client = Client::signed(&endpoint, &vault, device.request_signer()?)?;
  let signer = Signer::Device(&device);
  let out = match arg(0)? {
    "header" => {
      let digest = args.get(4).map_or("", String::as_str);
      let header = device
        .request_signer()?
        .header(&vault, arg(2)?, arg(3)?, digest);
      json!({ "header": header })
    }
    "revoke" => {
      let target = arg(2)?.to_string();
      let through = args.get(3).map_or(Ok(0), |t| t.parse())?;
      let s = client
        .propose(&vault, |m| m.revoke(&signer, None, &target, through))
        .await?;
      json!({ "seq": s.seq })
    }
    "delete" => {
      let mut text = String::new();
      std::io::stdin().read_to_string(&mut text)?;
      let kit = Kit::parse(text.trim())?;
      if kit.vault() != vault {
        return Err("the kit is another vault's".into());
      }
      let state = client.propose(&vault, |m| {
        m.settings(&signer, None, m.retention_days, true)
      });
      let settings = state.await?;
      let kit_signer = Signer::Kit(&kit);
      let delete = client
        .propose(&vault, |m| m.delete(&signer, Some(&kit_signer)))
        .await?;
      json!({ "settings": settings.seq, "delete": delete.seq })
    }
    other => return Err(format!("unknown command {other}").into()),
  };
  println!("{out}");
  Ok(())
}
