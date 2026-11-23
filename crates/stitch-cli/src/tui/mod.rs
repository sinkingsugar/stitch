//! The TUI (`stitch` with no command): threads, the conversation and an
//! input line. Chats go through the node, started if needed; the thread
//! list, transcripts and the attention view are read from the store.

mod app;
mod clipboard;
#[cfg(test)]
mod emu;
mod files;
mod markdown;
mod tree;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{MoveTo, Show};
use ratatui::crossterm::event::{
  DisableBracketedPaste, EnableBracketedPaste, Event, KeyboardEnhancementFlags,
  PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::style::Stylize;
use ratatui::crossterm::terminal::{self, Clear, ClearType};
use ratatui::{TerminalOptions, Viewport};
use std::io::Write as _;
use stitch_node::api::{Reply, Request};
use stitch_node::chat::ChatContext;
use stitch_node::client::Client;
use stitch_node::place::{self, Place};
use stitch_records::{NodeName, RecordId, RepoId};
use stitch_store::{AttentionItem, BlobStore, Index, RecordRoot, Role};
use tokio::sync::mpsc;

use app::{
  Action, App, AuthAction, Choice, Entry, Family, Fetch, Needs, Printed, ThreadItem, ThreadScope,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// What the TUI reads directly.
pub struct Store {
  pub data: PathBuf,
  pub node: NodeName,
  /// Where the TUI runs.
  pub here: Place,
  pub here_label: String,
  pub index: Mutex<Index>,
  pub blobs: BlobStore,
}

impl Store {
  /// Every repository held here (a project found meanwhile too).
  fn roots(&self, index: &Index) -> Vec<RecordRoot> {
    let dirs = index.roots().unwrap_or_default();
    dirs
      .into_values()
      .filter_map(|dir| RecordRoot::open(std::path::Path::new(&dir)).ok())
      .collect()
  }

  /// Indexes what the roots gained or lost that the node did not index
  /// itself (it indexes what it writes and syncs): whether anything did.
  /// It walks every root, so the poller does it now and then, not on each
  /// read (decided 2026-10-22, see design §TUI).
  fn scan(&self) -> bool {
    let mut index = self.index.lock().expect("index lock");
    let mut changed = false;
    for r in self.roots(&index) {
      if let Ok(report) = index.refresh(&r) {
        changed |= report.records + report.segments + report.removed > 0
          || !report.newer.is_empty()
          || !report.quarantined.is_empty();
      }
    }
    changed
  }

  /// Joins or parts the folder the TUI runs in (`stitch folder`): what to
  /// tell the user (`/link --force <node:path>` links a location Stitch
  /// does not know).
  fn link(&mut self, other: Option<&str>) -> std::result::Result<String, String> {
    if !matches!(self.here, Place::Folder(_)) {
      return Err(
        "not in a folder: a git checkout is a project, the home directory is global".into(),
      );
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let loc = place::location_of(self.node.as_str(), &cwd);
    let mut index = self.index.lock().expect("index lock");
    let roots = self.roots(&index);
    for r in &roots {
      let _ = index.refresh(r);
    }
    let draft = match other {
      Some(o) => {
        let (force, o) = match o.trim().strip_prefix("--force") {
          Some(rest) if rest.is_empty() || rest.starts_with(' ') => (true, rest.trim()),
          _ => (false, o.trim()),
        };
        if o.is_empty() {
          return Err("/link [--force] <node:path>".into());
        }
        match place::link(&index, &self.node, &loc, o, force)? {
          place::Link::Draft(d) => *d,
          place::Link::Same(label) => {
            return Ok(format!(
              "Already the same folder: {label}; nothing to link."
            ));
          }
        }
      }
      None => place::unlink_draft(&self.node, &loc).map_err(|e| e.to_string())?,
    };
    let home = roots
      .iter()
      .find(|r| r.config().role == Role::Home)
      .ok_or("no home workspace")?;
    home.publish(&self.node, draft).map_err(|e| e.to_string())?;
    index.refresh(home).map_err(|e| e.to_string())?;
    let places = place::Places::load(&index);
    self.here = Place::of_dir(&cwd, self.node.as_str(), Some(home.dir()), &places).0;
    self.here_label = self.here.label(self.node.as_str(), &places);
    Ok(format!(
      "This folder is now {}; its threads are listed here.",
      self.here_label
    ))
  }

  fn home_repo(roots: &[RecordRoot]) -> Option<RepoId> {
    roots
      .iter()
      .find(|r| r.config().role == Role::Home)
      .map(|r| r.repo().clone())
  }

  /// The repository holding a record.
  fn root_of(&self, id: &RecordId) -> Option<RecordRoot> {
    let index = self.index.lock().expect("index lock");
    let repo = index.record(id).ok().flatten()?.repo;
    self.roots(&index).into_iter().find(|r| r.repo() == &repo)
  }

  /// The threads `scope` lists (this place's, global ones, every other), most
  /// recent first.
  fn threads(&self, scope: ThreadScope) -> Vec<ThreadItem> {
    let index = self.index.lock().expect("index lock");
    let roots = self.roots(&index);
    let home = Self::home_repo(&roots);
    let tz = jiff::tz::TimeZone::system();
    // A compaction chain is one row, its newest thread (design §2.15).
    let threads = index.listed_threads().unwrap_or_default();
    let places = place::Places::load(&index);
    let all = scope == ThreadScope::All;
    place::listed(threads, home.as_ref(), &self.here, all, &places)
      .into_iter()
      .filter(|(_, _, p)| scope != ThreadScope::Here || p == &self.here)
      .map(|(r, at, p)| ThreadItem {
        title: index
          .lineage_title(&r.id)
          .unwrap_or_else(|| "(untitled)".into()),
        when: at.to_zoned(tz.clone()).strftime("%b %d %H:%M").to_string(),
        state: stitch_node::chat::detached_state(&index, &r).ok().flatten(),
        place: (p != self.here).then(|| p.label(self.node.as_str(), &places)),
        key: p.key(),
        pinned: index.lineage_pinned(&r.id),
        parent: stitch_node::meta::parent(&index, &r.id).map(|p| p.to_string()),
        goal: r
          .fields
          .get("goal")
          .and_then(|v| v.as_str())
          .and_then(|g| g.trim().lines().next())
          .map(|g| g.chars().take(200).collect()),
        id: r.id.to_string(),
        at,
      })
      .collect()
  }

  /// The thread to open on start: the last one opened here, else this
  /// place's latest; none in a place with none of its own (a new thread
  /// there). Falling back to a global one had what the user typed in a
  /// fresh checkout go to an old global thread, which stays global
  /// (2026-10-23: a review of a project, in a cancelled test thread).
  fn reopen(&self) -> Option<String> {
    // The last one opened here may belong to another place.
    let threads = self.threads(ThreadScope::All);
    let last = self.last_opened();
    let here = self.here.key();
    // One compacted since is opened as its newest.
    let last: std::collections::BTreeMap<String, String> = last
      .into_iter()
      .map(|(k, id)| {
        let id = self.newest(&id).unwrap_or(id);
        (k, id)
      })
      .collect();
    last
      .get(&here)
      .filter(|id| threads.iter().any(|t| &t.id == *id))
      .cloned()
      .or_else(|| threads.iter().find(|t| t.key == here).map(|t| t.id.clone()))
  }

  /// The last thread opened per place (`tui.toml`, local to this node).
  fn last_opened(&self) -> std::collections::BTreeMap<String, String> {
    std::fs::read_to_string(self.data.join("tui.toml"))
      .ok()
      .and_then(|t| toml::from_str::<TuiState>(&t).ok())
      .map(|s| s.last)
      .unwrap_or_default()
  }

  /// Remembers a thread as the last opened here and in its own place.
  fn remember(&self, thread: &ThreadItem) {
    let mut last = self.last_opened();
    last.insert(self.here.key(), thread.id.clone());
    last.insert(thread.key.clone(), thread.id.clone());
    if let Ok(text) = toml::to_string(&TuiState { last }) {
      let _ = std::fs::write(self.data.join("tui.toml"), text);
    }
  }

  /// A thread's activity, its latest sealed segment (else when it was
  /// made), from the index: not from the thread list, which lists only the
  /// scope's threads, so the open thread of another place (or one whose
  /// folder became a checkout while the TUI ran) had none, and a turn the
  /// node ran on it was never followed (2026-10-30).
  fn activity_of(&self, id: &str) -> Option<jiff::Timestamp> {
    let id = id.parse::<RecordId>().ok()?;
    let index = self.index.lock().expect("index lock");
    match index.last_segment_at(&id).ok().flatten() {
      Some(at) => Some(at),
      None => index.record(&id).ok().flatten().map(|r| r.created),
    }
  }

  fn transcript(&self, id: &str) -> Vec<Entry> {
    self.transcript_to(id, None).unwrap_or_default()
  }

  /// A thread's transcript up to and with a segment (all of it, `None`);
  /// `None` when it cannot be read. Over its compaction chain: what it was
  /// compacted from first, each compaction a divider (`compacted`).
  fn transcript_to(&self, id: &str, head: Option<&str>) -> Option<Vec<Entry>> {
    let id = id.parse::<RecordId>().ok()?;
    let head = match head {
      Some(h) => Some(h.parse::<RecordId>().ok()?),
      None => None,
    };
    let root = self.root_of(&id)?;
    let ctx = ChatContext {
      node: &self.node,
      root: &root,
      index: &self.index,
      blobs: &self.blobs,
    };
    Some(
      stitch_node::chat::lineage_shown(&ctx, &id, head.as_ref())
        .ok()?
        .into_iter()
        .map(|e| Entry {
          who: e.who,
          text: e.text,
          said: e.said,
          id: e.id,
          at: e.at,
          attachments: e.attachments.len(),
          replies: e.replies,
          actions: e.actions,
          call: e.call,
        })
        .collect(),
    )
  }

  fn attention(&self) -> Vec<Needs> {
    let index = self.index.lock().expect("index lock");
    index
      .attention()
      .unwrap_or_default()
      .into_iter()
      .map(|a| match a {
        AttentionItem::Question { id, title } => Needs {
          text: question_line(&index, &id, title),
          detail: index.record(&id).ok().flatten().and_then(|r| {
            r.fields
              .get("detail")
              .and_then(|d| d.as_str())
              .map(str::to_owned)
          }),
          grant: index
            .record(&id)
            .ok()
            .flatten()
            .and_then(|r| r.fields.get("grant").and_then(|g| g.as_array()).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|g| g.as_str().map(str::to_owned))
            .collect(),
          question: Some(id.to_string()),
        },
        AttentionItem::Finding { finding } => Needs {
          question: None,
          grant: Vec::new(),
          detail: None,
          text: format!("still failing  {finding}"),
        },
        AttentionItem::OverBudget { task } => Needs {
          question: None,
          grant: Vec::new(),
          detail: None,
          text: format!("over budget  {task}"),
        },
      })
      .collect()
  }

  /// Reports of detached threads: (record, the thread it reports to).
  /// Every report, and whether this view shows it as it comes: one to a
  /// thread is that thread's (the open one has it in its transcript, any
  /// other is none of this view's business); one to nobody (a detached
  /// thread the user started) is shown where that thread belongs; one
  /// kept for the user (`wake = false`) is shown in the thread it went to
  /// while that one is `open`.
  fn reports(&self, open: Option<&str>) -> Vec<(String, bool)> {
    let index = self.index.lock().expect("index lock");
    let roots = self.roots(&index);
    let home = Self::home_repo(&roots);
    let places = place::Places::load(&index);
    index
      .records(Some(stitch_records::kinds::REPORT))
      .unwrap_or_default()
      .into_iter()
      .map(|r| {
        let edges = index.edges_from(&r.id).unwrap_or_default();
        let target = edges.iter().find(|e| e.rel == "reports-to");
        let to = target.is_some();
        let kept = r.fields.get("wake").and_then(|v| v.as_bool()) == Some(false);
        if kept && target.is_some_and(|e| Some(e.dst.id.as_str()) == open) {
          return (r.id.to_string(), true);
        }
        let here = edges
          .iter()
          .find(|e| e.rel == "thread")
          .and_then(|e| index.record(&e.dst.id).ok().flatten())
          .is_some_and(|t| Place::of_thread(&t, home.as_ref(), &places) == self.here);
        (r.id.to_string(), !to && here)
      })
      .collect()
  }

  /// A thread's place in the tree: its title and place, its parent, how
  /// many below it. Read from the index, whatever place it belongs to.
  fn family(&self, thread: &str) -> Option<Family> {
    let id = thread.parse::<RecordId>().ok()?;
    let children = self.children(thread).len();
    let index = self.index.lock().expect("index lock");
    let record = index.record(&id).ok().flatten()?;
    let roots = self.roots(&index);
    let home = Self::home_repo(&roots);
    let places = place::Places::load(&index);
    let title = |id: &RecordId| {
      index
        .title(id)
        .ok()
        .flatten()
        .unwrap_or_else(|| "(untitled)".into())
    };
    let p = Place::of_thread(&record, home.as_ref(), &places);
    Some(Family {
      id: thread.to_owned(),
      title: title(&id),
      place: (p != self.here).then(|| p.label(self.node.as_str(), &places)),
      parent: parent_of(&index, &id).map(|p| (p.to_string(), title(&p))),
      children,
    })
  }

  /// The threads below `thread`, as choices for the picker: the title,
  /// then its state, when it started and where it belongs.
  fn children(&self, thread: &str) -> Vec<Choice> {
    let Ok(id) = thread.parse::<RecordId>() else {
      return Vec::new();
    };
    let index = self.index.lock().expect("index lock");
    let roots = self.roots(&index);
    let home = Self::home_repo(&roots);
    let places = place::Places::load(&index);
    let tz = jiff::tz::TimeZone::system();
    children_of(&index, &id)
      .into_iter()
      .map(|r| {
        let title = index
          .title(&r.id)
          .ok()
          .flatten()
          .unwrap_or_else(|| "(untitled)".into());
        let mut detail = Vec::new();
        if let Some(s) = stitch_node::chat::detached_state(&index, &r).ok().flatten() {
          detail.push(s);
        }
        detail.push(
          r.created
            .to_zoned(tz.clone())
            .strftime("%b %d %H:%M")
            .to_string(),
        );
        let p = Place::of_thread(&r, home.as_ref(), &places);
        if p != self.here {
          detail.push(p.label(self.node.as_str(), &places));
        }
        Choice {
          value: r.id.to_string(),
          label: title,
          detail: detail.join(" · "),
        }
      })
      .collect()
  }

  /// The reports kept for the user on a thread (`wake = false`) since it
  /// last ran: what is news to whoever opens it.
  fn kept_for(&self, thread: &str) -> Vec<String> {
    let Ok(id) = thread.parse::<RecordId>() else {
      return Vec::new();
    };
    let ids: Vec<String> = {
      let index = self.index.lock().expect("index lock");
      let since = index.last_segment_at(&id).ok().flatten();
      index
        .sources(stitch_records::kinds::REPORT, "reports-to", &id, None)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.fields.get("wake").and_then(|v| v.as_bool()) == Some(false))
        .filter(|r| since.is_none_or(|t| r.created > t))
        .map(|r| r.id.to_string())
        .collect()
    };
    ids.iter().filter_map(|r| self.report(r)).collect()
  }

  /// A report as a notice: its heading without the `#`, then its text.
  fn report(&self, id: &str) -> Option<String> {
    let id: RecordId = id.parse().ok()?;
    let body = self.root_of(&id)?.read_record(&id).ok()?.body;
    let body = body.trim();
    Some(format!("◆ {}", body.strip_prefix("# ").unwrap_or(body)))
  }

  /// A chat on the open thread ended at `segment`: what it showed live is
  /// caught up with, and what the thread gained after it (a report's turn,
  /// woken as the chat ended) is printed, not skipped.
  fn chat_ended(&self, app: &mut App, thread: &str, segment: &str) {
    let Some(id) = app.thread.clone().filter(|t| t == thread) else {
      self.follow(app, false);
      return;
    };
    let at = self.activity_of(&id);
    let transcript = self.transcript(&id);
    let shown = self
      .transcript_to(&id, Some(segment))
      .map_or(transcript.len(), |t| t.len());
    app.shown.0 = shown.min(transcript.len());
    app.followed(&transcript);
    app.shown.1 = at;
  }

  /// The same store on a connection of its own, for the poller's thread.
  fn reader(&self) -> std::result::Result<Store, stitch_store::Error> {
    Ok(Store {
      data: self.data.clone(),
      node: self.node.clone(),
      here: self.here.clone(),
      here_label: self.here_label.clone(),
      index: Mutex::new(Index::open(&self.data.join("index.sqlite"))?),
      blobs: BlobStore::open(&self.data.join("blobs"))?,
    })
  }

  /// The open thread's open segment, when last written.
  fn open_segment_at(&self, thread: Option<&str>) -> Option<jiff::Timestamp> {
    let t = thread?.parse::<RecordId>().ok()?;
    self.root_of(&t)?.open_segment_at(&t)
  }

  /// What the timer reads again (other nodes and other commands write
  /// too): the thread list, the attention view, the reports, and the open
  /// thread's transcript when its activity moved past what was shown.
  fn poll(&self, ask: &Poll) -> Polled {
    let threads = self.threads(ask.scope);
    // The open thread was compacted (a turn the node ran on it, not this
    // TUI's own chat, which is told): it goes on in the newest, whose
    // transcript is the old one's, a divider, then what came since.
    let moved = ask
      .thread
      .as_deref()
      .filter(|_| ask.follow)
      .and_then(|id| self.newest(id));
    let open = moved.as_deref().or(ask.thread.as_deref());
    let follow = open.filter(|_| ask.follow).and_then(|id| {
      let at = self.activity_of(id).max(self.open_segment_at(Some(id)));
      (at != ask.shown.1 || moved.is_some()).then(|| (at, self.transcript(id)))
    });
    Polled {
      attention: self.attention(),
      reports: self.reports(open),
      threads,
      follow,
      family: open.and_then(|id| self.family(id)),
      moved,
    }
  }

  /// The newest thread `id` was compacted into, when it was.
  fn newest(&self, id: &str) -> Option<String> {
    let t = id.parse::<RecordId>().ok()?;
    let now = self.index.lock().expect("index lock").compaction_newest(&t);
    (now != t).then(|| now.to_string())
  }

  /// Applies a poll's answer, if the view is still the one it was asked
  /// for (a thread opened meanwhile has it dropped: the next one is
  /// asked at once).
  fn polled(
    &self,
    app: &mut App,
    ask: &Poll,
    got: Polled,
    seen_reports: &mut std::collections::HashSet<String>,
  ) -> bool {
    if app.scope == ask.scope {
      app.set_threads(got.threads);
    }
    app.attention_read(got.attention, false);
    if app.thread != ask.thread {
      return false;
    }
    if let Some(to) = got.moved {
      app.compacted_into(to);
    }
    if got.family.is_some() {
      app.family = got.family;
    }
    if app.shown != ask.shown {
      return false;
    }
    if let Some((at, transcript)) = got.follow
      && app.pending.as_ref().is_none_or(|p| p.waiting)
    {
      app.followed(&transcript);
      app.shown.1 = at;
    }
    for (id, shown) in got.reports {
      if seen_reports.insert(id.clone())
        && shown
        && let Some(text) = self.report(&id)
      {
        app.out.push(Printed::Notice(text));
      }
    }
    true
  }

  /// The timer's work, done here and now (the bench; the event loop has
  /// the poller do it).
  #[cfg(test)]
  fn tick(&self, app: &mut App, seen_reports: &mut std::collections::HashSet<String>) {
    let ask = Poll::of(app, &self.here, false);
    let got = self.poll(&ask);
    self.polled(app, &ask, got, seen_reports);
  }

  /// Prints what the open thread gained since it was last read, if its
  /// activity moved (`print` false: only catch up, the TUI showed it live).
  fn follow(&self, app: &mut App, print: bool) {
    let Some(mut id) = app.thread.clone() else {
      return;
    };
    // Compacted meanwhile: it goes on in the newest (its transcript
    // continues this one's, after a divider).
    if let Some(to) = self.newest(&id) {
      app.compacted_into(to.clone());
      id = to;
    }
    // A turn's steps go to its open segment, which the thread list does
    // not see till it is sealed when the turn ends: a TUI waiting for a
    // long turn showed nothing of it for minutes (2026-10-21).
    let open = id
      .parse::<RecordId>()
      .ok()
      .and_then(|t| self.root_of(&t)?.open_segment_at(&t));
    let at = self.activity_of(&id).max(open);
    if at == app.shown.1 && print {
      return;
    }
    let transcript = self.transcript(&id);
    if !print {
      app.shown.0 = transcript.len();
    }
    app.followed(&transcript);
    app.shown.1 = at;
  }
}

/// What the event loop asks the poller to read: the view as it is.
#[derive(Clone, Debug, PartialEq)]
struct Poll {
  scope: ThreadScope,
  thread: Option<String>,
  shown: (usize, Option<jiff::Timestamp>),
  here: Place,
  /// The open thread is followed (no turn of this TUI's own runs).
  follow: bool,
  /// Read even if nothing changed (after something this TUI did).
  force: bool,
}

impl Poll {
  fn of(app: &App, here: &Place, force: bool) -> Poll {
    Poll {
      scope: app.scope,
      thread: app.thread.clone(),
      shown: app.shown,
      here: here.clone(),
      follow: app.pending.as_ref().is_none_or(|p| p.waiting),
      force,
    }
  }
}

/// The event loop's side of the poller: one question out at a time.
struct Poller {
  ask: std::sync::mpsc::Sender<Poll>,
  busy: bool,
  /// Asked again (`Some(force)`) once the answer is in.
  again: Option<bool>,
}

impl Poller {
  fn ask(&mut self, app: &App, here: &Place, force: bool) {
    if self.busy {
      self.again = Some(self.again.unwrap_or(false) || force);
      return;
    }
    self.busy = self.ask.send(Poll::of(app, here, force)).is_ok();
  }

  /// An answer came: what was asked meanwhile goes now.
  fn answered(&mut self, app: &App, here: &Place) {
    self.busy = false;
    if let Some(force) = self.again.take() {
      self.ask(app, here, force);
    }
  }
}

/// What the poller read.
struct Polled {
  threads: Vec<ThreadItem>,
  attention: Vec<Needs>,
  reports: Vec<(String, bool)>,
  /// The open thread's new activity and transcript, when it moved.
  follow: Option<(Option<jiff::Timestamp>, Vec<Entry>)>,
  /// The open thread's title, parent and children (the status line's ↑, ↓).
  family: Option<Family>,
  /// The open thread was compacted: the newest thread it continues in.
  moved: Option<String>,
}

/// How often the poller walks the record roots for files the node did
/// not index.
const SCAN_EVERY: Duration = Duration::from_secs(30);

/// The poller: a thread with its own connection to the index, so the
/// event loop never waits on the store's reads (they took 90 ms every two
/// seconds on a long thread, and the keys typed meanwhile waited). It
/// answers each `Poll` with `UiEvent::Polled`: `None` when nothing
/// changed since the last answer: the index's `data_version` (the node
/// commits all it writes), the open segment's time (a turn's steps, not
/// indexed till sealed) and the view asked about are the same, and the
/// roots' walk, now and then, found nothing new.
fn spawn_poller(store: Store, tx: mpsc::UnboundedSender<UiEvent>) -> std::sync::mpsc::Sender<Poll> {
  let (ask, asked) = std::sync::mpsc::channel::<Poll>();
  std::thread::spawn(move || {
    let mut store = store;
    let mut last: Option<(i64, Option<jiff::Timestamp>, Poll)> = None;
    let mut scanned = std::time::Instant::now();
    // The event loop asks again only once answered.
    while let Ok(poll) = asked.recv() {
      let mut changed = poll.force;
      if scanned.elapsed() >= SCAN_EVERY {
        scanned = std::time::Instant::now();
        changed |= store.scan();
      }
      let version = store.index.lock().expect("index lock").data_version().ok();
      let open = store.open_segment_at(poll.thread.as_deref());
      let key = (
        version.unwrap_or(-1),
        open,
        Poll {
          force: false,
          ..poll.clone()
        },
      );
      if !changed && version.is_some() && last.as_ref() == Some(&key) {
        if tx.send(UiEvent::Polled(poll, None)).is_err() {
          break;
        }
        continue;
      }
      store.here = poll.here.clone();
      let got = store.poll(&poll);
      last = Some(key);
      if tx.send(UiEvent::Polled(poll, Some(Box::new(got)))).is_err() {
        break;
      }
    }
  });
  ask
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct TuiState {
  #[serde(default)]
  last: std::collections::BTreeMap<String, String>,
}

enum UiEvent {
  EffectiveModel(Option<String>, Option<String>, bool, String),
  /// The poller's answer to a `Poll` (`None`: nothing changed).
  Polled(Poll, Option<Box<Polled>>),
  AuthUrl(String),
  /// GitHub's device code and where to enter it.
  AuthCode {
    code: String,
    url: String,
  },
  AuthDone(std::result::Result<String, String>),
  /// From the chat started `n`th (one let go of says nothing more here).
  Reply(u64, Reply),
  /// A command waits for approval; the answer goes back through `answer`.
  Approve {
    chat: u64,
    id: u64,
    session: String,
    command: String,
    question: String,
    grant: Vec<String>,
    answer: tokio::sync::oneshot::Sender<stitch_node::tools::Decision>,
  },
  Done(u64, std::result::Result<Reply, String>),
  /// A line for the conversation (results of side requests).
  Notice(String),
  /// A picker's choices, and a note under them.
  Picked(Vec<Choice>, String),
  /// Whether the open thread is in memory (`None`: no memory here).
  Memory(Option<bool>),
  /// Whether the open thread is read-only.
  ReadOnly(bool),
  /// The node the open thread runs on, when it was set to one.
  RunsOn(Option<String>),
  /// Whether shell commands run without asking, and what to say about it.
  /// The approval mode (`ask`, `auto`, `yolo`), and the node's line when
  /// it was set.
  Mode(String, Option<String>),
  /// What runs in the background.
  Jobs(Vec<stitch_node::api::Job>),
  /// A line for the status bar.
  Status(String),
  /// The step whose calls run now on a thread (`Request::InFlight`).
  InFlight(String, Option<stitch_node::chat::StepInFlight>),
  /// A thread was rewound; `resend`: its message to send again.
  Rewound {
    thread: String,
    upto: Option<String>,
    resend: Option<String>,
  },
  /// What deleting a thread would take (the node's dry run).
  DeletePreview(String, Vec<String>),
  /// Threads were deleted (`<id>\t<title>\t<role>` each).
  Deleted(Vec<String>),
  /// A fork was written: open it.
  Forked(String),
  /// A thread quoted into a file: a new thread, with it attached.
  Quoted(String),
}

/// The thread above `thread` (`meta::parent`).
fn parent_of(index: &Index, thread: &RecordId) -> Option<RecordId> {
  stitch_node::meta::parent(index, thread)
}

/// The threads below `thread`, the newest first: those `parent_of` leads
/// to it. Those spawned by (or reporting to) an earlier compaction of it
/// are its own, and a compacted child is listed as its newest version.
fn children_of(index: &Index, thread: &RecordId) -> Vec<stitch_store::IndexedRecord> {
  use stitch_records::kinds::THREAD;
  let compacted = |r: &stitch_store::IndexedRecord| {
    r.fields.get("compacted").and_then(|v| v.as_bool()) == Some(true)
  };
  // It and what it was before it was compacted.
  let mut chain = vec![thread.clone()];
  for _ in 0..64 {
    let Some(me) = index.record(chain.last().expect("one")).ok().flatten() else {
      break;
    };
    if !compacted(&me) {
      break;
    }
    let prev = index
      .edges_from(&me.id)
      .unwrap_or_default()
      .into_iter()
      .find(|e| e.rel == "continues")
      .map(|e| e.dst.id);
    match prev {
      Some(p) if !chain.contains(&p) => chain.push(p),
      _ => break,
    }
  }
  let mut out: Vec<stitch_store::IndexedRecord> = Vec::new();
  for id in &chain {
    for rel in ["spawned-by", "reports-to"] {
      for r in index.sources(THREAD, rel, id, None).unwrap_or_default() {
        // A thread is a child of one parent; an old version of a
        // compacted child is not listed beside its newest.
        let superseded = index
          .sources(THREAD, "continues", &r.id, None)
          .unwrap_or_default()
          .iter()
          .any(compacted);
        if superseded
          || out.iter().any(|o| o.id == r.id)
          || parent_of(index, &r.id).as_ref() != Some(thread)
        {
          continue;
        }
        out.push(r);
      }
    }
  }
  out.sort_by_key(|r| std::cmp::Reverse(r.created));
  out
}

/// A question on the attention list as one line: what it asks, for which
/// thread, the node that asked and how long ago, so a stale one shows.
pub(crate) fn question_line(index: &Index, id: &RecordId, title: Option<String>) -> String {
  let thread = index
    .edges_from(id)
    .ok()
    .and_then(|e| e.into_iter().find(|e| e.rel == "thread"))
    .and_then(|e| index.title(&e.dst.id).ok().flatten())
    .map(|t| format!(" · thread \"{}\"", app::clip(&t, 50)))
    .unwrap_or_default();
  let (node, ago) = match index.record(id).ok().flatten() {
    Some(r) => {
      let secs = (jiff::Timestamp::now() - r.created)
        .total(jiff::Unit::Second)
        .unwrap_or(0.0)
        .max(0.0) as u64;
      let ago = match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
        _ => format!("{}d", secs / 86400),
      };
      (format!(" · on {}", r.node), format!(" · {ago} ago"))
    }
    None => (String::new(), String::new()),
  };
  format!("{}{thread}{node}{ago}", title.unwrap_or_default())
}

/// Puts text on the clipboard: through the terminal (OSC 52, which also
/// works over SSH and in tmux with clipboard on) and the platform's tool.
fn copy_to_clipboard(text: &str) {
  use base64::Engine;
  use std::io::Write;
  let encoded = base64::engine::general_purpose::STANDARD.encode(text);
  let mut out = std::io::stdout();
  let _ = write!(out, "\x1b]52;c;{encoded}\x07");
  let _ = out.flush();
  for (program, args) in [
    ("pbcopy", &[][..]),
    ("wl-copy", &[][..]),
    ("xclip", &["-selection", "clipboard"][..]),
  ] {
    let child = std::process::Command::new(program)
      .args(args)
      .stdin(std::process::Stdio::piped())
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .spawn();
    if let Ok(mut child) = child {
      if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
      }
      let _ = child.wait();
      return;
    }
  }
}

/// Attaches the clipboard's files, or its image written to a PNG of this
/// process's own (`$TMPDIR/stitch-paste-<pid>/image-<n>.png`, read by the
/// node when the message is sent); `pasted` counts the images.
fn paste_clipboard(app: &mut App, pasted: &mut usize) {
  let dir = std::env::temp_dir().join(format!("stitch-paste-{}", std::process::id()));
  match clipboard::read(&dir, *pasted + 1) {
    Ok(clipboard::Pasted::Files(files)) => app.attach_files(&files),
    Ok(clipboard::Pasted::Image(path)) => {
      *pasted += 1;
      app.attach_files(&[path]);
    }
    Err(why) => app.status = why,
  }
}

type Term = ratatui::Terminal<CrosstermBackend<Tty>>;

/// The terminal's events, read on a thread of their own. It polls with a
/// timeout rather than waiting, so crossterm's event reader is let go of
/// between polls and `cursor::position` (asked on a resize) can take it to
/// read the terminal's answer. crossterm's `EventStream` holds the reader
/// while it waits for input: the answer would be read only after its two
/// seconds ran out, or a key came.
fn spawn_events() -> mpsc::UnboundedReceiver<std::io::Result<Event>> {
  use ratatui::crossterm::event;
  let (tx, rx) = mpsc::unbounded_channel();
  std::thread::spawn(move || {
    loop {
      match event::poll(Duration::from_millis(25)) {
        Ok(true) => {
          let ev = event::read();
          let failed = ev.is_err();
          if tx.send(ev).is_err() || failed {
            break;
          }
        }
        Ok(false) if tx.is_closed() => break,
        Ok(false) => {}
        Err(e) => {
          let _ = tx.send(Err(e));
          break;
        }
      }
    }
  });
  rx
}

/// Where the screen's bytes go: the terminal, or in tests a terminal
/// emulator (`emu`), which answers for its size and cursor too.
#[derive(Clone)]
enum Tty {
  Stdout,
  #[cfg(test)]
  Emu(std::sync::Arc<Mutex<emu::Emu>>),
}

impl std::io::Write for Tty {
  fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
    match self {
      Tty::Stdout => std::io::stdout().write(buf),
      #[cfg(test)]
      Tty::Emu(e) => {
        e.lock().unwrap().feed(buf);
        Ok(buf.len())
      }
    }
  }

  fn flush(&mut self) -> std::io::Result<()> {
    match self {
      Tty::Stdout => std::io::stdout().flush(),
      #[cfg(test)]
      Tty::Emu(_) => Ok(()),
    }
  }
}

impl Tty {
  /// Rows and columns.
  fn size(&self) -> (u16, u16) {
    match self {
      Tty::Stdout => {
        let (cols, rows) = terminal::size().unwrap_or((80, 24));
        (rows, cols)
      }
      #[cfg(test)]
      Tty::Emu(e) => e.lock().unwrap().size(),
    }
  }

  /// The cursor's row, asked of the terminal.
  fn cursor_row(&self) -> std::io::Result<u16> {
    match self {
      Tty::Stdout => ratatui::crossterm::cursor::position().map(|(_, row)| row),
      #[cfg(test)]
      Tty::Emu(e) => Ok(e.lock().unwrap().cursor().0),
    }
  }
}

/// The live area: a fixed rectangle under the printed conversation. What
/// is finished is printed above it as plain text, with newlines only where
/// the text has them, so the terminal wraps, scrolls, searches and copies.
/// Its place is computed as text is printed; the terminal's own cursor,
/// hidden, is parked where the text ends, so that when a resize reflows
/// the screen it is asked where that went (`resize`).
struct Screen {
  tty: Tty,
  term: Term,
  /// Where the next printed text starts.
  text_row: u16,
  /// The live area's height; it sits on the bottom rows.
  height: u16,
  rows: u16,
  cols: u16,
  /// The Markdown of the reply being printed (an open fence, a table).
  md: markdown::Md,
}

/// Rows `text` takes in a terminal `cols` wide: each line wraps by its
/// display width (wide characters count twice, tabs reach the next stop).
fn rows_of(text: &str, cols: u16) -> u16 {
  use unicode_width::UnicodeWidthChar;
  let cols = usize::from(cols.max(1));
  text
    .split('\n')
    .map(|line| {
      let mut width = 0usize;
      for c in line.chars() {
        width += match c {
          '\t' => 8 - width % 8,
          c => c.width().unwrap_or(0),
        };
      }
      width.div_ceil(cols).max(1) as u16
    })
    .sum()
}

impl Screen {
  /// Takes the whole screen: what the shell showed scrolls up into the
  /// scrollback, the conversation starts at the top and the live area sits
  /// on the bottom rows. The cursor is read where the shell left it.
  fn new(tty: Tty, height: u16) -> std::io::Result<Screen> {
    let (rows, cols) = tty.size();
    let row = tty.cursor_row().unwrap_or(0);
    scroll(&tty, rows, row)?;
    let mut s = Screen {
      term: fixed(&tty, 0, 1, cols)?,
      tty,
      text_row: 0,
      height: 1,
      rows,
      cols,
      md: markdown::Md::default(),
    };
    s.layout(height)?;
    Ok(s)
  }

  /// Puts a live area of `height` rows on the bottom rows, scrolling the
  /// text up first if it reaches into them.
  fn layout(&mut self, height: u16) -> std::io::Result<()> {
    let height = height.min(self.rows.max(1));
    let top = self.rows - height;
    if self.text_row > top {
      scroll(&self.tty, self.rows, self.text_row - top)?;
      self.text_row = top;
    }
    execute!(
      self.tty,
      MoveTo(0, self.text_row),
      Clear(ClearType::FromCursorDown)
    )?;
    self.term = fixed(&self.tty, top, height, self.cols)?;
    self.height = height;
    Ok(())
  }

  /// Prints `items` after the text so far, and puts a live area of `height`
  /// rows on the bottom rows.
  fn relayout(&mut self, items: &[Printed], height: u16) -> std::io::Result<()> {
    self.sync()?;
    let mut out = self.tty.clone();
    execute!(
      out,
      MoveTo(0, self.text_row),
      Clear(ClearType::FromCursorDown)
    )?;
    let mut used = 0u32;
    for item in items {
      let (text, plain) = printed(item, &mut self.md, self.cols);
      if text.is_empty() {
        continue;
      }
      write!(out, "{text}")?;
      used += u32::from(rows_of(&plain, self.cols));
    }
    out.flush()?;
    // Printing past the bottom scrolled the screen; the cursor is on the
    // last row then.
    self.text_row =
      (u32::from(self.text_row) + used).min(u32::from(self.rows.saturating_sub(1))) as u16;
    self.layout(height)
  }

  /// The window resized. The terminal has reflowed what it shows (wrapped
  /// or joined lines, pulled rows back from its scrollback or pushed them
  /// into it), so the row where the text ended is no longer known; the
  /// cursor, parked there, moved with the text, so it is asked for. From
  /// there down, what the live area left is cleared, and the live area
  /// drawn again on the new bottom rows.
  fn resize(&mut self, rows: u16, cols: u16, height: u16) -> std::io::Result<()> {
    (self.rows, self.cols) = (rows, cols);
    if let Ok(row) = self.tty.cursor_row() {
      self.text_row = row;
    }
    self.text_row = self.text_row.min(rows.saturating_sub(1));
    self.layout(height)
  }

  /// Takes a new size of the terminal into account before anything is
  /// written at the old one's rows: the resize event may come after a
  /// draw, and a draw would park the cursor where the text no longer ends.
  /// Whether it changed.
  fn sync(&mut self) -> std::io::Result<bool> {
    let (rows, cols) = self.tty.size();
    if (rows, cols) == (self.rows, self.cols) {
      return Ok(false);
    }
    self.resize(rows, cols, self.height.min(rows.saturating_sub(1)).max(1))?;
    Ok(true)
  }

  /// Leaves the terminal as a shell expects it: the live area gone, the
  /// prompt right under the conversation.
  fn close(&mut self) -> std::io::Result<()> {
    execute!(
      self.tty,
      MoveTo(0, self.text_row),
      Clear(ClearType::FromCursorDown),
      Show
    )?;
    self.tty.flush()
  }

  /// Draws the live area, then parks the terminal's cursor (hidden; the
  /// input draws its own caret) where the text ends, for `resize`.
  fn draw(&mut self, render: impl FnOnce(&mut ratatui::Frame)) -> std::io::Result<()> {
    self.sync()?;
    self.term.draw(render)?;
    execute!(self.tty, MoveTo(0, self.text_row))
  }
}

/// Scrolls the screen up `lines` lines, into the scrollback.
fn scroll(tty: &Tty, rows: u16, lines: u16) -> std::io::Result<()> {
  if lines == 0 {
    return Ok(());
  }
  let mut out = tty.clone();
  execute!(out, MoveTo(0, rows.saturating_sub(1)))?;
  write!(out, "{}", "\r\n".repeat(usize::from(lines)))?;
  out.flush()
}

fn fixed(tty: &Tty, top: u16, height: u16, cols: u16) -> std::io::Result<Term> {
  ratatui::Terminal::with_options(
    CrosstermBackend::new(tty.clone()),
    TerminalOptions {
      viewport: Viewport::Fixed(ratatui::layout::Rect::new(0, top, cols, height.max(1))),
    },
  )
}

/// `item` with only what the terminal shows as text (see `app::clean`).
fn cleaned(item: &Printed) -> Printed {
  let c = |t: &String| app::clean(t);
  match item {
    Printed::You(t) => Printed::You(c(t)),
    Printed::Reply { model, text } => Printed::Reply {
      model: c(model),
      text: c(text),
    },
    Printed::ReplyStart(t) => Printed::ReplyStart(c(t)),
    Printed::ReplyLines(t) => Printed::ReplyLines(c(t)),
    Printed::ReplyEnd => Printed::ReplyEnd,
    Printed::Tool(t) => Printed::Tool(c(t)),
    Printed::Output(t) => Printed::Output(c(t)),
    Printed::Error(t) => Printed::Error(c(t)),
    Printed::Refusal(t) => Printed::Refusal(c(t)),
    Printed::Notice(t) => Printed::Notice(c(t)),
    Printed::Separator(t) => Printed::Separator(c(t)),
    Printed::Compacted { line, summary } => Printed::Compacted {
      line: c(line),
      summary: c(summary),
    },
  }
}

/// The lines of a compaction's summary printed under its divider; the
/// rest is said as a count.
const SUMMARY_LINES: usize = 12;

/// A compaction's summary, dimmed and cut to `SUMMARY_LINES`.
fn summary_shown(summary: &str) -> String {
  let lines: Vec<&str> = summary.lines().collect();
  let mut text = lines
    .iter()
    .take(SUMMARY_LINES)
    .copied()
    .collect::<Vec<_>>()
    .join("\n");
  if lines.len() > SUMMARY_LINES {
    text.push_str(&format!(
      "\n[… {} more lines of the summary]",
      lines.len() - SUMMARY_LINES
    ));
  }
  text
}

/// Each line styled on its own, joined with CR LF (the terminal is raw).
fn lines_styled(text: &str, style: impl Fn(&str) -> String) -> String {
  text.lines().map(style).collect::<Vec<_>>().join("\r\n")
}

/// Rendered Markdown lines: as terminal text (CR LF between lines, links
/// as OSC 8 hyperlinks), and as the plain text they show.
fn md_text(lines: &[markdown::MdLine]) -> (String, String) {
  use ratatui::backend::IntoCrossterm as _;
  let styled = lines
    .iter()
    .map(|line| {
      line
        .iter()
        .map(|seg| {
          let text = seg.style.into_crossterm().apply(seg.text.as_str());
          match &seg.link {
            Some(url) => format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\"),
            None => text.to_string(),
          }
        })
        .collect::<String>()
    })
    .collect::<Vec<_>>()
    .join("\r\n");
  let shown = lines
    .iter()
    .map(|l| markdown::plain(l))
    .collect::<Vec<_>>()
    .join("\n");
  (styled, shown)
}

/// A finished piece of the conversation: as terminal text, and as the
/// plain text it shows (for counting the rows it takes); both empty when
/// it shows nothing yet (a table's rows, held till it ends). A reply's
/// Markdown is rendered, `md` carrying it from one streamed piece to the
/// next, for a terminal `cols` wide.
fn printed(item: &Printed, md: &mut markdown::Md, cols: u16) -> (String, String) {
  let item = &cleaned(item);
  let plain = |t: &str| lines_styled(t, |l| l.to_owned());
  let (styled, shown) = match item {
    Printed::You(t) => (
      format!("{}\r\n{}\r\n", "you".bold().cyan(), plain(t)),
      format!("you\n{t}\n"),
    ),
    Printed::Reply { model, text } => {
      let (styled, shown) = md_text(&markdown::render(text.trim_end_matches('\n'), cols));
      (
        format!("{}\r\n{styled}\r\n", model.as_str().bold().green()),
        format!("{model}\n{shown}\n"),
      )
    }
    // A streaming reply prints as it comes: heading, lines, a blank line.
    Printed::ReplyStart(model) => {
      *md = markdown::Md::new(cols);
      (model.as_str().bold().green().to_string(), model.clone())
    }
    // Split on every newline: blank lines between paragraphs stay.
    Printed::ReplyLines(t) => {
      md.width = cols;
      let lines = md.push(t);
      if lines.is_empty() {
        return (String::new(), String::new());
      }
      md_text(&lines)
    }
    // What was held (a table at the very end) prints before the blank line.
    Printed::ReplyEnd => {
      md.width = cols;
      let lines = md.finish();
      if lines.is_empty() {
        (String::new(), String::new())
      } else {
        let (styled, shown) = md_text(&lines);
        (format!("{styled}\r\n"), format!("{shown}\n"))
      }
    }
    Printed::Tool(t) => (lines_styled(t, |l| l.yellow().to_string()), t.clone()),
    Printed::Output(t) => (
      format!("{}\r\n", lines_styled(t, |l| l.dark_grey().to_string())),
      format!("{t}\n"),
    ),
    Printed::Error(t) => (
      format!("{}\r\n", lines_styled(t, |l| l.red().to_string())),
      format!("{t}\n"),
    ),
    Printed::Refusal(t) => (format!("{}\r\n", t.as_str().red().bold()), format!("{t}\n")),
    Printed::Notice(t) => (
      format!("{}\r\n", lines_styled(t, |l| l.italic().to_string())),
      format!("{t}\n"),
    ),
    Printed::Separator(t) => (
      format!("{}\r\n", format!("── {t} ──").dark_grey()),
      format!("── {t} ──\n"),
    ),
    Printed::Compacted { line, summary } => {
      let summary = summary_shown(summary);
      let head = format!("── {line} ──");
      if summary.is_empty() {
        (
          format!("{}\r\n", head.as_str().dark_grey()),
          format!("{head}\n"),
        )
      } else {
        (
          format!(
            "{}\r\n{}\r\n",
            head.as_str().dark_grey(),
            lines_styled(&summary, |l| l.dark_grey().italic().to_string())
          ),
          format!("{head}\n{summary}\n"),
        )
      }
    }
  };
  (format!("{styled}\r\n"), shown)
}

pub fn run(store: Store, default_model: String) -> Result<()> {
  let rt = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()?;
  terminal::enable_raw_mode()?;
  // A paste comes as one event, not as keys (a newline in it is no Enter).
  execute!(std::io::stdout(), EnableBracketedPaste)?;
  // Shift+Enter told from Enter, where the terminal can (the kitty
  // keyboard protocol); elsewhere both send a carriage return.
  let enhanced = terminal::supports_keyboard_enhancement().unwrap_or(false)
    && execute!(
      std::io::stdout(),
      PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )
    .is_ok();
  let restore = move || {
    if enhanced {
      let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    let _ = terminal::disable_raw_mode();
  };
  let hook = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    restore();
    hook(info);
  }));
  let result = rt.block_on(event_loop(store, default_model));
  restore();
  println!();
  result
}

async fn event_loop(mut store: Store, default_model: String) -> Result<()> {
  let exe = crate::upgrade::installed_exe()?;
  let mut app = App::new(store.node.to_string(), default_model);
  let mut pasted = 0usize;
  app.here = store.here_label.clone();
  app.files_root = std::env::current_dir().ok();
  app.set_threads(store.threads(ThreadScope::HereAndGlobal));
  app.attention_read(store.attention(), true);
  // Reports already there are history; new ones are printed as they come.
  let mut seen_reports: std::collections::HashSet<String> =
    store.reports(None).into_iter().map(|(id, _)| id).collect();
  app.out.push(Printed::Notice(format!(
    "stitch on {} · {} · Ctrl+N new thread (/new global: one for no folder) · Ctrl+O threads",
    app.node, app.here
  )));
  // The thread last open here comes back.
  if let Some(id) = store.reopen() {
    let transcript = store.transcript(&id);
    let kept = store.kept_for(&id);
    // Before it opens: its separator takes its title from it.
    app.family = store.family(&id);
    app.opened(Some(id), &transcript);
    app.out.extend(kept.into_iter().map(Printed::Notice));
  }
  let (_, cols0) = Tty::Stdout.size();
  let mut screen = Screen::new(Tty::Stdout, app.height(cols0))?;
  let (tx, mut rx) = mpsc::unbounded_channel::<UiEvent>();
  // The approval on screen: its ID on the chat's connection, and where
  // the answer goes.
  let mut approval: Option<(
    u64,
    tokio::sync::oneshot::Sender<stitch_node::tools::Decision>,
  )> = None;
  // To the chat that runs: messages for it, and a stop.
  let mut to_chat: Option<mpsc::UnboundedSender<Request>> = None;
  // The chat that runs, by number: one let go of (another thread opened
  // while it ran) goes on at the node, and what it still sends is not
  // this view's.
  let mut chat_no: u64 = 0;
  let mut chat_task: Option<tokio::task::JoinHandle<()>> = None;
  let mut events = spawn_events();
  // The store is read again on a timer, on the poller's thread.
  let mut poller = Poller {
    ask: spawn_poller(store.reader()?, tx.clone()),
    busy: false,
    again: None,
  };
  let mut tick = tokio::time::interval(Duration::from_secs(2));
  // A question for the open thread's call in flight is out.
  let mut asking_in_flight = false;
  // The spinner's frames, while a turn is pending or followed in a call.
  let mut spin = tokio::time::interval(Duration::from_millis(120));
  spin.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
  ask_memory(&tx, &store, &exe, app.thread.clone(), app.global);
  ask_mode(&tx, &store, &exe, None);
  // Where a new thread goes, as memory was last asked about: `/new global`
  // with nothing open changes it without opening anything.
  let mut asked_global = app.global;
  let mut model_context = None;
  let result = loop {
    let context = (app.thread.clone(), app.model.clone(), app.global);
    if model_context.as_ref() != Some(&context) {
      app.default_model = "asking node…".into();
      ask_effective_model(&tx, &store, &exe, &app);
      model_context = Some(context);
    }
    if app.thread.is_none() && app.global != asked_global {
      ask_memory(&tx, &store, &exe, None, app.global);
    }
    asked_global = app.global;
    // A resize is taken from the terminal's size, before anything is
    // written: its event may come later, or several at once.
    if let Err(e) = screen.sync() {
      break Err(e.into());
    }
    let want = app
      .height(screen.cols)
      .min(screen.rows.saturating_sub(1))
      .max(3);
    if !app.out.is_empty() || want != screen.height {
      let items = std::mem::take(&mut app.out);
      if let Err(e) = screen.relayout(&items, want) {
        break Err(e.into());
      }
    }
    if let Err(e) = screen.draw(|f| app.render(f)) {
      break Err(e.into());
    }
    tokio::select! {
      ev = events.recv() => {
        let Some(Ok(ev)) = ev else { break Ok(()) };
        let key = match ev {
          Event::Key(key) => key,
          // Some terminals paste nothing when the clipboard holds an
          // image: read it as Ctrl+V does.
          Event::Paste(text) if text.trim().is_empty() => {
            paste_clipboard(&mut app, &mut pasted);
            continue;
          }
          Event::Paste(text) => {
            app.paste(&text);
            continue;
          }
          // Taken at the loop's top (`Screen::sync`).
          Event::Resize(..) => continue,
          _ => continue,
        };
        match app.on_key(key) {
          Action::None => {}
          Action::Quit => break Ok(()),
          Action::Approve(decision) => {
            if let Some((_, answer)) = approval.take() {
              let _ = answer.send(decision);
            }
          }
          Action::Copy(text) => copy_to_clipboard(&text),
          Action::PasteClipboard => paste_clipboard(&mut app, &mut pasted),
          Action::Auth(action) => {
            let (tx, data) = (tx.clone(), store.data.clone());
            tokio::spawn(async move {
              let result = auth_action(&data, action, &tx).await.map_err(|e| e.to_string());
              let _ = tx.send(UiEvent::AuthDone(result));
            });
          }
          Action::Spawn { node, model, goal } => {
            let req = Request::Detach {
              goal,
              model,
              repo: None,
              cwd: std::env::current_dir().ok().map(|d| d.display().to_string()),
              parent: app.thread.clone(),
              budget_usd: None,
              budget_tokens: None,
              budget_minutes: None,
              max_steps: None,
              // It belongs where the thread it reports to does.
              global: app.place() == "global",
              node,
              // From the open conversation, when there is one.
              context: app.thread.as_ref().map(|_| "this".to_owned()),
              no_tools: false,
              tools: Vec::new(),
              skills: Vec::new(),
              keep: false,
              memory: false,
              read_only: false,
              scopes: Vec::new(),
            };
            side_request(&tx, &store, &exe, req);
          }
          Action::Fork { thread, upto, last, no_attachments, no_tools, summarize } => {
            let req = Request::Fork { thread, upto, last, no_attachments, no_tools, summarize };
            let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.clone());
            tokio::spawn(async move {
              match async { Client::connect_or_start(&data, &exe).await?.request(&req).await }.await {
                Ok(Reply::Forked(id)) => {
                  let _ = tx.send(UiEvent::Forked(id));
                }
                Ok(Reply::Error(e)) => {
                  let _ = tx.send(UiEvent::Status(format!("fork: {e}")));
                }
                Ok(other) => {
                  let _ = tx.send(UiEvent::Status(format!("fork: unexpected reply: {other:?}")));
                }
                Err(e) => {
                  let _ = tx.send(UiEvent::Status(format!("fork: {e}")));
                }
              }
            });
          }
          Action::Rename { thread, title } => {
            side_request(&tx, &store, &exe, Request::Rename { thread, title });
          }
          Action::ReadOnly { thread, on } => {
            let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.clone());
            tokio::spawn(async move {
              match read_only_of(&data, &exe, thread, Some(on)).await {
                Ok(now) => {
                  let _ = tx.send(UiEvent::Notice(if now {
                    "This thread is now read-only: its shells write only temporary directories, and \
                     only read-only commands run (from its next tool call)."
                      .into()
                  } else {
                    "This thread is now writable (from its next turn).".into()
                  }));
                  let _ = tx.send(UiEvent::ReadOnly(now));
                }
                Err(e) => {
                  let _ = tx.send(UiEvent::Notice(format!("Error: {e}")));
                }
              }
            });
          }
          Action::OpenApp(what) => {
            side_request(&tx, &store, &exe, Request::AppOpen { what });
          }
          Action::RunOn { thread, node } => {
            let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.clone());
            tokio::spawn(async move {
              let asked = async {
                Client::connect_or_start(&data, &exe)
                  .await?
                  .request(&Request::RunOn { thread: thread.clone(), node })
                  .await
              }
              .await;
              match asked {
                Ok(Reply::Lines(lines)) => {
                  let _ = tx.send(UiEvent::Notice(lines.join("\n")));
                  let _ = tx.send(UiEvent::RunsOn(runs_on_of(&data, &exe, thread).await));
                }
                Ok(Reply::Error(e)) => {
                  let _ = tx.send(UiEvent::Notice(format!("Error: {e}")));
                }
                Ok(other) => {
                  let _ = tx.send(UiEvent::Notice(format!("unexpected reply: {other:?}")));
                }
                Err(e) => {
                  let _ = tx.send(UiEvent::Notice(format!("Error: {e}")));
                }
              }
            });
          }
          Action::Pin { thread, on } => {
            side_request(&tx, &store, &exe, Request::Pin { thread, on });
          }
          Action::Press { thread, set, item } => {
            let req = Request::Press { thread, set, item };
            let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.clone());
            tokio::spawn(async move {
              // Its line and output come with the transcript the poller
              // follows; here only how it ended.
              let event = match async { Client::connect_or_start(&data, &exe).await?.request(&req).await }.await {
                Ok(Reply::Lines(l)) => {
                  let ok = l.first().and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                    .is_some_and(|v| v["ok"] == true);
                  UiEvent::Status(if ok { "▶ done".into() } else { "▶ failed: see the transcript".into() })
                }
                Ok(Reply::Error(e)) => UiEvent::Notice(format!("Error: {e}")),
                Ok(other) => UiEvent::Notice(format!("Unexpected reply: {other:?}")),
                Err(e) => UiEvent::Notice(format!("Error: {e}")),
              };
              let _ = tx.send(event);
            });
          }
          Action::Rewind { thread, upto, resend } => {
            let req = Request::Rewind { thread: thread.clone(), upto: upto.clone() };
            let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.clone());
            tokio::spawn(async move {
              let event = match async { Client::connect_or_start(&data, &exe).await?.request(&req).await }.await {
                Ok(Reply::Ok) => UiEvent::Rewound { thread, upto, resend },
                Ok(Reply::Error(e)) => UiEvent::Status(format!("rewind: {e}")),
                Ok(other) => UiEvent::Status(format!("rewind: unexpected reply: {other:?}")),
                Err(e) => UiEvent::Status(format!("rewind: {e}")),
              };
              let _ = tx.send(event);
            });
          }
          Action::DeletePreview(thread) => {
            let req = Request::DeleteThreads {
              threads: vec![thread.clone()],
              spawned_by: None,
              dry_run: true,
              from_thread: None,
            };
            let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.clone());
            tokio::spawn(async move {
              let event = match async { Client::connect_or_start(&data, &exe).await?.request(&req).await }.await {
                Ok(Reply::Lines(lines)) => UiEvent::DeletePreview(thread, lines),
                Ok(Reply::Error(e)) => UiEvent::Status(format!("delete: {e}")),
                Ok(other) => UiEvent::Status(format!("delete: unexpected reply: {other:?}")),
                Err(e) => UiEvent::Status(format!("delete: {e}")),
              };
              let _ = tx.send(event);
            });
          }
          Action::Delete(thread) => {
            let req = Request::DeleteThreads {
              threads: vec![thread],
              spawned_by: None,
              dry_run: false,
              from_thread: None,
            };
            let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.clone());
            tokio::spawn(async move {
              let event = match async { Client::connect_or_start(&data, &exe).await?.request(&req).await }.await {
                Ok(Reply::Lines(lines)) => UiEvent::Deleted(lines),
                Ok(Reply::Error(e)) => UiEvent::Status(format!("delete: {e}")),
                Ok(other) => UiEvent::Status(format!("delete: unexpected reply: {other:?}")),
                Err(e) => UiEvent::Status(format!("delete: {e}")),
              };
              let _ = tx.send(event);
            });
          }
          Action::Quote(thread) => {
            let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.clone());
            tokio::spawn(async move {
              match async { Client::connect_or_start(&data, &exe).await?.request(&Request::Quote { thread }).await }.await {
                Ok(Reply::Lines(lines)) if !lines.is_empty() => {
                  let _ = tx.send(UiEvent::Quoted(lines[0].clone()));
                }
                Ok(Reply::Error(e)) => {
                  let _ = tx.send(UiEvent::Status(format!("quote: {e}")));
                }
                Ok(other) => {
                  let _ = tx.send(UiEvent::Status(format!("quote: unexpected reply: {other:?}")));
                }
                Err(e) => {
                  let _ = tx.send(UiEvent::Status(format!("quote: {e}")));
                }
              }
            });
          }
          Action::Mode(m) => ask_mode(&tx, &store, &exe, Some(m)),
          Action::SendTo { thread, text } => {
            let req = Request::Send { thread, text, from: app.thread.clone(), keep: false, interject: false, scopes: Vec::new() };
            side_request(&tx, &store, &exe, req);
          }
          Action::Remember { thread, on } => {
            let cwd = std::env::current_dir().ok().map(|d| d.display().to_string());
            let (open, global) = (thread.clone(), app.global);
            let req = Request::Remember { thread: thread.unwrap_or_else(|| "last".into()), on, cwd };
            let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.clone());
            tokio::spawn(async move {
              let text = match async { Client::connect_or_start(&data, &exe).await?.request(&req).await }.await {
                Ok(Reply::Lines(lines)) => lines.join("\n"),
                Ok(Reply::Error(e)) => format!("Error: {e}"),
                Ok(other) => format!("Unexpected reply: {other:?}"),
                Err(e) => format!("Error: {e}"),
              };
              let _ = tx.send(UiEvent::Notice(text));
              let _ = tx.send(UiEvent::Memory(memory_of(&data, &exe, open, global).await));
            });
          }
          Action::Fetch(what) => {
            let skills = matches!(what, Fetch::Toolset { skills: true, .. });
            let (tx, data, exe, me) = (tx.clone(), store.data.clone(), exe.clone(), store.node.to_string());
            tokio::spawn(async move {
              let reply = async {
                let mut c = Client::connect_or_start(&data, &exe).await?;
                c.request(&match what {
                  Fetch::Models(node) => Request::Models { node },
                  Fetch::Peers => Request::Status,
                  Fetch::Toolset { thread, global, tools, skill_changes, .. } => Request::Toolset {
                    thread,
                    cwd: std::env::current_dir().ok().map(|d| d.display().to_string()),
                    global,
                    tools,
                    skills: skill_changes,
                  },
                  Fetch::Search { query, here } => Request::Search {
                    query,
                    here,
                    cwd: std::env::current_dir().ok().map(|d| d.display().to_string()),
                    limit: 50,
                  },
                }).await
              }
              .await;
              let (choices, note) = match reply {
                Ok(Reply::Models { models, problems, default }) => (
                  std::iter::once(Choice::new("default", "follow the synced/place default"))
                    .chain(std::iter::once(Choice::new("quick", "follow the quick model")))
                    .chain(models.into_iter().map(|m| {
                      let detail = if m == default { "default" } else { "" };
                      Choice::new(m, detail)
                    }))
                    .collect(),
                  problems.join(" · "),
                ),
                Ok(Reply::Status { peers, .. }) => {
                  let others: Vec<Choice> =
                    peers.into_iter().filter(|p| *p != me).map(|p| Choice::new(p, "")).collect();
                  let note = if others.is_empty() {
                    "no other machines yet: pair one with `stitch invite`".to_owned()
                  } else {
                    String::new()
                  };
                  (others, note)
                }
                Ok(Reply::Toolset { tools, skills: skill_items, .. }) => {
                  let items = if skills { skill_items } else { tools };
                  (items.into_iter().map(set_choice).collect(), "Enter: on or off · Esc: done".to_owned())
                }
                Ok(Reply::Hits(hits)) => {
                  let note = if hits.is_empty() { "nothing found".to_owned() } else { "Enter opens the thread".to_owned() };
                  let choices = hits
                    .into_iter()
                    .map(|h| Choice {
                      label: format!(
                        "{} · {} · {} {}",
                        app::clip(&h.title, 40),
                        h.at.get(..10).unwrap_or(&h.at),
                        h.sender,
                        h.msg
                      ),
                      value: h.thread,
                      detail: h.snippet,
                    })
                    .collect();
                  (choices, note)
                }
                Ok(Reply::Error(e)) => (Vec::new(), e),
                Ok(other) => (Vec::new(), format!("unexpected reply: {other:?}")),
                Err(e) => (Vec::new(), e.to_string()),
              };
              let _ = tx.send(UiEvent::Picked(choices, note));
            });
          }
          Action::Answer { question, decision } => {
            let req = Request::Answer {
              question,
              allow: decision.allow,
              reason: decision.reason,
              always: decision.always,
            };
            side_request(&tx, &store, &exe, req);
          }
          Action::Open(thread) => {
            if let Some(p) = &app.pending {
              if p.waiting {
                // Waiting for another turn: its connection is kept, so
                // the node still sends it (one closed takes a waiting
                // message back, as when the TUI quits); what was queued
                // is taken back, said below as not sent.
                if let Some(c) = &to_chat {
                  for q in &app.queued {
                    let _ = c.send(Request::Retract { id: q.id });
                  }
                }
                chat_task = None;
              } else if let Some(t) = chat_task.take() {
                // Its connection closes; the node goes on with the turn.
                t.abort();
              }
              to_chat = None;
              approval = None;
              chat_no += 1;
              app.let_go();
            }
            let transcript = thread.as_deref().map(|id| store.transcript(id)).unwrap_or_default();
            app.set_threads(store.threads(app.scope));
            let kept = thread.as_deref().map(|id| store.kept_for(id)).unwrap_or_default();
            app.family = thread.as_deref().and_then(|id| store.family(id));
            app.opened(thread, &transcript);
            app.out.extend(kept.into_iter().map(Printed::Notice));
            ask_memory(&tx, &store, &exe, app.thread.clone(), app.global);
            remember(&store, &app);
          }
          Action::ListThreads => app.set_threads(store.threads(app.scope)),
          Action::Children(thread) => {
            let children = store.children(&thread);
            if let Some(f) = app.family.as_mut().filter(|f| f.id == thread) {
              f.children = children.len();
            }
            app.show_children(children);
          }
          Action::Link(other) => {
            let text = match store.link(other.as_deref()) {
              Ok(text) => {
                app.here = store.here_label.clone();
                text
              }
              Err(e) => format!("Error: {e}"),
            };
            app.out.push(Printed::Notice(text));
            app.set_threads(store.threads(app.scope));
          }
          Action::Send { thread, text, attachments, model, global, tools, skills, memory } => {
            chat_no += 1;
            let (c, t) = start_chat(&tx, &store, &exe, chat_no, thread, text, attachments, model, global, tools, skills, memory);
            (to_chat, chat_task) = (Some(c), Some(t));
          }
          Action::Interject { id, text, attachments } => {
            if let Some(c) = &to_chat {
              let _ = c.send(Request::Interject { id, text, attachments });
            }
          }
          Action::Retract(id) => {
            if let Some(c) = &to_chat {
              let _ = c.send(Request::Retract { id });
            }
          }
          Action::Interrupt => {
            if let Some(c) = &to_chat {
              let _ = c.send(Request::Interrupt);
            }
          }
          Action::Jobs => ask_jobs(&tx, &store, &exe),
          Action::StopJob(id) => {
            let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.clone());
            tokio::spawn(async move {
              let reply = async {
                let mut c = Client::connect_or_start(&data, &exe).await?;
                let said = c.request(&Request::StopJob { id }).await?;
                // The list as it is after.
                let jobs = c.request(&Request::Jobs).await?;
                Ok::<_, stitch_node::Error>((said, jobs))
              }
              .await;
              match reply {
                Ok((Reply::Lines(lines), Reply::Jobs(jobs))) => {
                  let _ = tx.send(UiEvent::Status(lines.join(" · ")));
                  let _ = tx.send(UiEvent::Jobs(jobs));
                }
                Ok(other) => {
                  let _ = tx.send(UiEvent::Status(format!("unexpected reply: {other:?}")));
                }
                Err(e) => {
                  let _ = tx.send(UiEvent::Status(format!("stop: {e}")));
                }
              }
            });
          }
        }
      }
      Some(ev) = rx.recv() => match ev {
        UiEvent::AuthUrl(url) => {
          // Printed, so it can be selected and copied like any text.
          app.out.push(Printed::Notice(format!("Open this URL in your browser to sign in:\n{url}")));
          app.auth_text = "Waiting up to 10 minutes for the browser to come back. For a remote node, forward the callback port shown in redirect_uri.".into();
        }
        UiEvent::AuthCode { code, url } => {
          app.out.push(Printed::Notice(format!(
            "Enter the code {code} at {url} to sign in to GitHub."
          )));
          app.auth_text = format!("Enter {code} at {url}. Waiting for GitHub (15 minutes).");
        }
        UiEvent::AuthDone(result) => {
          app.auth_pending = false;
          app.auth_text = result.unwrap_or_else(|e| format!("Sign-in error: {e}"));
          app.status = "subscription status updated · /auth to view".into();
        }
        UiEvent::Reply(n, _) | UiEvent::Done(n, _) if n != chat_no => {}
        UiEvent::Approve { chat, .. } if chat != chat_no => {}
        UiEvent::Reply(_, Reply::Answered { id }) => {
          if approval.as_ref().is_some_and(|(a, _)| *a == id) {
            approval = None;
            app.unask();
          }
        }
        UiEvent::Reply(_, Reply::Waiting(false)) => {
          // The turn waited for ended: the rest of it, before this one's.
          app.set_threads(store.threads(app.scope));
          store.follow(&mut app, true);
          app.on_reply(&Reply::Waiting(false));
        }
        UiEvent::Reply(_, r) => app.on_reply(&r),
        UiEvent::Notice(text) => {
          app.status.clear();
          app.out.push(Printed::Notice(text));
          poller.ask(&app, &store.here, true);
        }
        UiEvent::Polled(ask, got) => {
          let moved = got.is_some();
          let current = match got {
            Some(got) => store.polled(&mut app, &ask, *got, &mut seen_reports),
            None => true,
          };
          if moved {
            mark_opened(&store, &exe, app.thread.as_deref());
          }
          poller.answered(&app, &store.here);
          if !current {
            // Asked about a view since left: about this one now.
            poller.ask(&app, &store.here, true);
          }
        }
        UiEvent::Approve { id, session, command, question, grant, answer, .. } => {
          approval = Some((id, answer));
          app.ask(session, command, question, grant);
        }
        UiEvent::Picked(choices, note) => app.filled(choices, note),
        UiEvent::Jobs(jobs) => app.jobs_listed(jobs),
        UiEvent::InFlight(thread, step) => {
          asking_in_flight = false;
          app.in_flight = step.map(|s| (thread, s));
        }
        UiEvent::Status(line) => app.status = line,
        UiEvent::Quoted(path) => {
          app.set_threads(store.threads(app.scope));
          app.opened(None, &[]);
          app.attachments.push(path);
          app.status = "the thread is attached, quoted: ask about it".into();
          ask_memory(&tx, &store, &exe, app.thread.clone(), app.global);
          remember(&store, &app);
        }
        UiEvent::Forked(id) => {
          let from = app.thread.clone();
          let transcript = store.transcript(&id);
          app.set_threads(store.threads(app.scope));
          app.opened(Some(id.clone()), &transcript);
          app.status.clear();
          app.out.push(Printed::Notice(format!(
            "Forked{}: this thread continues it; your next message is its first.",
            from.map(|f| format!(" from {f}")).unwrap_or_default()
          )));
          ask_memory(&tx, &store, &exe, app.thread.clone(), app.global);
          remember(&store, &app);
        }
        UiEvent::Rewound { thread, upto, resend } => {
          if app.thread.as_deref() == Some(thread.as_str()) {
            let transcript = store.transcript(&thread);
            app.set_threads(store.threads(app.scope));
            app.rewound(upto.as_deref(), &transcript);
            app.status = "rewound".into();
            if let Some(text) = resend
              && app.pending.is_none()
            {
              app.input = text;
              app.cursor = None;
              if let Action::Send { thread, text, attachments, model, global, tools, skills, memory } = app.submit() {
                chat_no += 1;
                let (c, t) = start_chat(&tx, &store, &exe, chat_no, thread, text, attachments, model, global, tools, skills, memory);
                (to_chat, chat_task) = (Some(c), Some(t));
              }
            }
          }
        }
        UiEvent::DeletePreview(thread, lines) => {
          let doomed: Vec<_> = lines.iter().map(|l| crate::doomed::parse(l)).collect();
          app.on_delete_preview(thread, &doomed);
        }
        UiEvent::Deleted(lines) => {
          let doomed: Vec<_> = lines.iter().map(|l| crate::doomed::parse(l)).collect();
          let ids: Vec<&str> = doomed.iter().map(|d| d.id.as_str()).collect();
          app.set_threads(store.threads(app.scope));
          let open_gone = app.thread.as_deref().is_some_and(|t| ids.contains(&t));
          // A conversation's parts share its title: said once.
          let mut titles: Vec<&str> = doomed.iter().map(|d| d.title.as_str()).collect();
          titles.dedup();
          app.status = format!("deleted: {}", titles.join(", "));
          if open_gone {
            app.opened(None, &[]);
            app.out.push(Printed::Notice(format!("Deleted the open thread: {}.", titles.join(", "))));
            ask_memory(&tx, &store, &exe, app.thread.clone(), app.global);
          }
        }
        UiEvent::EffectiveModel(thread, model, global, effective) => {
          if (thread, model, global) == (app.thread.clone(), app.model.clone(), app.global) {
            app.default_model = effective;
          }
        }
        UiEvent::Memory(state) => app.on_memory(state),
        UiEvent::ReadOnly(on) => app.read_only = on,
        UiEvent::RunsOn(node) => app.runs_on = node,
        UiEvent::Mode(mode, line) => {
          app.mode = mode;
          if let Some(line) = line {
            app.out.push(Printed::Notice(line));
          }
        }
        UiEvent::Done(_, result) => {
          // Gone into another turn, followed as it was written: what it
          // wrote last is printed as for any turn followed.
          let woven = matches!(&result, Ok(Reply::ChatDone { stop, .. }) if stop == "woven");
          let ended = match &result {
            Ok(Reply::ChatDone { thread, segment, .. }) if !woven => Some((thread.clone(), segment.clone())),
            _ => None,
          };
          let answered = app.on_done(result).is_some();
          ask_memory(&tx, &store, &exe, app.thread.clone(), app.global);
          app.set_threads(store.threads(app.scope));
          match ended {
            Some((thread, segment)) => store.chat_ended(&mut app, &thread, &segment),
            None => store.follow(&mut app, woven),
          }
          app.echo = None;
          if answered {
            remember(&store, &app);
            // What was typed while it worked goes now.
            if let Some(Action::Send { thread, text, attachments, model, global, tools, skills, memory }) = app.send_queued() {
              chat_no += 1;
              let (c, t) = start_chat(&tx, &store, &exe, chat_no, thread, text, attachments, model, global, tools, skills, memory);
              (to_chat, chat_task) = (Some(c), Some(t));
            }
          }
        }
      },
      _ = spin.tick(), if app.pending.is_some() || app.in_flight_call().is_some() => {}
      _ = tick.tick() => {
        ask_effective_model(&tx, &store, &exe, &app);
        if !asking_in_flight {
          asking_in_flight = ask_in_flight(&tx, &store, &app);
        }
        if app.overlay == app::Overlay::Jobs {
          ask_jobs(&tx, &store, &exe);
        }
        poller.ask(&app, &store.here, false);
      }
    }
  };
  let _ = screen.close();
  result
}

/// Sends a message to the node; what streams back comes as events, then
/// `Done`, each marked with `no`. What goes on the returned sender
/// meanwhile reaches the chat; aborting the task lets go of it (the node
/// goes on with it).
#[allow(clippy::too_many_arguments)]
fn start_chat(
  tx: &mpsc::UnboundedSender<UiEvent>,
  store: &Store,
  exe: &std::path::Path,
  no: u64,
  thread: Option<String>,
  text: String,
  attachments: Vec<String>,
  model: Option<String>,
  global: bool,
  tools: Vec<String>,
  skills: Vec<String>,
  memory: Option<bool>,
) -> (mpsc::UnboundedSender<Request>, tokio::task::JoinHandle<()>) {
  let (to_chat, outgoing) = mpsc::unbounded_channel();
  let cwd = std::env::current_dir()
    .ok()
    .map(|d| d.display().to_string());
  let req = Request::Chat {
    thread,
    text,
    attachments,
    model,
    repo: None,
    cwd,
    global,
    env: stitch_node::client::client_env(),
    tools,
    skills,
    place: None,
    memory,
    end_user: None,
    node: None,
    spoken: false,
    scopes: Vec::new(),
  };
  let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.to_owned());
  let task = tokio::spawn(async move {
    let result = async {
      let mut c = Client::connect_or_start(&data, &exe).await?;
      c.chat_with(&req, Some(outgoing), |r| match r {
        Reply::Approve {
          id,
          session,
          command,
          question,
          grant,
        } => {
          let (answer, decided) = tokio::sync::oneshot::channel();
          let _ = tx.send(UiEvent::Approve {
            chat: no,
            id: *id,
            session: session.clone(),
            command: command.clone(),
            question: question.clone(),
            grant: grant.clone(),
            answer,
          });
          Some(decided)
        }
        r => {
          let _ = tx.send(UiEvent::Reply(no, r.clone()));
          None
        }
      })
      .await
    }
    .await
    .map_err(|e| e.to_string());
    let _ = tx.send(UiEvent::Done(no, result));
  });
  (to_chat, task)
}

/// Whether `thread` is in memory (without one, whether a new thread here,
/// or global, would be: its place may keep it out), or `None` when memory
/// does not run on the node.
async fn memory_of(
  data: &std::path::Path,
  exe: &std::path::Path,
  thread: Option<String>,
  global: bool,
) -> Option<bool> {
  let mut c = Client::connect_or_start(data, exe).await.ok()?;
  let cwd = std::env::current_dir()
    .ok()
    .map(|d| d.display().to_string());
  match c
    .request(&Request::Memory {
      thread,
      cwd,
      global,
    })
    .await
    .ok()?
  {
    Reply::Memory {
      available: true,
      thread_on,
      ..
    } => Some(thread_on.unwrap_or(true)),
    _ => None,
  }
}

/// Asks the node for the call the open thread's turn runs now, when this
/// TUI's own turn does not run on it (a report's turn, a detached
/// thread's, another node's: `Request::InFlight`); the answer comes as
/// an event. Whether it asked.
fn ask_in_flight(tx: &mpsc::UnboundedSender<UiEvent>, store: &Store, app: &App) -> bool {
  let Some(thread) = app.thread.clone() else {
    return false;
  };
  if app.pending.as_ref().is_some_and(|p| !p.waiting) {
    return false;
  }
  let (tx, data) = (tx.clone(), store.data.clone());
  tokio::spawn(async move {
    let req = Request::InFlight {
      thread: thread.clone(),
    };
    // Never starts a node: none running, nothing runs.
    let reply = async { Client::connect(&data).await?.request(&req).await }.await;
    let step = match reply {
      Ok(Reply::Lines(l)) => l
        .first()
        .and_then(|j| stitch_node::chat::StepInFlight::from_json(j)),
      _ => None,
    };
    let _ = tx.send(UiEvent::InFlight(thread, step));
  });
  true
}

/// Asks the node what runs in the background; the list comes as an event.
fn ask_jobs(tx: &mpsc::UnboundedSender<UiEvent>, store: &Store, exe: &std::path::Path) {
  let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.to_owned());
  tokio::spawn(async move {
    let reply = async {
      Client::connect_or_start(&data, &exe)
        .await?
        .request(&Request::Jobs)
        .await
    }
    .await;
    let _ = tx.send(match reply {
      Ok(Reply::Jobs(jobs)) => UiEvent::Jobs(jobs),
      Ok(other) => UiEvent::Status(format!("unexpected reply: {other:?}")),
      Err(e) => UiEvent::Status(format!("jobs: {e}")),
    });
  });
}

/// Asks the node its approval mode (and, with `to`, sets it); the answer
/// comes back as an event, said only when set.
fn ask_mode(
  tx: &mpsc::UnboundedSender<UiEvent>,
  store: &Store,
  exe: &std::path::Path,
  to: Option<String>,
) {
  let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.to_owned());
  tokio::spawn(async move {
    let reply = async {
      Client::connect_or_start(&data, &exe)
        .await?
        .request(&Request::Mode {
          mode: to.clone(),
          cwd: std::env::current_dir()
            .ok()
            .map(|d| d.display().to_string()),
        })
        .await
    }
    .await;
    let event = match reply {
      Ok(Reply::Lines(lines)) => {
        let line = lines.join("\n");
        // The node's line starts with the mode (`auto: …`).
        let mode = match line.split(':').next().unwrap_or("") {
          m @ ("yolo" | "auto") => m.to_owned(),
          _ => "ask".to_owned(),
        };
        UiEvent::Mode(mode, to.map(|_| line))
      }
      Ok(Reply::Error(e)) => UiEvent::Notice(format!("Error: {e}")),
      Ok(other) => UiEvent::Notice(format!("Unexpected reply: {other:?}")),
      Err(e) => UiEvent::Notice(format!("Error: {e}")),
    };
    let _ = tx.send(event);
  });
}

/// Ask without listing providers: this must be cheap enough to refresh
/// after sync, and must use the same resolver as the next turn.
fn ask_effective_model(
  tx: &mpsc::UnboundedSender<UiEvent>,
  store: &Store,
  exe: &std::path::Path,
  app: &App,
) {
  let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.to_owned());
  let (thread, model, global) = (app.thread.clone(), app.model.clone(), app.global);
  tokio::spawn(async move {
    let request = Request::EffectiveModel {
      thread: thread.clone(),
      model: model.clone(),
      place: global.then(|| "global".into()),
      cwd: std::env::current_dir()
        .ok()
        .map(|d| d.display().to_string()),
    };
    if let Ok(mut c) = Client::connect_or_start(&data, &exe).await
      && let Ok(Reply::Lines(lines)) = c.request(&request).await
      && let Some(effective) = lines.into_iter().next()
    {
      let _ = tx.send(UiEvent::EffectiveModel(thread, model, global, effective));
    }
  });
}

/// Asks the node whether the open thread is in memory; the answer comes
/// back as an event.
fn ask_memory(
  tx: &mpsc::UnboundedSender<UiEvent>,
  store: &Store,
  exe: &std::path::Path,
  thread: Option<String>,
  global: bool,
) {
  let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.to_owned());
  tokio::spawn(async move {
    let _ = tx.send(UiEvent::Memory(
      memory_of(&data, &exe, thread.clone(), global).await,
    ));
    // Its mode too, for the status line.
    let _ = tx.send(UiEvent::ReadOnly(match thread.clone() {
      Some(t) => read_only_of(&data, &exe, t, None).await.unwrap_or(false),
      None => false,
    }));
    // And where it runs.
    let _ = tx.send(UiEvent::RunsOn(match thread {
      Some(t) => runs_on_of(&data, &exe, t).await,
      None => None,
    }));
  });
}

/// The node `thread` was set to run on (`/run-on`), if any.
async fn runs_on_of(
  data: &std::path::Path,
  exe: &std::path::Path,
  thread: String,
) -> Option<String> {
  let mut c = Client::connect_or_start(data, exe).await.ok()?;
  match c.request(&Request::RunsOn { thread }).await {
    Ok(Reply::Lines(lines)) => lines.into_iter().next(),
    _ => None,
  }
}

/// Whether `thread` is read-only, after switching it (`on`).
async fn read_only_of(
  data: &std::path::Path,
  exe: &std::path::Path,
  thread: String,
  on: Option<bool>,
) -> std::result::Result<bool, String> {
  let mut c = Client::connect_or_start(data, exe)
    .await
    .map_err(|e| e.to_string())?;
  match c.request(&Request::ReadOnly { thread, on }).await {
    Ok(Reply::Lines(lines)) => Ok(lines.first().is_some_and(|l| l == "read-only")),
    Ok(Reply::Error(e)) => Err(e),
    Ok(other) => Err(format!("unexpected reply: {other:?}")),
    Err(e) => Err(e.to_string()),
  }
}

/// Remembers the open thread as the last one opened here.
fn remember(store: &Store, app: &App) {
  if let Some(t) = app
    .thread
    .as_ref()
    .and_then(|id| app.threads.iter().find(|t| &t.id == id))
  {
    store.remember(t);
  }
}

/// Sends a one-shot request to the node; its result comes back as a notice.
/// Tells the node the user sees `thread`, each time what the TUI shows
/// changed (the node keeps it only when the thread moved since: loose
/// ends, design §2.10); quiet, best effort.
fn mark_opened(store: &Store, exe: &std::path::Path, thread: Option<&str>) {
  let Some(thread) = thread else { return };
  let (data, exe, thread) = (store.data.clone(), exe.to_owned(), thread.to_owned());
  tokio::spawn(async move {
    if let Ok(mut c) = Client::connect_or_start(&data, &exe).await {
      let _ = c.request(&Request::Opened { thread }).await;
    }
  });
}

fn side_request(
  tx: &mpsc::UnboundedSender<UiEvent>,
  store: &Store,
  exe: &std::path::Path,
  req: Request,
) {
  let (tx, data, exe) = (tx.clone(), store.data.clone(), exe.to_owned());
  tokio::spawn(async move {
    let text = match async {
      Client::connect_or_start(&data, &exe)
        .await?
        .request(&req)
        .await
    }
    .await
    {
      Ok(Reply::Detached(thread)) => {
        format!("Thread {thread} spawned; its report comes back here.")
      }
      Ok(Reply::Ok) => "Answered.".to_owned(),
      Ok(Reply::Lines(lines)) => lines.join("\n"),
      Ok(Reply::App {
        name,
        url: Some(url),
      }) => {
        let opened = tokio::task::spawn_blocking({
          let url = url.clone();
          move || crate::launch_browser(&url)
        })
        .await
        .unwrap_or(false);
        if opened {
          format!("Opened \"{name}\": {url}")
        } else {
          format!("\"{name}\" (no browser could be started here): {url}")
        }
      }
      Ok(Reply::App { name, url: None }) => {
        format!("\"{name}\": this node serves no apps to a browser ([apps] port = 0)")
      }
      Ok(Reply::Error(e)) => format!("Error: {e}"),
      Ok(other) => format!("Unexpected reply: {other:?}"),
      Err(e) => format!("Error: {e}"),
    };
    let _ = tx.send(UiEvent::Notice(text));
  });
}

async fn auth_action(
  data: &std::path::Path,
  action: AuthAction,
  tx: &mpsc::UnboundedSender<UiEvent>,
) -> stitch_node::Result<String> {
  use stitch_node::{auth, keys::Store};
  use stitch_providers::oauth::Service;
  match action {
    AuthAction::Login { provider, file } if provider == "github" => {
      let store = if file { Store::File } else { Store::Keychain };
      let account = stitch_node::github::login(data, store, |p| {
        let _ = tx.send(UiEvent::AuthCode {
          code: p.user_code.clone(),
          url: p.verification_uri.clone(),
        });
      })
      .await?;
      Ok(format!("Signed in to GitHub as {}.", account.login))
    }
    AuthAction::Logout { provider } if provider == "github" => {
      stitch_node::github::logout(data)?;
      Ok("Signed out of GitHub here. GitHub keeps the authorization until it is revoked in its settings (Applications).".into())
    }
    AuthAction::Login { provider, file } => {
      let service = Service::parse(&provider)?;
      let store = auth::login(
        data,
        service,
        if file { Store::File } else { Store::Keychain },
        |url| {
          let _ = tx.send(UiEvent::AuthUrl(url.into()));
        },
      )
      .await?;
      Ok(format!(
        "Signed in to {} ({}).\n\nExplicit API keys take precedence. Remove a stored key with stitch key rm {} and unset any environment key to use this subscription.\n\nUse /model {}/<model> to select a model, then send a message.",
        service.name(),
        if store == Store::File {
          "private file"
        } else {
          "keychain"
        },
        service.name(),
        service.name()
      ))
    }
    AuthAction::Logout { provider } => {
      let service = Service::parse(&provider)?;
      auth::logout(data, service).await?;
      Ok(format!(
        "Signed out of {}. API keys are unchanged.",
        service.name()
      ))
    }
    AuthAction::Status { provider } => {
      let services = match provider.as_deref() {
        Some("github") => vec![],
        Some(p) => vec![Service::parse(p)?],
        None => vec![Service::Anthropic, Service::OpenAi, Service::Grok],
      };
      let mut lines = Vec::new();
      for service in services {
        lines.push(format!(
          "{}: {}",
          service.name(),
          auth::status(data, service).await?
        ));
      }
      if matches!(provider.as_deref(), None | Some("github")) {
        lines.push(format!("github: {}", stitch_node::github::status(data)));
      }
      lines.push("\n/login <provider> [--file] · /logout <provider>".into());
      Ok(lines.join("\n"))
    }
  }
}

/// A tool or skill set's item as a picker's choice: its value is the
/// change Enter makes (`-x` turns an item that is on off).
fn set_choice(i: stitch_node::api::SetItem) -> Choice {
  let mut detail = i.note.clone();
  if let Some(n) = &i.node {
    detail = format!("on {n} · {detail}");
  }
  if i.on && !i.available {
    detail = format!("unavailable here · {detail}");
  }
  Choice {
    value: format!("{}{}", if i.on { '-' } else { '+' }, i.name),
    label: format!("{} {}", if i.on { "[x]" } else { "[ ]" }, i.name),
    detail,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use stitch_records::{Author, Draft, Ref, kinds};

  /// A thread's parent: who spawned it, else whom it reports to, the
  /// newest compaction of that one.
  #[test]
  fn a_threads_parent_is_who_spawned_it_followed_through_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let node: NodeName = "mbp".parse().unwrap();
    let root = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      Role::Home,
    )
    .unwrap();
    let thread = |refs: &[(&str, &RecordId)], compacted: bool| {
      let mut d = Draft::new(kinds::THREAD, Author::user(node.clone()))
        .unwrap()
        .body("# a thread\n");
      for (rel, to) in refs {
        d = d.reference(rel, Ref::to((*to).clone())).unwrap();
      }
      if compacted {
        d = d.field("compacted", true).unwrap();
      }
      root.publish(&node, d).unwrap().id
    };
    let top = thread(&[], false);
    let spawned = thread(&[("spawned-by", &top)], false);
    let reporting = thread(&[("reports-to", &spawned)], false);
    let fork = thread(&[("continues", &spawned)], false);
    let mut index = Index::open(&dir.path().join("index.sqlite")).unwrap();
    index.refresh(&root).unwrap();
    assert_eq!(parent_of(&index, &top), None);
    assert_eq!(parent_of(&index, &spawned), Some(top.clone()));
    assert_eq!(parent_of(&index, &reporting), Some(spawned.clone()));
    assert_eq!(parent_of(&index, &fork), None, "a fork is not a child");
    // The top thread is compacted, twice: its child goes to the newest.
    let once = thread(&[("continues", &top)], true);
    let twice = thread(&[("continues", &once)], true);
    index.refresh(&root).unwrap();
    assert_eq!(parent_of(&index, &spawned), Some(twice.clone()));
    // Below: what it spawned, whichever version of it did, newest first;
    // a fork of a child is not one of them.
    let later = thread(&[("spawned-by", &once)], false);
    index.refresh(&root).unwrap();
    let ids = |v: Vec<stitch_store::IndexedRecord>| v.into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(
      ids(children_of(&index, &twice)),
      vec![later, spawned.clone()]
    );
    assert_eq!(ids(children_of(&index, &spawned)), vec![reporting.clone()]);
    // A child compacted is listed once, as its newest.
    let reporting2 = thread(&[("continues", &reporting), ("reports-to", &spawned)], true);
    index.refresh(&root).unwrap();
    assert_eq!(ids(children_of(&index, &spawned)), vec![reporting2]);
    assert!(children_of(&index, &fork).is_empty());
  }

  #[test]
  fn a_view_shows_only_its_own_reports() {
    let dir = tempfile::tempdir().unwrap();
    let node: NodeName = "mbp".parse().unwrap();
    let root = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      Role::Home,
    )
    .unwrap();
    let thread = |folder: Option<&str>| {
      let mut d = Draft::new(kinds::THREAD, Author::user(node.clone()))
        .unwrap()
        .body("# a thread\n");
      if let Some(path) = folder {
        let mut f = toml::Table::new();
        f.insert("node".into(), "mbp".into());
        f.insert("path".into(), path.into());
        d = d.field("folder", toml::Value::Table(f)).unwrap();
      }
      root.publish(&node, d).unwrap().id
    };
    let report = |of: &RecordId, to: Option<&RecordId>| {
      let mut d = Draft::new(kinds::REPORT, Author::node(node.clone()))
        .unwrap()
        .reference("thread", Ref::to(of.clone()))
        .unwrap()
        .body("# done\n");
      if let Some(t) = to {
        d = d.reference("reports-to", Ref::to(t.clone())).unwrap();
      }
      root.publish(&node, d).unwrap().id.to_string()
    };
    let parent = thread(Some("~/work/management"));
    let elsewhere = thread(Some("~/work/management"));
    let mine = thread(None);
    // To a thread in another folder: that thread's.
    let to_other = report(&elsewhere, Some(&parent));
    // To nobody, from a thread here (no folder: global, as this view).
    let unaddressed_here = report(&mine, None);
    // To nobody, from a thread in another folder.
    let unaddressed_there = report(&elsewhere, None);
    let mut index = Index::open(&dir.path().join("index.sqlite")).unwrap();
    index.refresh(&root).unwrap();
    let store = Store {
      data: dir.path().to_owned(),
      node,
      here: Place::Global,
      here_label: "global".into(),
      index: Mutex::new(index),
      blobs: BlobStore::open(&dir.path().join("blobs")).unwrap(),
    };
    let shown: std::collections::HashMap<String, bool> = store.reports(None).into_iter().collect();
    assert!(!shown[&to_other], "another thread's report");
    assert!(shown[&unaddressed_here], "one of this place's, to nobody");
    assert!(!shown[&unaddressed_there], "another place's");
    // Kept for the user (it woke nothing): shown in the thread it went to,
    // while that thread is open, and when it is opened.
    let kept = {
      let d = Draft::new(kinds::REPORT, Author::node(store.node.clone()))
        .unwrap()
        .field("wake", false)
        .unwrap()
        .reference("thread", Ref::to(elsewhere.clone()))
        .unwrap()
        .reference("reports-to", Ref::to(parent.clone()))
        .unwrap()
        .body("# kept for you\n\nthe answer\n");
      root.publish(&store.node, d).unwrap().id.to_string()
    };
    store.index.lock().unwrap().refresh(&root).unwrap();
    let open_parent: std::collections::HashMap<String, bool> =
      store.reports(Some(parent.as_str())).into_iter().collect();
    assert!(open_parent[&kept], "kept, in the thread it went to");
    assert!(
      !open_parent[&to_other],
      "one that woke it is in its transcript"
    );
    let open_other: std::collections::HashMap<String, bool> =
      store.reports(Some(mine.as_str())).into_iter().collect();
    assert!(!open_other[&kept], "not in another thread");
    let on_open = store.kept_for(parent.as_str());
    assert_eq!(on_open.len(), 1, "{on_open:?}");
    assert!(on_open[0].contains("the answer"), "{on_open:?}");
  }

  #[test]
  fn a_place_with_no_threads_of_its_own_reopens_none() {
    let dir = tempfile::tempdir().unwrap();
    let node: NodeName = "mbp".parse().unwrap();
    let root = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      Role::Home,
    )
    .unwrap();
    let thread = |folder: Option<&str>| {
      let mut d = Draft::new(kinds::THREAD, Author::user(node.clone()))
        .unwrap()
        .body("# a thread\n");
      if let Some(path) = folder {
        let mut f = toml::Table::new();
        f.insert("node".into(), "mbp".into());
        f.insert("path".into(), path.into());
        d = d.field("folder", toml::Value::Table(f)).unwrap();
      }
      root.publish(&node, d).unwrap().id.to_string()
    };
    let global = thread(None);
    let notes = thread(Some("~/notes"));
    let mut index = Index::open(&dir.path().join("index.sqlite")).unwrap();
    index.refresh(&root).unwrap();
    let mut store = Store {
      data: dir.path().to_owned(),
      node,
      here: Place::Global,
      here_label: "global".into(),
      index: Mutex::new(index),
      blobs: BlobStore::open(&dir.path().join("blobs")).unwrap(),
    };
    assert_eq!(store.reopen(), Some(global), "global: its own");
    store.here = Place::Folder("~/notes".into());
    assert_eq!(store.reopen(), Some(notes), "a folder: its own");
    store.here = Place::Folder("~/fresh".into());
    assert_eq!(store.reopen(), None, "a fresh folder: no global one");
  }

  #[test]
  fn what_a_thread_gained_after_its_chat_ended_is_printed() {
    use stitch_records::{Event, Msg, Part, Role as Who};
    let dir = tempfile::tempdir().unwrap();
    let node: NodeName = "mbp".parse().unwrap();
    let root = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      Role::Home,
    )
    .unwrap();
    let thread = root
      .publish(
        &node,
        Draft::new(kinds::THREAD, Author::user(node.clone()))
          .unwrap()
          .body("# a thread\n"),
      )
      .unwrap()
      .id;
    let msg = |id: &str, role: Who, text: &str| {
      Event::Msg(Msg {
        id: id.into(),
        at: jiff::Timestamp::now(),
        role,
        from: None,
        model: (role == Who::Assistant).then(|| "m".into()),
        parts: vec![Part::Text { text: text.into() }],
        native: None,
      })
    };
    // The chat's own step, then a report's turn woken as it ended.
    let chat = root
      .publish_segment(
        &node,
        &thread,
        None,
        None,
        vec![
          msg("m1", Who::User, "hello"),
          msg("m2", Who::Assistant, "hi"),
        ],
      )
      .unwrap();
    root
      .publish_segment(
        &node,
        &thread,
        Some(chat.header.segment.clone()),
        None,
        vec![
          msg("m3", Who::User, "[Report r from mbp]\nthe child's answer"),
          msg("m4", Who::Assistant, "noted the report"),
        ],
      )
      .unwrap();
    let mut index = Index::open(&dir.path().join("index.sqlite")).unwrap();
    index.refresh(&root).unwrap();
    let store = Store {
      data: dir.path().to_owned(),
      node,
      here: Place::Global,
      here_label: "global".into(),
      index: Mutex::new(index),
      blobs: BlobStore::open(&dir.path().join("blobs")).unwrap(),
    };
    let mut app = App::new("mbp", "m");
    app.thread = Some(thread.to_string());
    store.chat_ended(&mut app, thread.as_str(), chat.header.segment.as_str());
    // The chat's own messages were shown live; the report and its reply
    // were not, and are printed now.
    let printed: Vec<String> = app.out.iter().map(|p| format!("{p:?}")).collect();
    assert_eq!(printed.len(), 2, "{printed:?}");
    assert!(printed[0].contains("the child's answer"), "{printed:?}");
    assert!(printed[1].contains("noted the report"), "{printed:?}");
  }

  /// The open thread is followed though the thread list does not have it
  /// (another place's, opened from the picker's All, or one whose folder
  /// became a git checkout while the TUI ran, 2026-10-30): a turn the node
  /// ran on it (an app's `stitch.report`), its segments sealed between two
  /// polls, is printed, not missed till the TUI restarts.
  #[test]
  fn a_turn_on_an_open_thread_of_another_place_is_followed() {
    use stitch_records::{Event, Msg, Part, Role as Who};
    let dir = tempfile::tempdir().unwrap();
    let node: NodeName = "mbp".parse().unwrap();
    let root = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      Role::Home,
    )
    .unwrap();
    let mut f = toml::Table::new();
    f.insert("node".into(), "mbp".into());
    f.insert("path".into(), "~/Games/cp2077".into());
    let thread = root
      .publish(
        &node,
        Draft::new(kinds::THREAD, Author::user(node.clone()))
          .unwrap()
          .field("folder", toml::Value::Table(f))
          .unwrap()
          .body("# a thread\n"),
      )
      .unwrap()
      .id;
    let msg = |id: &str, role: Who, text: &str| {
      Event::Msg(Msg {
        id: id.into(),
        at: jiff::Timestamp::now(),
        role,
        from: None,
        model: (role == Who::Assistant).then(|| "m".into()),
        parts: vec![Part::Text { text: text.into() }],
        native: None,
      })
    };
    let first = root
      .publish_segment(
        &node,
        &thread,
        None,
        None,
        vec![
          msg("m1", Who::User, "hello"),
          msg("m2", Who::Assistant, "hi"),
        ],
      )
      .unwrap();
    let mut index = Index::open(&dir.path().join("index.sqlite")).unwrap();
    index.refresh(&root).unwrap();
    let store = Store {
      data: dir.path().to_owned(),
      node: node.clone(),
      here: Place::Global,
      here_label: "global".into(),
      index: Mutex::new(index),
      blobs: BlobStore::open(&dir.path().join("blobs")).unwrap(),
    };
    let mut app = App::new("mbp", "m");
    app.threads = store.threads(app.scope);
    assert!(
      app.threads.iter().all(|t| t.id != thread.as_str()),
      "another place's thread is not listed here"
    );
    app.opened(Some(thread.to_string()), &store.transcript(thread.as_str()));
    let mut seen = std::collections::HashSet::new();
    store.tick(&mut app, &mut seen);
    app.out.clear();
    // The turn an app's button started, run by the node and sealed.
    std::thread::sleep(std::time::Duration::from_millis(5));
    root
      .publish_segment(
        &node,
        &thread,
        Some(first.header.segment.clone()),
        None,
        vec![
          msg("m3", Who::User, "[App \"NC Mods\" sent you data]\n{}"),
          msg("m4", Who::Assistant, "no updates"),
        ],
      )
      .unwrap();
    store.index.lock().unwrap().refresh(&root).unwrap();
    store.tick(&mut app, &mut seen);
    let printed: Vec<String> = app.out.iter().map(|p| format!("{p:?}")).collect();
    assert!(
      printed.iter().any(|p| p.contains("sent you data")),
      "{printed:?}"
    );
    assert!(
      printed.iter().any(|p| p.contains("no updates")),
      "{printed:?}"
    );
  }

  /// A thread compacted by a turn the node ran (not this TUI's chat) is
  /// one conversation (design §2.15): the poll switches to the newest and
  /// prints the divider and what came since, not the old messages again;
  /// the picker lists the chain once, as its newest, with the old one's
  /// pin and its title without the ✂.
  #[test]
  fn a_thread_compacted_meanwhile_goes_on_as_one_conversation() {
    use stitch_records::{Event, Msg, Part, Role as Who};
    let dir = tempfile::tempdir().unwrap();
    let node: NodeName = "mbp".parse().unwrap();
    let root = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      Role::Home,
    )
    .unwrap();
    let old = root
      .publish(
        &node,
        Draft::new(kinds::THREAD, Author::user(node.clone()))
          .unwrap()
          .body("# Long talk\n"),
      )
      .unwrap()
      .id;
    let msg = |id: &str, role: Who, text: &str| {
      Event::Msg(Msg {
        id: id.into(),
        at: jiff::Timestamp::now(),
        role,
        from: None,
        model: (role == Who::Assistant).then(|| "m".into()),
        parts: vec![Part::Text { text: text.into() }],
        native: None,
      })
    };
    let first = root
      .publish_segment(
        &node,
        &old,
        None,
        None,
        vec![
          msg("m1", Who::User, "hello"),
          msg("m2", Who::Assistant, "hi"),
        ],
      )
      .unwrap();
    // Pinned before it is compacted.
    root
      .publish(
        &node,
        Draft::new(kinds::SET, Author::user(node.clone()))
          .unwrap()
          .field("field", "pinned")
          .unwrap()
          .field("value", true)
          .unwrap()
          .reference("target", Ref::to(old.clone()))
          .unwrap()
          .reference("supersedes", Ref::to(old.clone()))
          .unwrap(),
      )
      .unwrap();
    let mut index = Index::open(&dir.path().join("index.sqlite")).unwrap();
    index.refresh(&root).unwrap();
    let store = Store {
      data: dir.path().to_owned(),
      node: node.clone(),
      here: Place::Global,
      here_label: "global".into(),
      index: Mutex::new(index),
      blobs: BlobStore::open(&dir.path().join("blobs")).unwrap(),
    };
    let mut app = App::new("mbp", "m");
    app.threads = store.threads(app.scope);
    app.opened(Some(old.to_string()), &store.transcript(old.as_str()));
    let mut seen = std::collections::HashSet::new();
    store.tick(&mut app, &mut seen);
    app.out.clear();
    // A report's turn compacted it, and went on in the new thread.
    std::thread::sleep(std::time::Duration::from_millis(5));
    let new = root
      .publish(
        &node,
        Draft::new(kinds::THREAD, Author::user(node.clone()))
          .unwrap()
          .body("# ✂ Long talk\n")
          .reference("continues", Ref::to(old.clone()))
          .unwrap()
          .field("continues_at", first.header.segment.as_str())
          .unwrap()
          .field("continues_last", 2)
          .unwrap()
          .field("summary", "SUMMARY-LINE")
          .unwrap()
          .field("compacted", true)
          .unwrap()
          .field("compacted_model", "anthropic/m")
          .unwrap()
          .field("compacted_window", 200_000)
          .unwrap()
          .field("compacted_tokens", 162_000)
          .unwrap(),
      )
      .unwrap()
      .id;
    root
      .publish_segment(
        &node,
        &new,
        None,
        None,
        vec![
          msg("m1", Who::User, "[Report r from mbp]\nthe child's answer"),
          msg("m2", Who::Assistant, "noted"),
        ],
      )
      .unwrap();
    store.index.lock().unwrap().refresh(&root).unwrap();
    store.tick(&mut app, &mut seen);
    assert_eq!(app.thread.as_deref(), Some(new.as_str()), "switched");
    assert_eq!(
      app.out.first(),
      Some(&Printed::Compacted {
        line: "compacted here (162k of 200k window): summary + last 2 messages".into(),
        summary: "SUMMARY-LINE".into(),
      }),
      "{:?}",
      app.out
    );
    let printed: Vec<String> = app.out.iter().map(|p| format!("{p:?}")).collect();
    assert_eq!(printed.len(), 3, "{printed:?}");
    assert!(printed[1].contains("the child's answer"), "{printed:?}");
    assert!(printed[2].contains("noted"), "{printed:?}");
    // Its earlier messages are not the newest's: the picker leaves them.
    assert_eq!(app.pickable(), vec![3, 4], "{:?}", app.entries);
    // One row, the newest, pinned, titled as the conversation.
    let rows = store.threads(ThreadScope::All);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].id, new.as_str());
    assert!(rows[0].pinned);
    assert_eq!(rows[0].title, "Long talk");
    // Opened again, the whole conversation, the divider between.
    let whole = store.transcript(new.as_str());
    let who: Vec<&str> = whole.iter().map(|e| e.who.as_str()).collect();
    assert_eq!(who, ["you", "m", "compacted", "you", "m"], "{whole:?}");
  }

  /// A message sent while another turn runs on the thread (a report's, a
  /// `send` from another thread) waits for it, and is woven into it:
  /// printed when sent, it is not printed again as that turn is followed;
  /// what was queued with it, not printed yet, is.
  #[test]
  fn a_message_woven_into_a_turn_followed_prints_once() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use stitch_records::{Event, Msg, Part, Role as Who};
    let dir = tempfile::tempdir().unwrap();
    let node: NodeName = "mbp".parse().unwrap();
    let root = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      Role::Home,
    )
    .unwrap();
    let thread = root
      .publish(
        &node,
        Draft::new(kinds::THREAD, Author::user(node.clone()))
          .unwrap()
          .body("# a thread\n"),
      )
      .unwrap()
      .id;
    let msg = |id: &str, role: Who, from: Option<&str>, text: &str| {
      Event::Msg(Msg {
        id: id.into(),
        at: jiff::Timestamp::now(),
        role,
        from: from.map(str::to_owned),
        model: (role == Who::Assistant).then(|| "m".into()),
        parts: vec![Part::Text { text: text.into() }],
        native: None,
      })
    };
    let first = root
      .publish_segment(
        &node,
        &thread,
        None,
        None,
        vec![
          msg("m1", Who::User, None, "hello"),
          msg("m2", Who::Assistant, None, "hi"),
        ],
      )
      .unwrap();
    let mut index = Index::open(&dir.path().join("index.sqlite")).unwrap();
    index.refresh(&root).unwrap();
    let store = Store {
      data: dir.path().to_owned(),
      node: node.clone(),
      here: Place::Global,
      here_label: "global".into(),
      index: Mutex::new(index),
      blobs: BlobStore::open(&dir.path().join("blobs")).unwrap(),
    };
    let mut app = App::new("mbp", "m");
    app.threads = store.threads(ThreadScope::All);
    app.opened(Some(thread.to_string()), &store.transcript(thread.as_str()));
    app.out.clear();
    // Sent while a report's turn runs: printed, then it waits.
    app.paste("what now?");
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.on_reply(&Reply::Waiting(true));
    // The turn that runs: the report, then the message with one queued
    // after it, woven in as one, and its answer.
    root
      .publish_segment(
        &node,
        &thread,
        Some(first.header.segment.clone()),
        None,
        vec![
          msg(
            "m3",
            Who::User,
            Some("report"),
            "[Report r]\nthe child's answer",
          ),
          msg("m4", Who::Assistant, None, "noted"),
          msg("m5", Who::User, Some("user@mbp"), "what now?\n\nand this"),
          msg("m6", Who::Assistant, None, "this, now"),
        ],
      )
      .unwrap();
    store.index.lock().unwrap().refresh(&root).unwrap();
    app.threads = store.threads(ThreadScope::All);
    store.follow(&mut app, true);
    app.on_reply(&Reply::Woven(vec![0]));
    let printed: Vec<String> = app.out.iter().map(|p| format!("{p:?}")).collect();
    assert_eq!(
      printed.iter().filter(|p| p.contains("what now?")).count(),
      1,
      "{printed:?}"
    );
    assert!(
      printed.iter().any(|p| p.contains("and this")),
      "{printed:?}"
    );
    assert!(
      printed.iter().any(|p| p.contains("this, now")),
      "{printed:?}"
    );
  }

  /// A fork (the message picker's `f` and `e`, `/fork`, a compaction)
  /// opens with its notice, not its parent's messages, which are above;
  /// a chat on it that ends prints nothing it showed live again. Its
  /// transcript up to a segment counted without the notice, so the last
  /// entry of each turn was printed twice.
  #[test]
  fn a_chat_on_a_fork_prints_each_message_once() {
    use stitch_node::chat::{Continues, Shape, Upto};
    use stitch_records::{Event, Msg, Part, Role as Who};
    let dir = tempfile::tempdir().unwrap();
    let node: NodeName = "mbp".parse().unwrap();
    let root = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      Role::Home,
    )
    .unwrap();
    let draft = || Draft::new(kinds::THREAD, Author::user(node.clone())).unwrap();
    let parent = root.publish(&node, draft().body("# parent\n")).unwrap().id;
    let msg = |id: &str, role: Who, text: &str| {
      Event::Msg(Msg {
        id: id.into(),
        at: jiff::Timestamp::now(),
        role,
        from: None,
        model: (role == Who::Assistant).then(|| "m".into()),
        parts: vec![Part::Text { text: text.into() }],
        native: None,
      })
    };
    let seg = root
      .publish_segment(
        &node,
        &parent,
        None,
        None,
        vec![
          msg("m1", Who::User, "hello"),
          msg("m2", Who::Assistant, "hi"),
          msg("m3", Who::User, "and then?"),
          msg("m4", Who::Assistant, "then this"),
        ],
      )
      .unwrap();
    // Forked up to m2: the picker's `f` on "hi", or `e` on "and then?".
    let continues = Continues {
      thread: parent.clone(),
      at: Upto::Segment(seg.header.segment.clone()),
      shape: Shape {
        upto: Some("m2".into()),
        ..Shape::default()
      },
    };
    let fork = root
      .publish(
        &node,
        continues.write(draft().body("# ⑂ parent\n")).unwrap(),
      )
      .unwrap()
      .id;
    let mut index = Index::open(&dir.path().join("index.sqlite")).unwrap();
    index.refresh(&root).unwrap();
    let store = Store {
      data: dir.path().to_owned(),
      node: node.clone(),
      here: Place::Global,
      here_label: "global".into(),
      index: Mutex::new(index),
      blobs: BlobStore::open(&dir.path().join("blobs")).unwrap(),
    };
    let mut app = App::new("mbp", "m");
    app.threads = store.threads(ThreadScope::All);
    app.opened(Some(parent.to_string()), &store.transcript(parent.as_str()));
    app.out.clear();
    // Forked: its separator and notice.
    app.opened(Some(fork.to_string()), &store.transcript(fork.as_str()));
    let shown: Vec<String> = app.out.iter().map(|p| format!("{p:?}")).collect();
    assert_eq!(shown.len(), 2, "{shown:?}");
    assert!(
      shown[1].contains("continues thread") && shown[1].contains("up to m2"),
      "{shown:?}"
    );
    app.out.clear();
    // The edited message is sent and answered, live; the chat ends.
    let turn = root
      .publish_segment(
        &node,
        &fork,
        None,
        None,
        vec![
          msg("m1", Who::User, "and else?"),
          msg("m2", Who::Assistant, "else that"),
        ],
      )
      .unwrap();
    store.index.lock().unwrap().refresh(&root).unwrap();
    store.chat_ended(&mut app, fork.as_str(), turn.header.segment.as_str());
    assert_eq!(app.out, Vec::new(), "shown live already");
    // The next turn ends the same way; a report's turn after it prints.
    let next = root
      .publish_segment(
        &node,
        &fork,
        Some(turn.header.segment.clone()),
        None,
        vec![
          msg("m3", Who::User, "more"),
          msg("m4", Who::Assistant, "more said"),
        ],
      )
      .unwrap();
    root
      .publish_segment(
        &node,
        &fork,
        Some(next.header.segment.clone()),
        None,
        vec![
          msg("m5", Who::User, "[Report r from mbp]\nthe child's answer"),
          msg("m6", Who::Assistant, "noted"),
        ],
      )
      .unwrap();
    store.index.lock().unwrap().refresh(&root).unwrap();
    store.chat_ended(&mut app, fork.as_str(), next.header.segment.as_str());
    let printed: Vec<String> = app.out.iter().map(|p| format!("{p:?}")).collect();
    assert_eq!(printed.len(), 2, "{printed:?}");
    assert!(printed[0].contains("the child's answer"), "{printed:?}");
    assert!(printed[1].contains("noted"), "{printed:?}");
  }

  /// The poller reads again only what changed: nothing written, nothing
  /// read; a record the node indexed, or a turn's step in the open
  /// segment, and it reads.
  #[test]
  fn the_poller_reads_only_when_something_changed() {
    use stitch_records::{Event, Msg, Part, Role as Who};
    let dir = tempfile::tempdir().unwrap();
    let node: NodeName = "mbp".parse().unwrap();
    let root = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      Role::Home,
    )
    .unwrap();
    let thread = |title: &str| {
      let d = Draft::new(kinds::THREAD, Author::user(node.clone()))
        .unwrap()
        .body(format!("# {title}\n"));
      root.publish(&node, d).unwrap().id
    };
    let first = thread("first");
    // The node's own connection, which indexes what it writes.
    let mut writer = Index::open(&dir.path().join("index.sqlite")).unwrap();
    writer.refresh(&root).unwrap();
    let store = Store {
      data: dir.path().to_owned(),
      node: node.clone(),
      here: Place::Global,
      here_label: "global".into(),
      index: Mutex::new(Index::open(&dir.path().join("index.sqlite")).unwrap()),
      blobs: BlobStore::open(&dir.path().join("blobs")).unwrap(),
    };
    let (tx, mut rx) = mpsc::unbounded_channel();
    let ask = spawn_poller(store.reader().unwrap(), tx);
    let mut app = App::new("mbp", "m");
    app.thread = Some(first.to_string());
    let mut poll = |app: &App, force: bool| {
      ask.send(Poll::of(app, &Place::Global, force)).unwrap();
      match rx.blocking_recv().unwrap() {
        UiEvent::Polled(_, got) => got,
        _ => panic!("not a poll's answer"),
      }
    };
    let got = poll(&app, false).expect("the first poll reads");
    assert_eq!(got.threads.len(), 1);
    assert!(poll(&app, false).is_none(), "nothing changed");
    assert!(poll(&app, true).is_some(), "forced");
    // Another thread, indexed by the node.
    thread("second");
    writer.refresh(&root).unwrap();
    let got = poll(&app, false).expect("a record was indexed");
    assert_eq!(got.threads.len(), 2);
    assert!(poll(&app, false).is_none(), "nothing changed since");
    // A refresh that finds nothing new writes nothing.
    writer.refresh(&root).unwrap();
    assert!(poll(&app, false).is_none(), "an empty refresh is no change");
    // A step of a turn, in the open segment (not indexed till sealed).
    std::thread::sleep(Duration::from_millis(20));
    root
      .append_step(
        &node,
        &first,
        None,
        vec![Event::Msg(Msg {
          id: "m1".into(),
          at: jiff::Timestamp::now(),
          role: Who::User,
          from: None,
          model: None,
          parts: vec![Part::Text {
            text: "hello".into(),
          }],
          native: None,
        })],
      )
      .unwrap();
    let got = poll(&app, false).expect("the open segment moved");
    let (_, transcript) = got.follow.expect("the open thread is read again");
    assert_eq!(transcript.len(), 1);
    // Another view (the thread list's scope) is read for.
    app.scope = ThreadScope::All;
    assert!(poll(&app, false).is_some(), "another view");
  }

  /// What the event loop does on its timer and on each key, timed against
  /// a copy of a real data directory (its index's `roots` pointing into the
  /// copy): `STITCH_BENCH_DATA=<dir> STITCH_BENCH_THREAD=<id> cargo test
  /// --release -p stitch-cli bench_tui -- --ignored --nocapture`.
  /// On a copy of a real index (`sqlite3 ~/.stitch/index.sqlite ".backup
  /// /tmp/x/index.sqlite"`; the records are read where it says, not
  /// written): every thread's transcript up to each of its segments, as
  /// `chat_ended` counts what a chat showed, is the start of its whole
  /// transcript. `STITCH_CHECK_DATA=/tmp/x cargo test -p stitch-cli
  /// transcripts_up_to_a_segment_start_the_whole -- --ignored --nocapture`.
  #[test]
  #[ignore]
  fn transcripts_up_to_a_segment_start_the_whole() {
    let data = PathBuf::from(std::env::var("STITCH_CHECK_DATA").expect("STITCH_CHECK_DATA"));
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    let store = Store {
      blobs: BlobStore::open(&data.join("blobs")).unwrap(),
      data,
      node: "gamer".parse().unwrap(),
      here: Place::Global,
      here_label: "global".into(),
      index: Mutex::new(index),
    };
    let key = |e: &Entry| (e.who.clone(), e.text.clone());
    let only = std::env::var("STITCH_CHECK_THREAD").ok();
    let (mut threads, mut segments, mut bad) = (0, 0, Vec::new());
    for t in store.threads(ThreadScope::All) {
      let Ok(id) = t.id.parse::<RecordId>() else {
        continue;
      };
      if only.as_ref().is_some_and(|o| *o != t.id) {
        continue;
      }
      let whole: Vec<_> = store.transcript(&t.id).iter().map(key).collect();
      let segs = store
        .index
        .lock()
        .unwrap()
        .thread_segments(&id)
        .unwrap_or_default();
      threads += 1;
      for (seg, _, _) in segs {
        segments += 1;
        let Some(part) = store.transcript_to(&t.id, Some(seg.as_str())) else {
          bad.push(format!("{} {seg}: unreadable", t.id));
          continue;
        };
        let part: Vec<_> = part.iter().map(key).collect();
        if part.len() > whole.len() || part[..] != whole[..part.len()] {
          let at = part
            .iter()
            .zip(&whole)
            .position(|(a, b)| a != b)
            .unwrap_or(part.len().min(whole.len()));
          bad.push(format!(
            "{} {seg}: {} entries up to it, {} whole, apart at {at}: {:?} / {:?}",
            t.id,
            part.len(),
            whole.len(),
            part.get(at).map(clip_dbg),
            whole.get(at).map(clip_dbg),
          ));
        }
      }
    }
    eprintln!("{threads} threads, {segments} segments");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
  }

  #[cfg(test)]
  fn clip_dbg(e: &(String, String)) -> String {
    format!("{}: {}", e.0, e.1.chars().take(80).collect::<String>())
  }

  #[test]
  #[ignore]
  fn bench_tui() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::time::Instant;
    let data = PathBuf::from(std::env::var("STITCH_BENCH_DATA").expect("STITCH_BENCH_DATA"));
    let thread = std::env::var("STITCH_BENCH_THREAD").expect("STITCH_BENCH_THREAD");
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    let store = Store {
      blobs: BlobStore::open(&data.join("blobs")).unwrap(),
      data,
      node: "gamer".parse().unwrap(),
      here: Place::Global,
      here_label: "global".into(),
      index: Mutex::new(index),
    };
    let time = |what: &str, n: u32, f: &mut dyn FnMut()| {
      let t = Instant::now();
      for _ in 0..n {
        f();
      }
      eprintln!("{what:<28} {:>10.2?}", t.elapsed() / n);
    };
    let mut app = App::new("gamer", "m");
    app.threads = store.threads(ThreadScope::HereAndGlobal);
    let transcript = store.transcript(&thread);
    eprintln!(
      "{} threads, {} entries",
      app.threads.len(),
      transcript.len()
    );
    app.opened(Some(thread.clone()), &transcript);
    app.out.clear();
    let mut seen = std::collections::HashSet::new();
    time("roots (scan, refresh)", 5, &mut || {
      store.scan();
    });
    time("threads", 5, &mut || {
      app.threads = store.threads(ThreadScope::HereAndGlobal)
    });
    time("attention", 5, &mut || {
      app.attention_read(store.attention(), false)
    });
    time("reports", 5, &mut || {
      for (id, _) in store.reports(Some(&thread)) {
        seen.insert(id);
      }
    });
    time("follow (nothing new)", 5, &mut || {
      store.follow(&mut app, true)
    });
    time("transcript", 5, &mut || {
      store.transcript(&thread);
    });
    time("tick (all of it)", 5, &mut || {
      store.tick(&mut app, &mut seen)
    });
    time("tick, nothing changed", 50, &mut || {
      store.index.lock().unwrap().data_version().unwrap();
      store.open_segment_at(Some(&thread));
    });
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 12)).unwrap();
    time("key + height + draw", 200, &mut || {
      app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
      let h = app.height(160).min(40);
      term
        .resize(ratatui::layout::Rect::new(0, 0, 160, h))
        .unwrap();
      term.draw(|f| app.render(f)).unwrap();
    });
  }

  #[test]
  fn a_streamed_reply_prints_as_markdown() {
    let mut md = markdown::Md::default();
    let mut print = |p: Printed| printed(&p, &mut md, 80);
    print(Printed::ReplyStart("m".into()));
    let (styled, shown) = print(Printed::ReplyLines("## Hi **there**".into()));
    assert_eq!(shown, "Hi there");
    assert!(styled.contains('\x1b'), "{styled:?}");
    // A table prints nothing till it ends (no row is counted for it).
    assert_eq!(
      print(Printed::ReplyLines("| a | b |\n|---|---|".into())),
      (String::new(), String::new())
    );
    let (_, shown) = print(Printed::ReplyLines("| 1 | 2 |".into()));
    assert!(shown.is_empty());
    // Ended by the reply's end: it prints, then the blank line.
    let (styled, shown) = print(Printed::ReplyEnd);
    assert_eq!(shown, "a │ b\n──┼──\n1 │ 2\n");
    assert!(styled.ends_with("\r\n\r\n"));
    // A link is an OSC 8 hyperlink; the rows count what shows.
    let (styled, shown) = print(Printed::Reply {
      model: "m".into(),
      text: "[x](https://e.com)\n".into(),
    });
    assert_eq!(shown, "m\nx (https://e.com)\n");
    assert!(styled.contains("\x1b]8;;https://e.com\x1b\\"), "{styled:?}");
  }
}
