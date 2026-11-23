//! `stitch discord` (design §2.21): a Discord bot in front of a node. A
//! mention of the bot (or `/ask`) in an allowed guild starts a Stitch
//! thread, answered in a Discord thread started from the message; what is
//! said in that Discord thread continues it. Replies are cut under
//! Discord's 2000 characters, "typing…" shows while a turn runs, each
//! Discord user has a rate of turns, their ID goes with the turn's model
//! calls (`end_user`, for the key-holding proxy to account by), and the
//! rare approval a node asks is declined with a word in the reply.

pub mod bridge;
pub mod follow;
pub mod split;
pub mod text;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bridge::{Bridge, Discord, Incoming, Node, Settings, Turn};
use stitch_node::api::{Reply, Request};
use twilight_gateway::{
  CloseFrame, Event, EventTypeFlags, Intents, Shard, ShardId, StreamExt as _,
};
use twilight_http::Client as Http;
use twilight_model::application::command::Command;
use twilight_model::application::interaction::InteractionData;
use twilight_model::application::interaction::application_command::CommandOptionValue;
use twilight_model::http::interaction::{
  InteractionResponse, InteractionResponseData, InteractionResponseType,
};
use twilight_model::id::Id;

/// The application `stitch` was registered as (public; the invite URL
/// and the docs use it). The gateway says the real one at `READY`.
pub const APPLICATION_ID: u64 = 1563836825141780564;

/// How the bridge runs, from the environment (`stitch discord` reads
/// `DISCORD_TOKEN` or `DISCORD_TOKEN_FILE`, `STITCH_DISCORD_GUILDS`,
/// `STITCH_DISCORD_TURNS_PER_HOUR`, `STITCH_DISCORD_MODEL`).
#[derive(Clone, Debug)]
pub struct Options {
  pub token: String,
  pub guilds: HashSet<u64>,
  pub turns_per_hour: usize,
  pub model: Option<String>,
  /// The node's data directory.
  pub data: PathBuf,
  /// Posts a line in each allowed guild's system channel when it
  /// connects (a check that it can post).
  pub announce: bool,
  /// The per-server instructions, `<dir>/<guild id>.md`
  /// (`STITCH_DISCORD_SERVERS`).
  pub servers: Option<PathBuf>,
  /// Discord user IDs marked `[approver]` (`STITCH_DISCORD_APPROVERS`).
  pub approvers: HashSet<u64>,
}

/// Guild IDs from a list (`123,456` or spaces).
pub fn parse_guilds(s: &str) -> Result<HashSet<u64>, String> {
  s.split(|c: char| c == ',' || c.is_whitespace())
    .filter(|p| !p.is_empty())
    .map(|p| {
      p.parse::<u64>()
        .ok()
        .filter(|&n| n > 0)
        .ok_or_else(|| format!("`{p}` is not a guild ID"))
    })
    .collect()
}

/// The node of a data directory, over its socket: a connection a turn.
pub struct LocalNode {
  pub data: PathBuf,
  pub model: Option<String>,
}

impl Node for LocalNode {
  async fn chat(
    &self,
    thread: Option<String>,
    text: String,
    end_user: String,
  ) -> Result<Turn, String> {
    let mut c = stitch_node::client::Client::connect(&self.data)
      .await
      .map_err(|e| format!("the node: {e}"))?;
    let req = Request::Chat {
      thread,
      text,
      attachments: Vec::new(),
      model: self.model.clone(),
      repo: None,
      cwd: None,
      global: true,
      env: Vec::new(),
      tools: Vec::new(),
      skills: Vec::new(),
      place: None,
      memory: None,
      end_user: Some(end_user),
      node: None,
      spoken: false,
      scopes: Vec::new(),
    };
    let mut text = String::new();
    let mut declined = Vec::new();
    let mut thread = None;
    let done = c
      .chat(&req, |r| {
        match r {
          Reply::Thread(t) => thread = Some(t.clone()),
          Reply::Text(t) => text.push_str(t),
          // Nobody here may approve: declined, and said.
          Reply::Approve { command, .. } => {
            declined.push(command.clone());
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = tx.send(stitch_node::tools::Decision {
              allow: false,
              always: false,
              reason: Some(
                "asked from Discord, where nobody can approve: do without it, or say it needs the owner"
                  .into(),
              ),
            });
            return Some(rx);
          }
          _ => {}
        }
        None
      })
      .await
      .map_err(|e| e.to_string())?;
    match done {
      Reply::ChatDone { thread: t, .. } => Ok(Turn {
        thread: t,
        text,
        declined,
      }),
      _ => Err(format!(
        "the turn was not sent{}",
        thread.map(|t| format!(" (thread {t})")).unwrap_or_default()
      )),
    }
  }
}

/// Discord over its HTTP API.
pub struct Rest {
  pub http: Arc<Http>,
}

fn id<T>(n: u64) -> Result<Id<T>, String> {
  Id::new_checked(n).ok_or_else(|| "a zero ID".to_owned())
}

impl Discord for Rest {
  async fn send(
    &self,
    channel: u64,
    text: &str,
    reply_to: Option<u64>,
    ping: &[u64],
  ) -> Result<u64, String> {
    // Only the users named ping: a model's `@everyone`, `@here` or a role
    // stays text.
    let allowed = twilight_model::channel::message::AllowedMentions {
      users: ping.iter().filter_map(|&u| Id::new_checked(u)).collect(),
      ..Default::default()
    };
    let mut m = self
      .http
      .create_message(id(channel)?)
      .content(text)
      .allowed_mentions(Some(&allowed));
    if let Some(r) = reply_to {
      m = m.reply(id(r)?).fail_if_not_exists(false);
    }
    let msg = m
      .await
      .map_err(|e| e.to_string())?
      .model()
      .await
      .map_err(|e| e.to_string())?;
    Ok(msg.id.get())
  }

  async fn typing(&self, channel: u64) {
    if let Ok(c) = id(channel) {
      let _ = self.http.create_typing_trigger(c).await;
    }
  }

  async fn start_thread(&self, channel: u64, message: u64, name: &str) -> Result<u64, String> {
    let t = self
      .http
      .create_thread_from_message(id(channel)?, id(message)?, name)
      .await
      .map_err(|e| e.to_string())?
      .model()
      .await
      .map_err(|e| e.to_string())?;
    Ok(t.id.get())
  }

  async fn channel_name(&self, channel: u64) -> Option<String> {
    let c = self.http.channel(id(channel).ok()?).await.ok()?;
    c.model().await.ok()?.name
  }
}

/// The slash command: `/ask question:<text>`.
pub fn ask_command() -> Command {
  serde_json::from_value(serde_json::json!({
    "name": "ask",
    "description": "Ask Stitch something; it answers in a thread",
    "type": 1,
    "version": "1",
    "default_member_permissions": null,
    "options": [{
      "name": "question",
      "description": "What to ask",
      "type": 3,
      "required": true,
      "max_length": 1800
    }]
  }))
  .expect("the command's JSON")
}

/// Runs the bridge until the gateway closes for good.
pub async fn run(o: Options) -> Result<(), String> {
  let _ = rustls::crypto::ring::default_provider().install_default();
  if o.guilds.is_empty() {
    eprintln!(
      "discord: STITCH_DISCORD_GUILDS is empty: no guild is answered; the guilds the bot is in are logged below, to add"
    );
  }
  let http = Arc::new(Http::new(o.token.clone()));
  let settings = Settings {
    guilds: o.guilds.clone(),
    turns_per_hour: o.turns_per_hour,
    state: o.data.join("discord").join("threads.json"),
    home: Some(o.data.join("home")),
    servers: o.servers.clone(),
    approvers: o.approvers.clone(),
  };
  if let Some(d) = &o.servers {
    eprintln!(
      "discord: per-server instructions from {}/<guild id>.md",
      d.display()
    );
  }
  let node = Arc::new(LocalNode {
    data: o.data.clone(),
    model: o.model.clone(),
  });
  let bridge = Bridge::new(Arc::new(Rest { http: http.clone() }), node, settings);
  // Turns the bridge did not start, posted as they end.
  tokio::spawn(bridge.clone().follow(std::time::Duration::from_secs(3)));
  let intents = Intents::GUILDS | Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT;
  let mut shard = Shard::new(ShardId::ONE, o.token.clone(), intents);
  let mut me: u64 = 0;
  let mut app: u64 = APPLICATION_ID;
  let mut registered: HashSet<u64> = HashSet::new();
  let mut announced: HashSet<u64> = HashSet::new();
  let wanted = EventTypeFlags::READY
    | EventTypeFlags::GUILD_CREATE
    | EventTypeFlags::MESSAGE_CREATE
    | EventTypeFlags::INTERACTION_CREATE
    | EventTypeFlags::ROLE_CREATE
    | EventTypeFlags::ROLE_UPDATE
    | EventTypeFlags::CHANNEL_CREATE
    | EventTypeFlags::CHANNEL_UPDATE
    | EventTypeFlags::THREAD_CREATE;
  loop {
    let event = tokio::select! {
      e = shard.next_event(wanted) => e,
      _ = tokio::signal::ctrl_c() => {
        shard.close(CloseFrame::NORMAL);
        return Ok(());
      }
    };
    let event = match event {
      None => return Err("the gateway connection ended".into()),
      Some(Ok(e)) => e,
      Some(Err(e)) => {
        eprintln!("discord: gateway: {e}");
        continue;
      }
    };
    match event {
      Event::Ready(r) => {
        me = r.user.id.get();
        bridge.set_me(me, &r.user.name);
        app = r.application.id.get();
        let guilds: Vec<String> = r.guilds.iter().map(|g| g.id.to_string()).collect();
        eprintln!(
          "discord: connected as {} ({me}), application {app}; in guilds [{}], answering in [{}]",
          r.user.name,
          guilds.join(", "),
          bridge
            .guilds()
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(", ")
        );
      }
      Event::GuildCreate(g) => {
        let gid = g.id().get();
        if let twilight_model::gateway::payload::incoming::GuildCreate::Available(guild) = &*g {
          bridge.guild_seen(gid, &guild.name);
          let roles: Vec<(u64, String)> = guild
            .roles
            .iter()
            .map(|r| (r.id.get(), r.name.clone()))
            .collect();
          let channels: Vec<(u64, String)> = guild
            .channels
            .iter()
            .chain(guild.threads.iter())
            .filter_map(|c| Some((c.id.get(), c.name.clone()?)))
            .collect();
          bridge.learn(&roles, &channels);
          eprintln!(
            "discord: in guild {gid} \"{}\"{}",
            guild.name,
            if bridge.allowed(gid) {
              ""
            } else {
              " (not allowed: ignored)"
            }
          );
          if bridge.allowed(gid)
            && o.announce
            && announced.insert(gid)
            && let Some(ch) = guild.system_channel_id
          {
            let d = Rest { http: http.clone() };
            if let Err(e) = d
              .send(
                ch.get(),
                "Stitch is online: mention me with a question, or use `/ask`.",
                None,
                &[],
              )
              .await
            {
              eprintln!("discord: announcing in {ch}: {e}");
            }
          }
        }
        if bridge.allowed(gid) && registered.insert(gid) {
          let cmds = [ask_command()];
          match http
            .interaction(id(app)?)
            .set_guild_commands(id(gid)?, &cmds)
            .await
          {
            Ok(_) => eprintln!("discord: /ask registered in guild {gid}"),
            Err(e) => eprintln!("discord: registering /ask in guild {gid}: {e}"),
          }
        }
      }
      Event::MessageCreate(m) => {
        let mentions_me = m.mentions.iter().any(|u| u.id.get() == me);
        let name = m
          .member
          .as_ref()
          .and_then(|mem| mem.nick.clone())
          .or_else(|| m.author.global_name.clone())
          .unwrap_or_else(|| m.author.name.clone());
        let incoming = Incoming {
          guild: m.guild_id.map(|g| g.get()),
          channel: m.channel_id.get(),
          message: Some(m.id.get()),
          author: m.author.id.get(),
          author_name: name,
          author_is_bot: m.author.bot || m.author.id.get() == me,
          text: m.content.clone(),
          mentions_me,
          slash: false,
          mentions: m
            .mentions
            .iter()
            .map(|u| {
              let name = u
                .member
                .as_ref()
                .and_then(|mem| mem.nick.clone())
                .unwrap_or_else(|| u.name.clone());
              (u.id.get(), name)
            })
            .collect(),
          reply: m.referenced_message.as_ref().map(|r| {
            let who = r
              .member
              .as_ref()
              .and_then(|mem| mem.nick.clone())
              .or_else(|| r.author.global_name.clone())
              .unwrap_or_else(|| r.author.name.clone());
            (who, r.content.clone())
          }),
        };
        tokio::spawn(bridge.clone().handle(incoming));
      }
      Event::InteractionCreate(i) => {
        let Some(InteractionData::ApplicationCommand(cmd)) = &i.data else {
          continue;
        };
        if cmd.name != "ask" {
          continue;
        }
        let question = cmd
          .options
          .iter()
          .find(|o| o.name == "question")
          .and_then(|o| match &o.value {
            CommandOptionValue::String(s) => Some(s.clone()),
            _ => None,
          })
          .unwrap_or_default();
        let user = i
          .member
          .as_ref()
          .and_then(|m| m.user.clone())
          .or_else(|| i.user.clone());
        let (Some(user), Some(channel)) = (user, i.channel.as_ref().map(|c| c.id)) else {
          continue;
        };
        let name = i
          .member
          .as_ref()
          .and_then(|m| m.nick.clone())
          .or_else(|| user.global_name.clone())
          .unwrap_or_else(|| user.name.clone());
        // Another guild's `/ask` is left alone: with several bridges on
        // one token (instances, each its own guilds) every one gets the
        // interaction, and only the bridge that answers there may.
        if !i.guild_id.is_some_and(|g| bridge.allowed(g.get())) {
          continue;
        }
        let shown = format!("**{name}** asked: {question}");
        let shown: String = shown.chars().take(1990).collect();
        let response = InteractionResponse {
          kind: InteractionResponseType::ChannelMessageWithSource,
          data: Some(InteractionResponseData {
            content: Some(shown),
            allowed_mentions: Some(Default::default()),
            ..Default::default()
          }),
        };
        let ic = http.interaction(i.application_id);
        if let Err(e) = ic.create_response(i.id, &i.token, &response).await {
          eprintln!("discord: answering /ask: {e}");
          continue;
        }
        // The answer stands for the question: a thread starts from it,
        // unless the channel already is the bot's.
        let posted = match ic.response(&i.token).await {
          Ok(r) => r.model().await.ok().map(|m| m.id.get()),
          Err(_) => None,
        };
        let channel = channel.get();
        let message = if bridge.thread_of(channel).is_some() {
          None
        } else {
          posted
        };
        let incoming = Incoming {
          guild: i.guild_id.map(|g| g.get()),
          channel,
          message,
          author: user.id.get(),
          author_name: name,
          author_is_bot: false,
          text: question,
          mentions_me: true,
          slash: true,
          ..Incoming::default()
        };
        tokio::spawn(bridge.clone().handle(incoming));
      }
      Event::RoleCreate(r) => bridge.learn(&[(r.role.id.get(), r.role.name.clone())], &[]),
      Event::RoleUpdate(r) => bridge.learn(&[(r.role.id.get(), r.role.name.clone())], &[]),
      Event::ChannelCreate(c) => {
        if let Some(n) = &c.name {
          bridge.learn(&[], &[(c.id.get(), n.clone())]);
        }
      }
      Event::ChannelUpdate(c) => {
        if let Some(n) = &c.name {
          bridge.learn(&[], &[(c.id.get(), n.clone())]);
        }
      }
      Event::ThreadCreate(c) => {
        if let Some(n) = &c.name {
          bridge.learn(&[], &[(c.id.get(), n.clone())]);
        }
      }
      Event::GatewayClose(frame) => {
        let code = frame.as_ref().map(|f| f.code);
        eprintln!("discord: the gateway closed the connection: {frame:?}");
        if code == Some(4014) {
          return Err(
            "Discord refused the intents asked (4014, disallowed intents): turn on the Message Content intent \
             in the Developer Portal, Bot → Privileged Gateway Intents"
              .into(),
          );
        }
        if code == Some(4004) {
          return Err("Discord refused the token (4004, authentication failed)".into());
        }
      }
      _ => {}
    }
  }
}

/// The token from `DISCORD_TOKEN`, or the file `DISCORD_TOKEN_FILE` names.
pub fn token_from_env() -> Result<String, String> {
  if let Ok(t) = std::env::var("DISCORD_TOKEN")
    && !t.trim().is_empty()
  {
    return Ok(t.trim().to_owned());
  }
  if let Ok(p) = std::env::var("DISCORD_TOKEN_FILE") {
    return std::fs::read_to_string(Path::new(&p))
      .map(|t| t.trim().to_owned())
      .map_err(|e| format!("DISCORD_TOKEN_FILE {p}: {e}"));
  }
  Err("no Discord token: set DISCORD_TOKEN or DISCORD_TOKEN_FILE".into())
}

#[cfg(test)]
mod tests;
