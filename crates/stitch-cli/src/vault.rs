//! `stitch vault`: the node's opt-in encrypted vault (docs/vault-protocol.md). The protocol and
//! the node-side state are `stitch_vault::local`; this is the command line.
use std::collections::BTreeSet;
use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use std::time::Duration;

use clap::Subcommand;
use stitch_store::{BlobStore, Index, RecordRoot};
use stitch_vault::local::{self, Local};
use stitch_vault::{Kit, Role};

use crate::{Cli, Result, all_roots, data_dir, home_root, node_name, open_index, runtime};

#[derive(Subcommand)]
pub enum VaultCommand {
  /// Create a vault at a vault service and make this node its first admin. Prints
  /// the recovery kit once: keep it offline (it is the vault's root of trust).
  Create {
    #[arg(long)]
    endpoint: String,
  },
  /// Join an existing vault. With --kit (read from standard input or a prompt), the
  /// kit admits this node at once; otherwise this prints the keys an admin passes to
  /// `stitch vault admit`.
  Join {
    #[arg(long)]
    endpoint: String,
    #[arg(long)]
    vault: String,
    #[arg(long)]
    kit: bool,
    /// admin, writer or reader (with --kit; default writer).
    #[arg(long, default_value = "writer")]
    role: String,
  },
  /// Admit another device (this node must be an admin). An admin admitted this way
  /// is a writer for 7 days first.
  Admit {
    device: String,
    seal: String,
    #[arg(long, default_value = "writer")]
    role: String,
  },
  /// This node's standing in the vault, pending destructive actions, the last backup.
  Status {
    #[arg(long)]
    json: bool,
  },
  /// Back up every record root and the original blobs they attach (incremental).
  Backup {
    /// Keep running: back up the repositories the change feed reports, checking
    /// every this many seconds.
    #[arg(long)]
    watch: Option<u64>,
  },
  /// Recover into a new staging directory and write an inert preview (nothing in it
  /// runs; import chosen conversations with `stitch vault import`).
  Recover {
    /// Where the inert preview goes (must not exist).
    #[arg(long)]
    preview: PathBuf,
    /// The staging directory (default: <preview>.staging); with --resume, the one a
    /// stopped recovery left.
    #[arg(long)]
    staging: Option<PathBuf>,
    /// The state as of this cursor (default: the head).
    #[arg(long)]
    through: Option<u64>,
    #[arg(long)]
    resume: bool,
    /// Phone-sized limits (per-run download bound; resume to continue).
    #[arg(long)]
    phone: bool,
    /// Recover with the recovery kit (no device of the vault on this node).
    #[arg(long)]
    kit: bool,
    /// With --kit: the vault service.
    #[arg(long)]
    endpoint: Option<String>,
  },
  /// Import records (and a thread's transcript) from a preview into a record root.
  Import {
    preview: PathBuf,
    /// Record IDs as the preview names them (repeatable).
    #[arg(long = "id", required = true)]
    ids: Vec<String>,
    /// The record root to import into (default: the home workspace).
    #[arg(long)]
    root: Option<PathBuf>,
  },
  /// Stop using the vault on this node (its directory, device key included, is moved
  /// aside; nothing changes in the vault).
  Disable,
}

fn role(s: &str) -> Result<Role> {
  Ok(match s {
    "admin" => Role::Admin,
    "writer" => Role::Writer,
    "reader" => Role::Reader,
    _ => return Err(format!("unknown role {s} (admin, writer or reader)").into()),
  })
}
fn read_kit() -> Result<Kit> {
  let text = if std::io::stdin().is_terminal() {
    rpassword::prompt_password("Recovery kit: ")?
  } else {
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    line
  };
  Ok(Kit::parse(text.trim())?)
}
fn opened(cli: &Cli) -> Result<Local> {
  Local::open(&data_dir(cli)?)?
    .ok_or_else(|| "no vault on this node (stitch vault create or join)".into())
}

pub fn run(cli: &Cli, action: &VaultCommand) -> Result<()> {
  let data = data_dir(cli)?;
  let rt = runtime()?;
  match action {
    VaultCommand::Create { endpoint } => {
      let node = node_name(cli)?.to_string();
      let (local, kit) = rt.block_on(Local::create(&data, &node, endpoint))?;
      println!("vault {} at {}", local.config.vault, local.config.endpoint);
      println!(
        "this node is its first admin (device {})",
        local.config.device
      );
      println!();
      println!("Recovery kit (shown once; keep it offline, it can recover and control the vault):");
      println!("{}", kit.text().as_str());
    }
    VaultCommand::Join {
      endpoint,
      vault,
      kit,
      role: r,
    } => {
      let node = node_name(cli)?.to_string();
      let kit = if *kit { Some(read_kit()?) } else { None };
      let local = rt.block_on(Local::join(
        &data,
        &node,
        endpoint,
        vault,
        kit.as_ref(),
        role(r)?,
      ))?;
      if kit.is_some() {
        println!("joined vault {} as {r}", local.config.vault);
      } else {
        println!(
          "set up for vault {}; ask an admin to run:",
          local.config.vault
        );
        println!(
          "  stitch vault admit {} {} --role {r}",
          local.config.device, local.config.seal
        );
      }
    }
    VaultCommand::Admit {
      device,
      seal,
      role: r,
    } => {
      let local = opened(cli)?;
      let s = rt.block_on(local.admit(device, seal, role(r)?))?;
      println!("admitted {device} as {r} (statement {})", s.seq);
      if r == "admin" {
        println!("as a writer for now: admin after 7 days unless cancelled (any admin or the kit)");
      }
    }
    VaultCommand::Status { json } => {
      let local = opened(cli)?;
      let s = rt.block_on(local.status())?;
      if *json {
        println!("{}", serde_json::to_string_pretty(&s)?);
        return Ok(());
      }
      println!("vault {} at {}", s.config.vault, s.config.endpoint);
      let standing = match (&s.role, s.frozen, s.revoked) {
        (None, _, _) => "not admitted yet".to_string(),
        (_, _, true) => "revoked".into(),
        (Some(r), true, _) => format!("{r}, frozen"),
        (Some(r), false, _) => r.clone(),
      };
      println!("this node: {standing} (device {})", s.config.device);
      println!(
        "epoch {}, retention {} days, two-admin approval {}",
        s.epoch,
        s.retention_days,
        if s.two_admins { "on" } else { "off" }
      );
      if s.deleted {
        println!("THE VAULT IS DELETED");
      }
      for p in &s.pending {
        println!(
          "PENDING: {:?} (statement {}) by {} takes effect at {} ms unless cancelled",
          p.action, p.seq, p.signer, p.effective
        );
      }
      if let Some(r) = &s.refused {
        println!("WARNING: the membership log has a refused statement: {r}");
      }
      if !s.kit_envelope_problems.is_empty() {
        println!(
          "WARNING: the kit's envelopes in statements {:?} do not check out: rotate keys",
          s.kit_envelope_problems
        );
      }
      match &s.last_backup {
        Some(b) => println!(
          "last backup: {} repositories ({} unchanged), {} files up, {} deleted, {} blobs, {}",
          b.repos,
          b.unchanged_repos,
          b.uploaded,
          b.deleted,
          b.blobs,
          if b.complete() {
            "complete".to_string()
          } else {
            format!(
              "incomplete: {} originals missing here",
              b.missing_blobs.len()
            )
          }
        ),
        None => println!("no backup yet"),
      }
      println!("{} files in {} repositories backed up", s.files, s.repos);
      if s.unpublished {
        println!("a commit waits to be published again (next backup)");
      }
    }
    VaultCommand::Backup { watch } => {
      let mut local = opened(cli)?;
      let blobs = BlobStore::open(&data.join("blobs"))?;
      let mut index = open_index(cli)?;
      let roots = all_roots(&mut index)?;
      let s = rt.block_on(local.backup(&mut index, &roots, &blobs))?;
      print_summary(&s);
      let Some(secs) = watch else { return Ok(()) };
      // The change feed: refreshing the roots (this process) and polling (other
      // processes, the node) tell which repositories changed.
      let changed = std::sync::Arc::new(std::sync::Mutex::new(BTreeSet::new()));
      let feed = changed.clone();
      index.changes().subscribe(move |c| {
        if let Ok(mut set) = feed.lock() {
          set.insert(c.repo.to_string());
        }
      });
      let _ = index.poll_changes();
      loop {
        std::thread::sleep(Duration::from_secs((*secs).max(5)));
        let roots = all_roots(&mut index)?;
        let _ = index.poll_changes();
        let repos: BTreeSet<String> =
          std::mem::take(&mut *changed.lock().map_err(|_| "feed lock")?);
        if repos.is_empty() {
          continue;
        }
        let chosen: Vec<RecordRoot> = roots
          .into_iter()
          .filter(|r| repos.contains(&r.repo().to_string()))
          .collect();
        match rt.block_on(local.backup(&mut index, &chosen, &blobs)) {
          Ok(s) => print_summary(&s),
          Err(e) => eprintln!("stitch vault: backup failed (retried at the next change): {e}"),
        }
      }
    }
    VaultCommand::Recover {
      preview,
      staging,
      through,
      resume,
      phone,
      kit,
      endpoint,
    } => {
      let staging = staging.clone().unwrap_or_else(|| {
        let mut s = preview.clone().into_os_string();
        s.push(".staging");
        PathBuf::from(s)
      });
      let node = node_name(cli)?.to_string();
      let (report, p) = if *kit {
        let endpoint = endpoint.as_deref().ok_or("--kit needs --endpoint")?;
        let kit = read_kit()?;
        rt.block_on(local::recover_with_kit(
          endpoint, &kit, &node, &staging, preview, *through, *resume, *phone,
        ))?
      } else {
        let local = opened(cli)?;
        rt.block_on(local.recover(&staging, preview, *through, *resume, *phone))?
      };
      println!(
        "recovered through cursor {} ({}): {} records, {} segments, {} blobs into {}",
        report.cursor,
        if report.complete() {
          "complete"
        } else {
          "with problems"
        },
        p.records,
        p.segments,
        p.blobs,
        preview.display()
      );
      for i in report
        .rejected
        .iter()
        .chain(&report.incomplete)
        .chain(&report.missing)
      {
        println!("  {} {} {}: {}", i.cursor, i.device, i.name, i.reason);
      }
      println!(
        "inert: nothing in it runs. Import with: stitch vault import {} --id <record>",
        preview.display()
      );
    }
    VaultCommand::Import { preview, ids, root } => {
      let mut index: Index = open_index(cli)?;
      let root = match root {
        Some(r) => RecordRoot::open(r)?,
        None => home_root(&index)?.ok_or("no home workspace: pass --root")?,
      };
      let blobs = BlobStore::open(&data.join("blobs"))?;
      let out = local::import(preview, ids, &root, &blobs)?;
      index.refresh(&root)?;
      println!(
        "imported {} records, {} segments, {} blobs ({} already present) into {}",
        out.records,
        out.segments,
        out.blobs,
        out.present,
        root.dir().display()
      );
      for id in &out.not_found {
        println!("  not in the preview: {id}");
      }
    }
    VaultCommand::Disable => {
      let aside = Local::disable(&data)?;
      println!(
        "vault disabled on this node; its directory is at {}",
        aside.display()
      );
    }
  }
  Ok(())
}

fn print_summary(s: &local::Summary) {
  println!(
    "backup: {} repositories ({} unchanged), {} files up, {} deleted, {} blobs, {} commits{}{}",
    s.repos,
    s.unchanged_repos,
    s.uploaded,
    s.deleted,
    s.blobs,
    s.commits,
    if s.secrets_skipped > 0 {
      format!(", {} secret records left out", s.secrets_skipped)
    } else {
      String::new()
    },
    if s.complete() {
      String::new()
    } else {
      format!(
        "; incomplete: {} originals not on this node",
        s.missing_blobs.len()
      )
    }
  );
}
