//! The `edge-talk` skill (design §2.17): the user's edge talk history, its
//! chats and distilled memories, read in place from edge talk's own
//! database with `Db.Query` (read-only).
//!
//! - **Where the database is:** `edge-talk.db` in the `Documents/` of the
//!   app group `group.fragcolor.edge-talk`'s container: on macOS
//!   `~/Library/Group Containers/group.fragcolor.edge-talk/`, on iOS the
//!   container the app resolves (`FileManager.containerURL(...)`) and gives
//!   the node when it opens it ([`set_container`]).
//! - **Only where it is:** the skill, and so its tools, are listed on a node
//!   only while the file exists there.
//! - **Only for its tools:** a call of one of the built-in skill's tools gets
//!   the path in its `env` (`edge-talk-db`) and a read grant on that one
//!   file (`FsAccess::grant_read`); no other Otter tool, and no `otter_run`,
//!   can read it. A skill of the user's named `edge-talk` replaces the text
//!   and its scripts get neither.
//! - **Unasked:** they only read the user's own data from one fixed file,
//!   so they run without asking (in read-only threads too), unless
//!   `[otter] ask` names them.

use std::path::PathBuf;
use std::sync::Mutex;

/// The skill's name, as `crate::apps::SKILLS` lists it.
pub const SKILL: &str = "edge-talk";

/// The key of the database's path in its tools' `env`.
pub const ENV_KEY: &str = "edge-talk-db";

/// Edge talk's app group, which the iOS app joins to read it.
pub const GROUP: &str = "group.fragcolor.edge-talk";

/// The database, in the group container.
const FILE: &str = "Documents/edge-talk.db";

/// The group container the app resolved (iOS), set before the node opens.
static CONTAINER: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Where edge talk's group container is on this device, as the app found
/// it (`FileManager.containerURL(forSecurityApplicationGroupIdentifier:)`).
pub fn set_container(dir: PathBuf) {
  *CONTAINER.lock().expect("edge talk lock") = Some(dir);
}

/// The group container: the one the app gave, else macOS's known place
/// (not in unit tests, which must not depend on this machine's).
fn container() -> Option<PathBuf> {
  if let Some(dir) = CONTAINER.lock().expect("edge talk lock").clone() {
    return Some(dir);
  }
  if cfg!(all(target_os = "macos", not(test))) {
    let home = std::env::var_os("HOME")?;
    return Some(
      PathBuf::from(home)
        .join("Library/Group Containers")
        .join(GROUP),
    );
  }
  None
}

/// Edge talk's database on this node, when it is there.
pub fn db() -> Option<PathBuf> {
  db_in(container())
}

fn db_in(container: Option<PathBuf>) -> Option<PathBuf> {
  container.map(|c| c.join(FILE)).filter(|p| p.is_file())
}

#[cfg(test)]
mod tests {
  use std::path::Path;

  use serde_json::{Value, json};
  use stitch_records::RecordId;

  use crate::otter::host::Scripts;
  use crate::otter::{OtterSettings, OtterTools, Script, built_in_scripts, scripts_in};

  /// Edge talk's tables as its Scripts-Src/db.sql creates them, the parts
  /// the tools read (without cr-sqlite, whose triggers keep the FTS
  /// indexes; the fixture fills them as they do).
  const SCHEMA: &str = "
    CREATE TABLE actors (id TEXT PRIMARY KEY NOT NULL, name TEXT, system_prompt TEXT);
    CREATE TABLE conversations (
      id TEXT PRIMARY KEY NOT NULL, actor_id TEXT, title TEXT, pinned BOOLEAN DEFAULT 0,
      created_at TEXT DEFAULT CURRENT_TIMESTAMP, updated_at TEXT DEFAULT CURRENT_TIMESTAMP,
      settings_json TEXT, is_hidden BOOLEAN DEFAULT 0, parent_id TEXT, project_id TEXT,
      kind TEXT, memory_opt_out BOOLEAN DEFAULT 0);
    CREATE TABLE messages (
      id TEXT PRIMARY KEY NOT NULL, conversation_id TEXT, sender TEXT, message TEXT,
      created_at TEXT DEFAULT CURRENT_TIMESTAMP, tool_id TEXT, tool_name TEXT,
      is_system BOOLEAN DEFAULT 0, sequence_number INTEGER DEFAULT 0, reasoning TEXT,
      attachment BLOB, model TEXT, parent_message_id TEXT);
    CREATE INDEX idx_messages_conversation_ordering
      ON messages(conversation_id, created_at, sequence_number);
    CREATE TABLE memory (
      id TEXT PRIMARY KEY NOT NULL, actor_id TEXT, conversation_id TEXT, memory TEXT,
      type TEXT, plasticity REAL DEFAULT 0.5, importance REAL DEFAULT 0.5,
      access_count INTEGER DEFAULT 0, last_access TEXT,
      created_at TEXT DEFAULT CURRENT_TIMESTAMP);
    CREATE VIRTUAL TABLE memory_fts USING fts5(
      id UNINDEXED, memory, actor_id UNINDEXED, type UNINDEXED, content='',
      contentless_delete=1, contentless_unindexed=1,
      tokenize='porter unicode61 remove_diacritics 1');
    CREATE VIRTUAL TABLE messages_fts USING fts5(
      id UNINDEXED, message, content='', contentless_delete=1, contentless_unindexed=1,
      tokenize='porter unicode61 remove_diacritics 1');
    CREATE TABLE messages_fts_map (docid INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE);
    CREATE TABLE memory_fts_map (docid INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE);
  ";

  /// A database like edge talk's, in WAL mode, the connection that wrote
  /// it kept open (as edge talk keeps its own).
  fn fixture(path: &Path) -> rusqlite::Connection {
    let db = rusqlite::Connection::open(path).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    db.execute_batch(SCHEMA).unwrap();
    db.execute_batch(
      "INSERT INTO actors VALUES ('a1', 'Sage', '');
       INSERT INTO conversations (id, actor_id, title, created_at, updated_at) VALUES
         ('c1', 'a1', 'Otter tools in Rust', '2025-03-01 10:00:00', '2025-03-02 09:00:00'),
         ('c2', 'a1', 'Bathroom tiles', '2024-06-10 08:00:00', '2024-06-10 08:30:00'),
         ('c3', NULL, NULL, '2023-01-05 12:00:00', '2023-01-05 12:00:00');",
    )
    .unwrap();
    let long = format!(
      "{} The otter language compiles scripts while the host runs. {}",
      "Lead in. ".repeat(30),
      "More words follow here. ".repeat(40)
    );
    // id, conversation, sender, text, time, sequence, tool, system.
    type Row<'a> = (
      &'a str,
      &'a str,
      &'a str,
      &'a str,
      &'a str,
      i64,
      Option<&'a str>,
      i64,
    );
    let messages: &[Row] = &[
      (
        "m1",
        "c1",
        "user",
        "How do I write tools for the otter runtime in a host application?",
        "2025-03-01 10:00:00",
        0,
        None,
        0,
      ),
      (
        "m2",
        "c1",
        "assistant",
        &long,
        "2025-03-01 10:00:05",
        1,
        None,
        0,
      ),
      (
        "m3",
        "c1",
        "tool",
        "{\"result\": \"otter tool output\"}",
        "2025-03-01 10:00:06",
        2,
        Some("search"),
        0,
      ),
      (
        "m4",
        "c1",
        "user",
        "Thanks, and how does the scheduler of otter handle waiting meshes?",
        "2025-03-02 09:00:00",
        3,
        None,
        0,
      ),
      (
        "m5",
        "c2",
        "user",
        "Which tile sizes work best for a small bathroom floor and the walls?",
        "2024-06-10 08:00:00",
        0,
        None,
        0,
      ),
      (
        "m6",
        "c2",
        "system",
        "You are Sage.",
        "2024-06-10 07:59:00",
        0,
        None,
        1,
      ),
      (
        "m7",
        "c3",
        "user",
        "Remember that my sister lives in Osaka and visits in spring time.",
        "2023-01-05 12:00:00",
        0,
        None,
        0,
      ),
    ];
    for (id, cid, sender, text, at, seq, tool, sys) in messages {
      db.execute(
        "INSERT INTO messages (id, conversation_id, sender, message, created_at, sequence_number, tool_id, tool_name, is_system)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
        rusqlite::params![id, cid, sender, text, at, seq, tool, sys],
      )
      .unwrap();
      if tool.is_none() && *sys == 0 {
        db.execute("INSERT INTO messages_fts_map (id) VALUES (?1)", [id])
          .unwrap();
        db.execute(
          "INSERT INTO messages_fts (rowid, id, message)
           VALUES ((SELECT docid FROM messages_fts_map WHERE id = ?1), ?1, ?2)",
          [id, text],
        )
        .unwrap();
      }
    }
    let memories = [
      (
        "k1",
        "c3",
        "The user's sister lives in Osaka.",
        "fact",
        "2023-01-05 12:01:00",
      ),
      (
        "k2",
        "c1",
        "The user writes Rust and builds the otter language.",
        "fact",
        "2025-03-01 11:00:00",
      ),
      (
        "k3",
        "c2",
        "The user prefers large tiles.",
        "preference",
        "2024-06-10 09:00:00",
      ),
    ];
    for (id, cid, text, kind, at) in memories {
      db.execute(
        "INSERT INTO memory (id, actor_id, conversation_id, memory, type, created_at)
         VALUES (?1, 'a1', ?2, ?3, ?4, ?5)",
        [id, cid, text, kind, at],
      )
      .unwrap();
      db.execute("INSERT INTO memory_fts_map (id) VALUES (?1)", [id])
        .unwrap();
      db.execute(
        "INSERT INTO memory_fts (rowid, id, memory, actor_id, type)
         VALUES ((SELECT docid FROM memory_fts_map WHERE id = ?1), ?1, ?2, 'a1', ?3)",
        [id, text, kind],
      )
      .unwrap();
    }
    db
  }

  struct Setup {
    dir: tempfile::TempDir,
    db: std::path::PathBuf,
    _writer: rusqlite::Connection,
  }

  fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    // As on a device: the database in a container of its own, outside
    // the thread's directory.
    let container = dir.path().join("container/Documents");
    std::fs::create_dir_all(&container).unwrap();
    std::fs::create_dir_all(dir.path().join("work")).unwrap();
    let db = container.join("edge-talk.db");
    let writer = fixture(&db);
    Setup {
      dir,
      db,
      _writer: writer,
    }
  }

  fn skill() -> Vec<Script> {
    let b = crate::apps::SKILLS
      .iter()
      .find(|b| b.name == super::SKILL)
      .unwrap();
    built_in_scripts(b)
  }

  fn tools(s: &Setup, scripts: &[Script], settings: OtterSettings) -> OtterTools {
    let mut t = OtterTools::new(&Scripts::default(), scripts, settings);
    t.cwd = s.dir.path().join("work");
    t.scratch = s.dir.path().join("scratch");
    t.edge_talk = Some(s.db.clone());
    t
  }

  fn thread() -> RecordId {
    "2026-10-20T120000Z-mbp-k3f9".parse().unwrap()
  }

  async fn call(t: &OtterTools, name: &str, input: Value) -> (bool, String) {
    let out = t.call(&thread(), name, &input).await;
    (out.ok, out.output)
  }

  #[tokio::test]
  async fn the_tools_search_and_read_edge_talks_database() {
    let s = setup();
    let t = tools(&s, &skill(), Default::default());
    let names: Vec<String> = t.definitions().into_iter().map(|d| d.name).collect();
    assert_eq!(
      names,
      [
        "edge_talk_conversations",
        "edge_talk_memories",
        "edge_talk_read",
        "edge_talk_search"
      ]
    );

    // Search: nobody to ask, and it runs (unasked by default).
    let (ok, out) = call(&t, "edge_talk_search", json!({"query": "otter"})).await;
    assert!(ok, "{out}");
    assert!(
      out.starts_with("3 edge talk messages matching \"otter\""),
      "{out}"
    );
    assert!(out.contains("[c1] Otter tools in Rust"), "{out}");
    assert!(
      out.contains("user: How do I write tools for the otter runtime"),
      "{out}"
    );
    // A long message's snippet is cut around the word.
    assert!(out.contains("assistant: …"), "{out}");
    assert!(out.contains("The otter language compiles"), "{out}");
    // Tool rows are not indexed.
    assert!(!out.contains("tool output"), "{out}");
    // Stemmed: "tile size" finds "tile sizes"; a date range narrows.
    let (ok, out) = call(&t, "edge_talk_search", json!({"query": "Tile size?"})).await;
    assert!(ok && out.contains("[c2] Bathroom tiles"), "{out}");
    let (ok, out) = call(
      &t,
      "edge_talk_search",
      json!({"query": "otter", "from": "2025-03-02", "to": "2025-03-02", "limit": 5}),
    )
    .await;
    assert!(ok && out.starts_with("1 edge talk messages"), "{out}");
    assert!(out.contains("scheduler"), "{out}");
    // All the words, else any of them; operators are only words.
    let (ok, out) = call(&t, "edge_talk_search", json!({"query": "otter Osaka"})).await;
    assert!(ok && out.contains("any of them"), "{out}");
    assert!(out.contains("[c3] (untitled)"), "{out}");
    let (ok, out) = call(&t, "edge_talk_search", json!({"query": "NEAR(\"x\" OR"})).await;
    assert!(
      ok && out.starts_with("No edge talk message matches"),
      "{out}"
    );
    let (ok, out) = call(&t, "edge_talk_search", json!({"query": "  "})).await;
    assert!(!ok && out.contains("query is required"), "{out}");
    let (ok, out) = call(
      &t,
      "edge_talk_search",
      json!({"query": "otter", "from": "May"}),
    )
    .await;
    assert!(!ok && out.contains("from must be a date"), "{out}");

    // Memories: searched, and the latest.
    let (ok, out) = call(&t, "edge_talk_memories", json!({"query": "sister"})).await;
    assert!(ok, "{out}");
    assert!(
      out.contains("- 2023-01-05 (fact), Sage [c3]: The user's sister lives in Osaka."),
      "{out}"
    );
    let (ok, out) = call(&t, "edge_talk_memories", json!({"limit": 2})).await;
    assert!(
      ok && out.starts_with("The 2 latest edge talk memories:"),
      "{out}"
    );
    assert!(
      out.contains("otter language") && out.contains("large tiles"),
      "{out}"
    );
    assert!(!out.contains("Osaka"), "{out}");

    // Conversations: the latest first, by title and dates.
    let (ok, out) = call(&t, "edge_talk_conversations", json!({})).await;
    assert!(ok, "{out}");
    let c1 = out.find("[c1]").unwrap();
    assert!(
      c1 < out.find("[c2]").unwrap() && out.contains("[c3]"),
      "{out}"
    );
    assert!(
      out.contains("- [c1] Otter tools in Rust (2025-03-01 to 2025-03-02, 4 messages, Sage)"),
      "{out}"
    );
    let (ok, out) = call(&t, "edge_talk_conversations", json!({"title": "bathroom"})).await;
    assert!(ok && out.contains("[c2]") && !out.contains("[c1]"), "{out}");
    let (ok, out) = call(
      &t,
      "edge_talk_conversations",
      json!({"from": "2024-01-01", "to": "2024-12-31"}),
    )
    .await;
    assert!(
      ok && out.contains("[c2]") && !out.contains("[c1]") && !out.contains("[c3]"),
      "{out}"
    );
    let (ok, out) = call(&t, "edge_talk_conversations", json!({"limit": 1})).await;
    assert!(ok && out.contains("More with offset 1."), "{out}");

    // Read: in order, tool and system rows left out, paged, cut.
    let (ok, out) = call(&t, "edge_talk_read", json!({"id": "c1", "max-chars": 200})).await;
    assert!(ok, "{out}");
    assert!(
      out.starts_with("Otter tools in Rust [c1] (2025-03-01 10:00 to 2025-03-02 09:00, with Sage): messages 1 to 3 of 3"),
      "{out}"
    );
    assert!(
      out.contains("#1 2025-03-01 10:00 user:\nHow do I write"),
      "{out}"
    );
    assert!(out.contains("more characters]"), "{out}");
    assert!(out.contains("#3 2025-03-02 09:00 user:"), "{out}");
    assert!(!out.contains("tool output"), "{out}");
    let (ok, out) = call(
      &t,
      "edge_talk_read",
      json!({"id": "c1", "include-tools": true, "offset": 2, "limit": 1}),
    )
    .await;
    assert!(ok && out.contains("messages 3 to 3 of 4"), "{out}");
    assert!(
      out.contains("#3 2025-03-01 10:00 tool (tool search):"),
      "{out}"
    );
    assert!(out.contains("More: offset 3."), "{out}");
    let (ok, out) = call(
      &t,
      "edge_talk_read",
      json!({"id": "c2", "include-tools": true}),
    )
    .await;
    assert!(
      ok && out.contains("system (system):\nYou are Sage."),
      "{out}"
    );
    let (ok, out) = call(&t, "edge_talk_read", json!({"id": "nope"})).await;
    assert!(
      !ok && out.contains("no edge talk conversation has the id nope"),
      "{out}"
    );
  }

  #[tokio::test]
  async fn they_run_unasked_unless_ask_says_and_in_read_only_threads() {
    let s = setup();
    let mut t = tools(&s, &skill(), Default::default());
    t.read_only = true;
    let (ok, out) = call(&t, "edge_talk_search", json!({"query": "otter"})).await;
    assert!(ok, "{out}");
    let t = tools(
      &s,
      &skill(),
      OtterSettings {
        ask: vec!["edge_talk_*".into()],
        ..Default::default()
      },
    );
    let (ok, out) = call(&t, "edge_talk_search", json!({"query": "otter"})).await;
    assert!(!ok && out.contains("nobody is here"), "{out}");
    // Without the database here, they say so.
    let mut t = tools(&s, &skill(), Default::default());
    t.edge_talk = None;
    let (ok, out) = call(&t, "edge_talk_search", json!({"query": "otter"})).await;
    assert!(!ok && out.contains("not on this node"), "{out}");
  }

  #[tokio::test]
  async fn no_other_tool_reads_the_database() {
    let s = setup();
    let db = s.db.display().to_string();
    // A skill of the user's whose tool queries the path it is given.
    let probe = r#"
@fn(ProbeDb input: {input: {} env: {} working-dir: String} output: String params: {} {
  input.input.db | If({IsString} {ExpectString} {""}) = db
  input.env.edge-talk-db | If({IsString} {ExpectString} {db}) = path
  none | Db.Query("SELECT count(*) AS n FROM messages" path) = rows
  rows | Count | ToString
})
{definition: {name: "probe" description: "Probes."} use: "ProbeDb"}
"#;
    let home = s.dir.path().join("home");
    let mut scripts = Vec::new();
    for name in ["mine", super::SKILL] {
      let dir = home.join(name);
      std::fs::create_dir_all(dir.join("tools")).unwrap();
      std::fs::write(dir.join("tools/probe.otr"), probe).unwrap();
      scripts.extend(scripts_in(name, &dir));
    }
    // Both the user's skill and one named `edge-talk` (files, not the
    // built-in): yolo, so nothing asks, and neither reads it.
    for script in scripts {
      let mut t = tools(&s, &[script], Default::default());
      t.yolo = true;
      let (ok, out) = call(&t, "probe", json!({"db": db})).await;
      assert!(
        !ok && out.contains("outside the allowed directory"),
        "{out}"
      );
    }
    // Nor does otter_run.
    let mut t = tools(&s, &[], Default::default());
    t.language = true;
    t.yolo = true;
    let code = format!("none | Db.Query(\"SELECT count(*) AS n FROM messages\" \"{db}\")");
    let (ok, out) = call(&t, "otter_run", json!({"code": code})).await;
    assert!(out.contains("outside the allowed directory"), "{ok} {out}");
    // The built-in's, given it, does.
    let t = tools(&s, &skill(), Default::default());
    assert!(call(&t, "edge_talk_conversations", json!({})).await.0);
  }

  /// The tools on this Mac's own edge talk database, printed:
  /// `cargo test -p stitch-node --lib edge_talk::tests::on_this_macs_database -- --ignored --nocapture`.
  #[tokio::test]
  #[ignore = "reads this machine's edge talk database"]
  async fn on_this_macs_database() {
    let home = std::env::var_os("HOME").unwrap();
    let container = Path::new(&home)
      .join("Library/Group Containers")
      .join(super::GROUP);
    let db = super::db_in(Some(container)).expect("no edge talk database here");
    let dir = tempfile::tempdir().unwrap();
    let mut t = OtterTools::new(&Scripts::default(), &skill(), Default::default());
    t.cwd = dir.path().to_owned();
    t.scratch = dir.path().join("scratch");
    t.edge_talk = Some(db);
    let query = std::env::var("EDGE_TALK_QUERY").unwrap_or_else(|_| "otter".into());
    let started = std::time::Instant::now();
    let (ok, out) = call(&t, "edge_talk_search", json!({"query": query, "limit": 5})).await;
    println!(
      "== edge_talk_search ({:?}, ok {ok})\n{out}\n",
      started.elapsed()
    );
    let cid = out
      .split('[')
      .nth(1)
      .and_then(|s| s.split(']').next())
      .map(str::to_owned);
    for (name, input) in [
      ("edge_talk_memories", json!({"query": query, "limit": 5})),
      (
        "edge_talk_conversations",
        json!({"title": query, "limit": 5}),
      ),
      (
        "edge_talk_read",
        json!({"id": cid, "limit": 4, "max-chars": 300}),
      ),
    ] {
      let started = std::time::Instant::now();
      let (ok, out) = call(&t, name, input).await;
      println!("== {name} ({:?}, ok {ok})\n{out}\n", started.elapsed());
    }
  }

  #[test]
  fn the_database_is_in_the_group_container() {
    let dir = tempfile::tempdir().unwrap();
    let c = Some(dir.path().to_owned());
    assert_eq!(super::db_in(c.clone()), None);
    std::fs::create_dir_all(dir.path().join("Documents")).unwrap();
    assert_eq!(super::db_in(c.clone()), None);
    std::fs::write(dir.path().join("Documents/edge-talk.db"), "").unwrap();
    assert_eq!(
      super::db_in(c),
      Some(dir.path().join("Documents/edge-talk.db"))
    );
    assert_eq!(super::db_in(None), None);
  }
}
