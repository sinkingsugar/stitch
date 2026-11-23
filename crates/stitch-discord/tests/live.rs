//! Against Discord itself (ignored; needs `DISCORD_TOKEN` and
//! `STITCH_DISCORD_TEST_CHANNEL`, a text channel of a test server the bot
//! may post and start threads in): the bot posts a question as if a user
//! had mentioned it, and the bridge (Discord's HTTP API, a fake node)
//! answers it in a thread started from it, a long reply in pieces.
//!
//! `cargo test -p stitch-discord --test live -- --ignored --nocapture`

use std::sync::Arc;

use stitch_discord::bridge::{Bridge, Discord, Incoming, Node, Settings, Turn};
use stitch_discord::{Rest, token_from_env};

struct Fake;

impl Node for Fake {
  async fn chat(&self, _: Option<String>, text: String, end_user: String) -> Result<Turn, String> {
    let mut answer = format!("(live test of the bridge) I got `{text}` for {end_user}.\n\n");
    for i in 1..=60 {
      answer.push_str(&format!(
        "{i}. A line of a long answer, to be split under 2000 characters.\n"
      ));
    }
    Ok(Turn {
      thread: "live-test".into(),
      text: answer,
      declined: vec!["sudo reboot".into()],
    })
  }
}

#[tokio::test]
#[ignore]
async fn the_bridge_answers_in_a_thread_on_discord() {
  let _ = rustls::crypto::ring::default_provider().install_default();
  let token = token_from_env().unwrap();
  let channel: u64 = std::env::var("STITCH_DISCORD_TEST_CHANNEL")
    .unwrap()
    .parse()
    .unwrap();
  let guild: u64 = std::env::var("STITCH_DISCORD_TEST_GUILD")
    .unwrap()
    .parse()
    .unwrap();
  let rest = Arc::new(Rest {
    http: Arc::new(twilight_http::Client::new(token)),
  });
  // What a user would have said.
  let question = rest
    .send(
      channel,
      "(live test) @stitch what is a good first project in Rust?",
      None,
      &[],
    )
    .await
    .unwrap();
  let dir = tempfile::tempdir().unwrap();
  let bridge = Bridge::new(
    rest,
    Arc::new(Fake),
    Settings {
      guilds: [guild].into(),
      turns_per_hour: 5,
      state: dir.path().join("threads.json"),
      servers: None,
      approvers: Default::default(),
      home: None,
    },
  );
  bridge
    .clone()
    .handle(Incoming {
      guild: Some(guild),
      channel,
      message: Some(question),
      author: 1,
      author_name: "tester".into(),
      text: "what is a good first project in Rust?".into(),
      mentions_me: true,
      ..Incoming::default()
    })
    .await;
  let thread = std::fs::read_to_string(dir.path().join("threads.json")).unwrap();
  println!("mapped: {thread}");
  assert!(thread.contains("live-test"));
}
