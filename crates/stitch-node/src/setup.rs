//! Setting a node up and joining the user's other nodes (design §2.11):
//! `stitch setup` and `stitch join`, and the app's first start and pairing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use stitch_net::{
  BlobPolicy, Discovery, Net, NodeState, Peer, Peers, Policy, SessionReport, Ticket, bind,
  load_or_create_key,
};
use stitch_records::NodeName;
use stitch_store::{BlobStore, Index, RecordRoot, Role};

use crate::config::Config;
use crate::{Error, Result};

/// Where a node keeps its home workspace unless told otherwise.
pub fn default_home(data: &Path) -> PathBuf {
  data.join("home")
}

fn open_index(data: &Path) -> Result<Index> {
  std::fs::create_dir_all(data).map_err(Error::io(data.display()))?;
  Ok(Index::open(&data.join("index.sqlite"))?)
}

/// A node set up: its endpoint's public key, and its home workspace.
pub struct SetUp {
  pub key: String,
  pub home: Option<RecordRoot>,
}

/// Names the node (`config.toml`), makes its key, and makes or opens its
/// home workspace at `home` (none: no home workspace). `local`: discovery
/// on the local network only. `lazy`: blobs fetched when needed (phones).
pub fn setup(
  data: &Path,
  name: &NodeName,
  home: Option<&Path>,
  local: bool,
  lazy: bool,
) -> Result<SetUp> {
  let mut c = Config::load(data)?;
  c.node = Some(name.to_string());
  if local {
    c.discovery = Some("local".into());
  }
  if lazy {
    c.blobs = Some("lazy".into());
  }
  c.save(data)?;
  let key = load_or_create_key(&data.join("node.key"))?;
  let home = match home {
    Some(dir) => {
      let root = match RecordRoot::open(dir) {
        Ok(r) => r,
        Err(_) => RecordRoot::init(dir, "home".parse()?, Role::Home)?,
      };
      open_index(data)?.refresh(&root)?;
      Some(root)
    }
    None => None,
  };
  Ok(SetUp {
    key: key.public().to_string(),
    home,
  })
}

/// What joining brought.
pub struct Joined {
  pub inviter: Peer,
  /// The other nodes the inviter trusts, trusted here now.
  pub fleet: Vec<Peer>,
  /// The home workspace, synced once with the inviter; `None` when the
  /// inviter holds none.
  pub home: Option<SessionReport>,
}

/// Joins the nodes of an invitation (`stitch invite` on one of them):
/// trusts them, and syncs their home workspace into `home`. The node of
/// `data` must not run meanwhile: its key binds the endpoint here.
pub async fn join(data: &Path, name: &NodeName, ticket: &Ticket, home: &Path) -> Result<Joined> {
  let discovery = match Config::load(data)?.discovery.as_deref() {
    Some("local") => Discovery::Local,
    _ => Discovery::N0,
  };
  let lazy = Policy {
    blobs: BlobPolicy::Lazy,
  };
  let key = load_or_create_key(&data.join("node.key"))?;
  let peers_path = data.join("peers.toml");
  let state = |roots| -> Result<NodeState> {
    Ok(NodeState::new(
      name.clone(),
      roots,
      open_index(data)?,
      BlobStore::open(&data.join("blobs"))?,
      Peers::load(&peers_path)?,
      Some(peers_path.clone()),
      lazy,
    ))
  };
  let endpoint = bind(key.clone(), discovery, None).await?;
  let net = Net::spawn(endpoint, Arc::new(state(Default::default())?));
  let joined = net.join(ticket).await;
  net.shutdown().await?;
  let joined = joined?;
  let report = match &joined.home {
    Some(repo) => {
      let root = match RecordRoot::open(home) {
        Ok(r) if r.repo().as_str() == repo => r,
        Ok(r) => {
          return Err(Error::Config(format!(
            "{} holds repository {}, not {repo}",
            home.display(),
            r.repo()
          )));
        }
        Err(_) => RecordRoot::init(home, repo.parse()?, Role::Home)?,
      };
      open_index(data)?.refresh(&root)?;
      let roots = [(root.repo().clone(), root.clone())].into_iter().collect();
      let endpoint = bind(key, discovery, None).await?;
      let net = Net::spawn(endpoint, Arc::new(state(roots)?));
      let report = net.sync_with(joined.inviter.addr()?, root.repo()).await;
      net.shutdown().await?;
      Some(report?)
    }
    None => None,
  };
  Ok(Joined {
    inviter: joined.inviter,
    fleet: joined.fleet,
    home: report,
  })
}
