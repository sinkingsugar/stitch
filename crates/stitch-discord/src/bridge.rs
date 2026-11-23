//! What the bridge does with a message, apart from Discord's and the
//! node's wires (`Discord`, `Node`), so tests drive it with fakes.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::follow;
use crate::split;
use crate::text::{self, Names};

/// Discord, as the bridge uses it.
pub trait Discord: Send + Sync + 'static {
  /// Posts `text` in `channel` (a reply to `reply_to` there), pinging
  /// the users `ping` and no one else; the message's ID.
  fn send(
    &self,
    channel: u64,
    text: &str,
    reply_to: Option<u64>,
    ping: &[u64],
  ) -> impl Future<Output = Result<u64, String>> + Send;
  /// "Bot is typing…" in `channel`, for ten seconds or until it posts.
  fn typing(&self, channel: u64) -> impl Future<Output = ()> + Send;
  /// A thread started from `message` in `channel`; its channel ID.
  fn start_thread(
    &self,
    channel: u64,
    message: u64,
    name: &str,
  ) -> impl Future<Output = Result<u64, String>> + Send;
  /// A channel's name (a thread's included), when it can be had.
  fn channel_name(&self, channel: u64) -> impl Future<Output = Option<String>> + Send;
}

/// What a turn on the node gave.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Turn {
  pub thread: String,
  pub text: String,
  /// Commands that asked for approval, declined.
  pub declined: Vec<String>,
}

/// The node, as the bridge uses it.
pub trait Node: Send + Sync + 'static {
  /// A turn: on `thread` (a new one when `None`), for `end_user`.
  fn chat(
    &self,
    thread: Option<String>,
    text: String,
    end_user: String,
  ) -> impl Future<Output = Result<Turn, String>> + Send;
}

#[derive(Clone, Debug)]
pub struct Settings {
  /// The guilds it answers in; none: nowhere (it logs the guilds that
  /// asked, to add).
  pub guilds: HashSet<u64>,
  /// Turns a Discord user may start an hour.
  pub turns_per_hour: usize,
  /// Where the map of Discord channels to Stitch threads is kept.
  pub state: PathBuf,
  /// The per-server instructions: `<dir>/<guild id>.md`, given in the
  /// first message of each new Stitch thread in that server.
  pub servers: Option<PathBuf>,
  /// Discord user IDs whose messages carry the approver tag.
  pub approvers: HashSet<u64>,
  /// The node's home workspace (`<data>/home`), where the bridge follows
  /// its mapped threads for turns it did not start; `None`: it does not.
  pub home: Option<PathBuf>,
}

/// A message to the bot, from a mention, a bot thread or `/ask`.
#[derive(Clone, Debug, Default)]
pub struct Incoming {
  pub guild: Option<u64>,
  pub channel: u64,
  /// The message, for a mention or a message in a bot thread; `None`
  /// for a slash command (`Discord::send` posted what stands for it).
  pub message: Option<u64>,
  pub author: u64,
  pub author_name: String,
  pub author_is_bot: bool,
  pub text: String,
  pub mentions_me: bool,
  /// `/ask`: always for the bot.
  pub slash: bool,
  /// Users the text mentions, with their display names (the gateway
  /// gives them with the message).
  pub mentions: Vec<(u64, String)>,
  /// The message this one is a Discord reply to: its author's name and
  /// its text.
  pub reply: Option<(String, String)>,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
  /// Discord channel (a thread, mostly) → Stitch thread.
  threads: HashMap<u64, String>,
  /// Stitch thread → the last of its messages posted or seen by the
  /// bridge (`m12` → 12): what comes after is a turn it did not start.
  #[serde(default)]
  posted: HashMap<String, u64>,
}

#[derive(Default)]
struct State {
  saved: Saved,
  /// Each user's turns of the last hour.
  turns: HashMap<u64, Vec<Instant>>,
  /// Users with a turn running.
  busy: HashSet<u64>,
  /// Guilds not allowed that were told of in the log.
  told: HashSet<u64>,
  /// Guild names, as the gateway gave them.
  names: HashMap<u64, String>,
  /// Roles' and channels' names (the gateway's, and channels asked for),
  /// users' as messages mention them.
  known: Names,
  /// The bot: its user ID and name.
  me: (u64, String),
  /// Mapped threads' sealed segments when last read (`follow::stamp`).
  stamps: HashMap<String, String>,
  /// When "typing" was last shown for a turn the bridge did not start.
  typing_at: HashMap<u64, Instant>,
  /// Each channel's last message for the bot: what a reply the bridge
  /// did not ask for answers.
  last_msg: HashMap<u64, u64>,
  /// Channels with a turn running → the messages waiting for the next.
  active: HashMap<u64, Vec<Pending>>,
  /// Each conversation's people (ID → display name), whom the agent may
  /// ping by `@Name`.
  people: HashMap<u64, HashMap<u64, String>>,
}

/// A message waiting for its turn.
struct Pending {
  /// What the agent reads (`Name: text`).
  line: String,
  author: u64,
  /// The message and the channel it is in.
  message: Option<u64>,
  from: u64,
  /// A mention of the bot or `/ask`: an answer is due.
  addressed: bool,
  /// Its author and the people it mentions.
  who: Vec<(u64, String)>,
}

enum Enter {
  Run(Pending),
  Queued,
  Refused(String),
}

/// The longest per-server instructions given (characters); the rest is
/// cut, with a word in the log.
pub const SERVER_INSTRUCTIONS_MAX: usize = 20_000;

/// The instructions for `guild` in `dir` (`<guild id>.md`), trimmed;
/// `None` without a file or when it is blank.
pub fn server_instructions(dir: &Path, guild: u64) -> Option<String> {
  let path = dir.join(format!("{guild}.md"));
  let text = std::fs::read_to_string(&path).ok()?;
  let text = text.trim();
  if text.is_empty() {
    return None;
  }
  if text.chars().count() > SERVER_INSTRUCTIONS_MAX {
    eprintln!(
      "discord: {} is longer than {SERVER_INSTRUCTIONS_MAX} characters: cut",
      path.display()
    );
    return Some(text.chars().take(SERVER_INSTRUCTIONS_MAX).collect());
  }
  Some(text.to_owned())
}

/// The tags the bridge writes; in what people say they are defused (`<`
/// becomes `‹`), so no message can pass for the bridge's context.
const TAGS: [&str; 2] = ["discord-context", "server-instructions"];

/// The tag after the name of a trusted user (`STITCH_DISCORD_APPROVERS`,
/// by Discord user ID): `Ann [approver]: …`.
pub const APPROVER_TAG: &str = "[approver]";
const APPROVER_WORD: &str = "approver]";

/// `text` with the bridge's tags defused (any case, opening or closing).
pub fn defuse(text: &str) -> String {
  let lower = text.to_ascii_lowercase();
  let mut out = String::with_capacity(text.len());
  for (i, c) in text.char_indices() {
    if c == '<' {
      let rest = lower[i + 1..].strip_prefix('/').unwrap_or(&lower[i + 1..]);
      if TAGS.iter().any(|t| rest.starts_with(t)) {
        out.push('‹');
        continue;
      }
    }
    // The approver tag is the bridge's alone: `[approver]` in a name or
    // a message reads `⁅approver]`.
    if c == '[' && lower[i + 1..].starts_with(APPROVER_WORD) {
      out.push('⁅');
      continue;
    }
    out.push(c);
  }
  out
}

/// An attribute's value: defused, quotes and line breaks out.
fn attr(s: &str) -> String {
  defuse(s)
    .replace('"', "'")
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
}

/// The context the first message of a Stitch thread carries, before
/// what was said: the server (name and ID), the channel, and that
/// server's instructions when it has some.
pub fn context_header(
  server_name: Option<&str>,
  guild: u64,
  channel_name: Option<&str>,
  instructions: Option<&str>,
) -> String {
  let mut head = String::from("<discord-context");
  if let Some(n) = server_name {
    head.push_str(&format!(" server=\"{}\"", attr(n)));
  }
  head.push_str(&format!(" server_id=\"{guild}\""));
  if let Some(c) = channel_name {
    head.push_str(&format!(" channel=\"#{}\"", attr(c)));
  }
  match instructions {
    None => {
      head.push_str(" server_instructions=\"none\" />");
      head
    }
    Some(i) => {
      format!("{head}>\n<server-instructions>\n{i}\n</server-instructions>\n</discord-context>")
    }
  }
}

pub struct Bridge<D, N> {
  pub discord: Arc<D>,
  pub node: Arc<N>,
  settings: Settings,
  state: Mutex<State>,
}

/// A thread's name from what started it.
fn thread_name(text: &str) -> String {
  let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
  let mut name: String = one.chars().take(80).collect();
  if name.is_empty() {
    name = "Stitch".into();
  }
  name
}

/// `text` without mentions of `me` (`<@id>`, `<@!id>`).
pub fn strip_mention(text: &str, me: u64) -> String {
  text
    .replace(&format!("<@{me}>"), "")
    .replace(&format!("<@!{me}>"), "")
    .trim()
    .to_owned()
}

impl<D: Discord, N: Node> Bridge<D, N> {
  pub fn new(discord: Arc<D>, node: Arc<N>, settings: Settings) -> Arc<Self> {
    let saved = std::fs::read(&settings.state)
      .ok()
      .and_then(|b| serde_json::from_slice(&b).ok())
      .unwrap_or_default();
    Arc::new(Bridge {
      discord,
      node,
      settings,
      state: Mutex::new(State {
        saved,
        ..State::default()
      }),
    })
  }

  pub fn allowed(&self, guild: u64) -> bool {
    self.settings.guilds.contains(&guild)
  }

  pub fn guilds(&self) -> Vec<u64> {
    self.settings.guilds.iter().copied().collect()
  }

  /// A guild's name, from the gateway (`GUILD_CREATE`).
  pub fn guild_seen(&self, guild: u64, name: &str) {
    let mut s = self.state.lock().expect("bridge state");
    s.names.insert(guild, name.to_owned());
  }

  /// The bot's user, from `READY`: its own mention reads `@<name>`.
  pub fn set_me(&self, id: u64, name: &str) {
    self.state.lock().expect("bridge state").me = (id, name.to_owned());
  }

  /// Roles' and channels' names, from the gateway.
  pub fn learn(&self, roles: &[(u64, String)], channels: &[(u64, String)]) {
    let mut s = self.state.lock().expect("bridge state");
    s.known.roles.extend(roles.iter().cloned());
    s.known.channels.extend(channels.iter().cloned());
  }

  /// `text` with mentions as the names known now (a thread's name).
  fn resolved(&self, text: &str) -> String {
    let s = self.state.lock().expect("bridge state");
    text::resolve(text, &s.known)
  }

  /// What the agent reads of a message: `Name: text`, mentions as names
  /// (the bot's own as `@<its name>`), `(/ask)` for the command, and
  /// `(replying to Other: …)` for a Discord reply; defused.
  async fn line(&self, m: &Incoming, text: &str) -> String {
    // Channels mentioned and not known yet: asked for once.
    let unknown: Vec<u64> = {
      let s = self.state.lock().expect("bridge state");
      let mut ids: Vec<u64> = text::mentions(text)
        .into_iter()
        .chain(
          m.reply
            .as_ref()
            .map(|r| text::mentions(&r.1))
            .unwrap_or_default(),
        )
        .filter_map(|x| match x {
          text::Mention::Channel(id) if !s.known.channels.contains_key(&id) => Some(id),
          _ => None,
        })
        .collect();
      ids.sort_unstable();
      ids.dedup();
      ids
    };
    for id in unknown {
      if let Some(name) = self.discord.channel_name(id).await {
        let mut s = self.state.lock().expect("bridge state");
        s.known.channels.insert(id, name);
      }
    }
    let names = {
      let mut s = self.state.lock().expect("bridge state");
      for (id, name) in &m.mentions {
        s.known.users.insert(*id, name.clone());
      }
      s.known.users.insert(m.author, m.author_name.clone());
      let (me, my_name) = s.me.clone();
      let mut names = s.known.clone();
      if me != 0 && !my_name.is_empty() {
        names.users.insert(me, my_name);
      }
      names
    };
    // The name defused on its own, then the tag: no nickname can carry it.
    let mut head = defuse(&m.author_name);
    if self.settings.approvers.contains(&m.author) {
      head.push(' ');
      head.push_str(APPROVER_TAG);
    }
    let mut rest = String::new();
    if m.slash {
      rest.push_str(" (/ask)");
    }
    if let Some((who, said)) = &m.reply {
      rest.push(' ');
      rest.push_str(&text::reply_marker(who, &text::resolve(said, &names)));
    }
    let rest = defuse(&format!("{rest}: {}", text::resolve(text, &names)));
    format!("{head}{rest}")
  }

  /// The first message's context for a new Stitch thread from `channel`
  /// in `guild`: the server's file read now, so an edit counts for the
  /// conversations started after it.
  async fn context(&self, guild: u64, channel: u64) -> String {
    let name = self
      .state
      .lock()
      .expect("bridge state")
      .names
      .get(&guild)
      .cloned();
    let channel_name = self.discord.channel_name(channel).await;
    let instructions = self
      .settings
      .servers
      .as_deref()
      .and_then(|d| server_instructions(d, guild));
    context_header(
      name.as_deref(),
      guild,
      channel_name.as_deref(),
      instructions.as_deref(),
    )
  }

  /// The Stitch thread a Discord channel continues.
  pub fn thread_of(&self, channel: u64) -> Option<String> {
    let s = self.state.lock().expect("bridge state");
    s.saved.threads.get(&channel).cloned()
  }

  fn map(&self, channel: u64, thread: &str) {
    let mut s = self.state.lock().expect("bridge state");
    s.saved.threads.insert(channel, thread.to_owned());
    self.save(&s);
  }

  /// The thread's messages so far are seen (its turn posted, or not
  /// for Discord).
  fn mark(&self, thread: &str) {
    let Some(home) = &self.settings.home else {
      return;
    };
    let latest = follow::latest(&follow::files(home, thread));
    let mut s = self.state.lock().expect("bridge state");
    s.saved.posted.insert(thread.to_owned(), latest);
    self.save(&s);
  }

  /// Follows the mapped threads every `every`: `follow_once` forever.
  pub async fn follow(self: Arc<Self>, every: Duration) {
    if self.settings.home.is_none() {
      return;
    }
    loop {
      self.follow_once().await;
      tokio::time::sleep(every).await;
    }
  }

  /// Posts what mapped threads said in turns the bridge did not start (a
  /// report waking one, a schedule, a `send`), once each turn has ended:
  /// "typing" while one runs, its reply where the thread is mapped, as
  /// the bridge's own turns are posted (split, pings, `[pass]`: nothing).
  /// A thread first seen (none of it marked) is marked, not posted.
  pub async fn follow_once(&self) {
    let Some(home) = self.settings.home.clone() else {
      return;
    };
    let mapped: Vec<(u64, String)> = {
      let s = self.state.lock().expect("bridge state");
      s.saved
        .threads
        .iter()
        .filter(|(c, _)| !s.active.contains_key(c))
        .map(|(c, t)| (*c, t.clone()))
        .collect()
    };
    for (channel, thread) in mapped {
      let f = follow::files(&home, &thread);
      if follow::running(&f) {
        let due = {
          let mut s = self.state.lock().expect("bridge state");
          let now = Instant::now();
          let due = s
            .typing_at
            .get(&channel)
            .is_none_or(|t| now.duration_since(*t) >= Duration::from_secs(8));
          if due {
            s.typing_at.insert(channel, now);
          }
          due
        };
        if due {
          self.discord.typing(channel).await;
        }
        continue;
      }
      let stamp = follow::stamp(&f);
      let mark = {
        let mut s = self.state.lock().expect("bridge state");
        s.typing_at.remove(&channel);
        if s.stamps.get(&thread) == Some(&stamp) {
          continue;
        }
        s.stamps.insert(thread.clone(), stamp);
        s.saved.posted.get(&thread).copied()
      };
      let Some(mark) = mark else {
        // Mapped before the bridge followed threads: its past is not
        // posted again.
        self.mark(&thread);
        continue;
      };
      let (text, last) = follow::said_after(&f, mark);
      if last == mark {
        continue;
      }
      let reply_to = {
        let mut s = self.state.lock().expect("bridge state");
        // A turn of its own began meanwhile: that one posts.
        if s.active.contains_key(&channel) {
          s.stamps.remove(&thread);
          continue;
        }
        s.saved.posted.insert(thread.clone(), last);
        self.save(&s);
        s.last_msg.get(&channel).copied()
      };
      if !text.trim().is_empty() {
        self.post(channel, &text, reply_to).await;
      }
    }
  }

  fn save(&self, s: &State) {
    if let Some(dir) = self.settings.state.parent() {
      let _ = std::fs::create_dir_all(dir);
    }
    let tmp = self.settings.state.with_extension("tmp");
    if let Ok(b) = serde_json::to_vec_pretty(&s.saved)
      && std::fs::write(&tmp, b).is_ok()
    {
      let _ = std::fs::rename(&tmp, &self.settings.state);
    }
  }

  /// One more message from `user` this hour, unless over the limit:
  /// `Err` says why.
  fn count(&self, s: &mut State, user: u64) -> Result<(), String> {
    let now = Instant::now();
    let hour = Duration::from_secs(3600);
    let turns = s.turns.entry(user).or_default();
    turns.retain(|t| now.duration_since(*t) < hour);
    if turns.len() >= self.settings.turns_per_hour {
      let wait = hour - now.duration_since(turns[0]);
      return Err(format!(
        "You've asked {} times this hour, my limit: again in {} min.",
        self.settings.turns_per_hour,
        wait.as_secs().div_ceil(60)
      ));
    }
    turns.push(now);
    Ok(())
  }

  /// A new conversation from `user`: one at a time, and the hour's
  /// allowance. `Err` says why not.
  fn take_turn(&self, user: u64) -> Result<(), String> {
    let mut s = self.state.lock().expect("bridge state");
    if s.busy.contains(&user) {
      return Err("I'm still on your last message: one at a time, please.".into());
    }
    self.count(&mut s, user)?;
    s.busy.insert(user);
    Ok(())
  }

  fn done(&self, user: u64) {
    self.state.lock().expect("bridge state").busy.remove(&user);
  }

  /// A message for a conversation: run now (the channel is claimed), or
  /// queued behind the turn running there, for the next one.
  fn enter(&self, channel: u64, p: Pending) -> Enter {
    let mut s = self.state.lock().expect("bridge state");
    if let Err(why) = self.count(&mut s, p.author) {
      return Enter::Refused(why);
    }
    if let Some(q) = s.active.get_mut(&channel) {
      q.push(p);
      return Enter::Queued;
    }
    s.active.insert(channel, Vec::new());
    Enter::Run(p)
  }

  /// The messages that came while a turn ran in `channel`, all at once;
  /// none: the channel is free again.
  fn next(&self, channel: u64) -> Option<Vec<Pending>> {
    let mut s = self.state.lock().expect("bridge state");
    match s.active.get_mut(&channel) {
      Some(q) if !q.is_empty() => Some(std::mem::take(q)),
      _ => {
        s.active.remove(&channel);
        None
      }
    }
  }

  /// People of a conversation, whom the agent may ping by name.
  fn met(&self, channel: u64, who: &[(u64, String)]) {
    let mut s = self.state.lock().expect("bridge state");
    let me = s.me.0;
    let people = s.people.entry(channel).or_default();
    for (id, name) in who {
      if *id != me && !name.is_empty() {
        people.insert(*id, name.clone());
      }
    }
  }

  /// Whether a message is the bot's to answer, and the text to send.
  fn wanted(&self, m: &Incoming) -> Option<String> {
    if m.author_is_bot {
      return None;
    }
    let guild = m.guild?;
    if !self.allowed(guild) {
      let mut s = self.state.lock().expect("bridge state");
      if s.told.insert(guild) {
        eprintln!(
          "discord: guild {guild} is not allowed (STITCH_DISCORD_GUILDS): its messages are ignored"
        );
      }
      return None;
    }
    let active = self
      .state
      .lock()
      .expect("bridge state")
      .active
      .contains_key(&m.channel);
    if !(m.slash || m.mentions_me || active || self.thread_of(m.channel).is_some()) {
      return None;
    }
    Some(m.text.trim().to_owned())
  }

  /// Answers a message: in the Discord thread it belongs to, or one
  /// started from it, on the Stitch thread mapped there. Messages that
  /// come while a turn runs in that thread wait and go together in the
  /// next turn, each line its author's.
  pub async fn handle(self: Arc<Self>, m: Incoming) {
    let Some(text) = self.wanted(&m) else {
      return;
    };
    let addressed = m.mentions_me || m.slash;
    let reply_here = |t: String| {
      let d = self.discord.clone();
      async move {
        if let Err(e) = d.send(m.channel, &t, m.message, &[]).await {
          eprintln!("discord: posting in {}: {e}", m.channel);
        }
      }
    };
    let me = self.state.lock().expect("bridge state").me.0;
    let bare = strip_mention(&text, me);
    if bare.is_empty() {
      if addressed {
        reply_here("Ask me something: mention me with a question, or use `/ask`.".into()).await;
      }
      return;
    }
    let mut who = vec![(m.author, m.author_name.clone())];
    who.extend(m.mentions.iter().cloned());
    if let Some(id) = m.message {
      self
        .state
        .lock()
        .expect("bridge state")
        .last_msg
        .insert(m.channel, id);
    }
    let pending = Pending {
      line: self.line(&m, &text).await,
      author: m.author,
      message: m.message,
      from: m.channel,
      addressed,
      who,
    };
    let mapped = self.thread_of(m.channel);
    let ongoing = self
      .state
      .lock()
      .expect("bridge state")
      .active
      .contains_key(&m.channel);
    // A conversation going on (mapped, or a turn running here): this
    // message joins it, now or with the next turn; no one-at-a-time rule
    // between the people of a thread.
    if mapped.is_some() || ongoing {
      match self.enter(m.channel, pending) {
        Enter::Run(p) => self.converse(m.channel, mapped, p, None).await,
        Enter::Queued => {}
        Enter::Refused(why) => {
          if addressed {
            reply_here(why).await;
          }
        }
      }
      return;
    }
    // A new conversation: a thread started from the message (where
    // threads cannot start, here).
    if let Err(why) = self.take_turn(m.author) {
      reply_here(why).await;
      return;
    }
    let channel = match m.message {
      None => m.channel,
      Some(msg) => match self
        .discord
        .start_thread(m.channel, msg, &thread_name(&self.resolved(&bare)))
        .await
      {
        Ok(t) => t,
        Err(e) => {
          eprintln!(
            "discord: no thread from message {msg} in {}: {e}",
            m.channel
          );
          m.channel
        }
      },
    };
    let first = {
      let mut s = self.state.lock().expect("bridge state");
      if let Some(q) = s.active.get_mut(&channel) {
        q.push(pending);
        None
      } else {
        s.active.insert(channel, Vec::new());
        Some(pending)
      }
    };
    if let Some(p) = first {
      let context = m.guild.map(|g| (g, m.channel));
      self.converse(channel, None, p, context).await;
    }
    self.done(m.author);
  }

  /// Turns in `channel` until no message waits: the first `p`, then
  /// what came meanwhile, together. `context`: the guild and the channel
  /// the conversation started from, for a new Stitch thread's header.
  async fn converse(
    &self,
    channel: u64,
    mut thread: Option<String>,
    first: Pending,
    context: Option<(u64, u64)>,
  ) {
    let mut batch = vec![first];
    loop {
      for p in &batch {
        self.met(channel, &p.who);
      }
      let mut said = batch
        .iter()
        .map(|p| p.line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
      if thread.is_none()
        && let Some((guild, from)) = context
      {
        said = format!("{}\n{said}", self.context(guild, from).await);
      }
      let last = batch.last().expect("a batch has a message");
      let end_user = format!("discord:{}", last.author);
      // A reply to the last message, where it is (not from a new
      // thread to the message it started from).
      let reply_to = (last.from == channel).then_some(last.message).flatten();
      let addressed = batch.iter().any(|p| p.addressed);
      let typing = {
        let d = self.discord.clone();
        tokio::spawn(async move {
          loop {
            d.typing(channel).await;
            tokio::time::sleep(Duration::from_secs(8)).await;
          }
        })
      };
      let turn = self.node.chat(thread.clone(), said, end_user).await;
      typing.abort();
      let text = match turn {
        Ok(t) => {
          if thread.as_deref() != Some(t.thread.as_str()) {
            self.map(channel, &t.thread);
            thread = Some(t.thread.clone());
          }
          let pass = text::is_pass(&t.text);
          if pass && t.declined.is_empty() && !addressed {
            // The agent passed: nothing is posted.
            None
          } else {
            if pass {
              eprintln!("discord: the agent passed on a message for it, in {channel}");
            }
            let mut text = if pass { String::new() } else { t.text };
            for c in &t.declined {
              text.push_str(&format!(
                "\n\n*(I wanted to run `{c}`, which needs the owner's approval: declined.)*"
              ));
            }
            Some(if text.trim().is_empty() {
              "*(no answer)*".to_owned()
            } else {
              text
            })
          }
        }
        Err(e) => {
          eprintln!("discord: a turn in {channel} failed: {e}");
          let e: String = e.chars().take(300).collect();
          Some(format!("Sorry, that failed: {e}"))
        }
      };
      if let Some(text) = text {
        self.post(channel, &text, reply_to).await;
      }
      // What the thread holds now is seen: the follower posts only what
      // comes after (turns the bridge did not start).
      if let Some(t) = &thread {
        self.mark(t);
      }
      match self.next(channel) {
        Some(more) => batch = more,
        None => break,
      }
    }
  }

  /// Posts an answer in pieces, the first a reply to `reply_to`; `@Name`
  /// of the conversation's people pings them, nothing else does.
  async fn post(&self, channel: u64, text: &str, reply_to: Option<u64>) {
    let people = self
      .state
      .lock()
      .expect("bridge state")
      .people
      .get(&channel)
      .cloned()
      .unwrap_or_default();
    let (text, ping) = text::pings(text, &people);
    for (i, part) in split::split(&text, split::LIMIT).into_iter().enumerate() {
      let to = if i == 0 { reply_to } else { None };
      let ids: Vec<u64> = ping
        .iter()
        .copied()
        .filter(|id| part.contains(&format!("<@{id}>")))
        .collect();
      if let Err(e) = self.discord.send(channel, &part, to, &ids).await {
        eprintln!("discord: posting in {channel}: {e}");
        break;
      }
    }
  }
}
