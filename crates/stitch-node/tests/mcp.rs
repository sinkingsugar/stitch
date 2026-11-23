//! The MCP client against a tiny stdio MCP server: this test binary
//! itself, started again with `STITCH_MCP_TEST_SERVER` set (so the
//! harness is ours: libtest would print onto the server's stdout).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_node::mcp::{Mcp, McpSettings, McpTools, ServerSettings};
use stitch_node::rules::{Granted, Live};
use stitch_node::tools::{ApprovalRequest, Approver, Decision};
use stitch_records::{NodeName, RecordId};

const SERVER: &str = "STITCH_MCP_TEST_SERVER";
const LOG: &str = "STITCH_MCP_TEST_LOG";
/// Set: the server answers `tools/list` once, then never again.
const STALL: &str = "STITCH_MCP_TEST_STALL";
/// A 1×1 PNG.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn main() {
  if std::env::var_os(SERVER).is_some() {
    serve();
    return;
  }
  let rt = tokio::runtime::Runtime::new().unwrap();
  rt.block_on(tests());
  println!("mcp: all passed");
}

// ---- the server ----

fn log(line: &str) {
  if let Some(p) = std::env::var_os(LOG) {
    let mut f = std::fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(p)
      .unwrap();
    writeln!(f, "{line}").unwrap();
  }
}

fn send(out: &Arc<Mutex<std::io::Stdout>>, v: Value) {
  let mut o = out.lock().unwrap();
  writeln!(o, "{v}").unwrap();
  o.flush().unwrap();
}

fn tool(name: &str, description: &str) -> Value {
  json!({"name": name, "description": description,
         "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}, "ms": {"type": "integer"}}}})
}

fn text(t: &str) -> Value {
  json!({"content": [{"type": "text", "text": t}]})
}

fn serve() {
  log(&format!("started {}", std::process::id()));
  let out = Arc::new(Mutex::new(std::io::stdout()));
  let extra = Arc::new(Mutex::new(false));
  let mut lists = 0;
  for line in std::io::stdin().lock().lines() {
    let Ok(line) = line else { break };
    let Ok(msg) = serde_json::from_str::<Value>(&line) else {
      continue;
    };
    let method = msg["method"].as_str().unwrap_or("");
    let id = msg.get("id").cloned();
    let p = &msg["params"];
    let result = match method {
      "initialize" => json!({
        "protocolVersion": "2025-06-18",
        "capabilities": {"tools": {"listChanged": true}},
        "serverInfo": {"name": "tiny", "version": "1"}
      }),
      "notifications/cancelled" => {
        log(&format!("cancelled {}", p["requestId"]));
        continue;
      }
      "ping" => json!({}),
      "tools/list" => {
        lists += 1;
        if lists > 1 && std::env::var_os(STALL).is_some() {
          continue;
        }
        let mut tools = vec![
          tool("echo", "Says the text back."),
          tool("picture", "A picture."),
          tool("fail", "Fails."),
          tool("slow", "Answers after ms."),
          tool("crash", "Exits."),
          tool("more", "Adds a tool."),
          tool("dotted.name", "A name with a dot."),
          tool("hidden", "Not offered."),
          tool("spawn", "Starts a child process."),
          tool("long", "Much text."),
        ];
        if *extra.lock().unwrap() {
          tools.push(tool("extra", "Added later."));
        }
        json!({"tools": tools})
      }
      "tools/call" => {
        let args = &p["arguments"];
        match p["name"].as_str().unwrap_or("") {
          "echo" => text(args["text"].as_str().unwrap_or("")),
          "picture" => json!({"content": [
            {"type": "text", "text": "here it is"},
            {"type": "image", "data": PNG, "mimeType": "image/png"},
            {"type": "resource_link", "uri": "file:///x.txt", "name": "x"}
          ]}),
          "fail" => json!({"content": [{"type": "text", "text": "it broke"}], "isError": true}),
          "slow" => {
            let ms = args["ms"].as_u64().unwrap_or(1000);
            let (out, id) = (out.clone(), id.clone());
            std::thread::spawn(move || {
              std::thread::sleep(Duration::from_millis(ms));
              send(
                &out,
                json!({"jsonrpc": "2.0", "id": id, "result": text("slept")}),
              );
            });
            continue;
          }
          "crash" => std::process::exit(3),
          "more" => {
            *extra.lock().unwrap() = true;
            send(
              &out,
              json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}),
            );
            text("added")
          }
          "dotted.name" => json!({"content": [], "structuredContent": {"n": 1}}),
          // What the server starts is in its process group.
          "spawn" => {
            // Never waited for: it must outlive this call, and be stopped
            // with the server's process group.
            // It ignores SIGTERM (and says so once it does), so only the
            // group's SIGKILL stops it, though the server itself exits
            // as soon as its input closes.
            #[allow(clippy::zombie_processes)]
            std::process::Command::new("sh")
              .args([
                "-c",
                "trap '' TERM; echo \"child $$\" >> \"$STITCH_MCP_TEST_LOG\"; exec sleep 60",
              ])
              .spawn()
              .unwrap();
            text("spawned")
          }
          "long" => json!({"content": [
            {"type": "text", "text": "x".repeat(40_000)},
            {"type": "resource", "resource": {"uri": "mem://note", "text": "a note"}}
          ]}),
          other => text(&format!("no tool {other}")),
        }
      }
      _ if id.is_none() => continue,
      _ => {
        send(
          &out,
          json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "unknown"}}),
        );
        continue;
      }
    };
    send(&out, json!({"jsonrpc": "2.0", "id": id, "result": result}));
  }
}

// ---- the tests ----

fn settings(log: &Path, timeout: u64) -> ServerSettings {
  let mut env = std::collections::BTreeMap::new();
  env.insert(SERVER.to_owned(), "1".to_owned());
  env.insert(LOG.to_owned(), log.display().to_string());
  ServerSettings {
    command: Some(std::env::current_exe().unwrap().display().to_string()),
    env,
    tools: vec!["*".into(), "hidden-not".into()],
    allow: vec![
      "echo".into(),
      "picture".into(),
      "fail".into(),
      "s*".into(),
      "crash".into(),
      "more".into(),
      "dotted.name".into(),
      "long".into(),
      "spawn".into(),
    ],
    timeout_secs: Some(timeout),
    ..ServerSettings::default()
  }
}

fn log_of(p: &Path) -> String {
  std::fs::read_to_string(p).unwrap_or_default()
}

async fn eventually(what: &str, mut f: impl FnMut() -> bool) {
  for _ in 0..100 {
    if f() {
      return;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  panic!("never: {what}");
}

fn alive(pid: &str) -> bool {
  std::process::Command::new("kill")
    .args(["-0", pid])
    .stderr(std::process::Stdio::null())
    .status()
    .is_ok_and(|s| s.success())
}

async fn tests() {
  let dir = tempfile::tempdir().unwrap();
  let log = dir.path().join("server.log");
  let scratch: PathBuf = dir.path().join("downloads");
  let hub = Mcp::new(dir.path());
  let mut servers = std::collections::BTreeMap::new();
  let mut tiny = settings(&log, 1);
  tiny.tools = vec!["*".into()];
  servers.insert("tiny".to_owned(), tiny);
  // A server that never starts offers nothing and stops nothing else.
  servers.insert(
    "broken".to_owned(),
    ServerSettings {
      command: Some("/nonexistent/mcp-server".into()),
      ..ServerSettings::default()
    },
  );
  hub.configure(&servers);
  let thread = RecordId::random(jiff::Timestamp::now(), &"test".parse::<NodeName>().unwrap());

  // Lazily started: nothing runs until a loop prepares its tools.
  assert_eq!(log_of(&log), "");
  let tools = McpTools::new(&hub, &["tiny".into(), "broken".into()], scratch.clone());
  tools.prepare().await;
  let names: Vec<String> = tools.definitions().into_iter().map(|d| d.name).collect();
  assert_eq!(
    names,
    [
      // Said down, not left out silently.
      "broken__unavailable",
      "tiny__crash",
      "tiny__dotted_name",
      "tiny__echo",
      "tiny__fail",
      "tiny__hidden",
      "tiny__long",
      "tiny__more",
      "tiny__picture",
      "tiny__slow",
      "tiny__spawn"
    ]
  );
  let def = tools
    .definitions()
    .into_iter()
    .find(|d| d.name == "tiny__echo")
    .unwrap();
  assert_eq!(def.description, "MCP server tiny: Says the text back.");
  assert_eq!(def.input_schema["properties"]["text"]["type"], "string");
  assert_eq!(log_of(&log).matches("started").count(), 1);

  // An enable list of patterns keeps the list small.
  let few = McpTools::new(
    &hub,
    &["tiny__e*".into(), "tiny__fail".into()],
    scratch.clone(),
  );
  few.prepare().await;
  let names: Vec<String> = few.definitions().into_iter().map(|d| d.name).collect();
  assert_eq!(names, ["tiny__echo", "tiny__fail"]);
  // Shared: still the one server.
  assert_eq!(log_of(&log).matches("started").count(), 1);

  // Text, images, failures, structured content.
  let o = tools
    .call(&thread, "tiny__echo", &json!({"text": "hi"}))
    .await;
  assert!(o.ok, "{o:?}");
  assert_eq!(o.output, "hi");
  let o = tools.call(&thread, "tiny__picture", &json!({})).await;
  assert!(o.ok);
  assert!(o.output.contains("here it is"), "{}", o.output);
  assert!(
    o.output.contains("[resource link: file:///x.txt x]"),
    "{}",
    o.output
  );
  assert_eq!(o.images.len(), 1);
  assert_eq!(o.images[0].media_type, "image/png");
  assert!(o.images[0].path.is_file());
  let o = tools.call(&thread, "tiny__fail", &json!({})).await;
  assert!(!o.ok);
  assert_eq!(o.output, "it broke");
  let o = tools.call(&thread, "tiny__dotted_name", &json!({})).await;
  assert!(o.ok);
  assert!(o.output.contains("\"n\": 1"), "{}", o.output);
  // Long text is cut, the whole saved; embedded text resources inlined.
  let o = tools.call(&thread, "tiny__long", &json!({})).await;
  assert!(o.ok, "{}", &o.output[..o.output.len().min(300)]);
  assert!(o.output.len() < 33_000, "{}", o.output.len());
  let saved = o
    .output
    .split("all of it is in ")
    .nth(1)
    .and_then(|s| s.strip_suffix(']'))
    .unwrap_or_else(|| panic!("no saved path in {}", &o.output[32_000..]));
  let whole = std::fs::read_to_string(saved).unwrap();
  assert!(
    whole.ends_with("[resource mem://note]\na note"),
    "{}",
    &whole[40_000..]
  );

  // Timeouts cancel the call on the server.
  let o = tools
    .call(&thread, "tiny__slow", &json!({"ms": 3000}))
    .await;
  assert!(!o.ok);
  assert!(
    o.output.contains("did not answer within 1s"),
    "{}",
    o.output
  );
  eventually("the timeout cancelled", || {
    log_of(&log).contains("cancelled")
  })
  .await;
  // So does dropping the call (the user interrupted it).
  let before = log_of(&log).matches("cancelled").count();
  let dropped = tokio::time::timeout(
    Duration::from_millis(200),
    tools.call(&thread, "tiny__slow", &json!({"ms": 900})),
  )
  .await;
  assert!(dropped.is_err());
  eventually("the interrupt cancelled", || {
    log_of(&log).matches("cancelled").count() > before
  })
  .await;
  // Concurrent calls share the connection.
  let (slow, echo) = (json!({"ms": 300}), json!({"text": "meanwhile"}));
  let (a, b) = tokio::join!(
    tools.call(&thread, "tiny__slow", &slow),
    tools.call(&thread, "tiny__echo", &echo),
  );
  assert_eq!(
    (a.output.as_str(), b.output.as_str()),
    ("slept", "meanwhile")
  );

  // The tool list follows list_changed.
  let o = tools.call(&thread, "tiny__more", &json!({})).await;
  assert!(o.ok);
  for _ in 0..100 {
    tools.prepare().await;
    if tools.has("tiny__extra") {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  assert!(tools.has("tiny__extra"), "listed again after list_changed");

  // A crash fails the call in flight, and the next call starts it again.
  let o = tools.call(&thread, "tiny__crash", &json!({})).await;
  assert!(!o.ok);
  assert!(o.output.contains("stopped during the call"), "{}", o.output);
  let o = tools
    .call(&thread, "tiny__echo", &json!({"text": "again"}))
    .await;
  assert!(o.ok, "{o:?}");
  assert_eq!(o.output, "again");
  assert_eq!(log_of(&log).matches("started").count(), 2);

  // Approvals: what `allow` does not cover asks; a decline says why.
  let mut asking = settings(&log, 5);
  asking.allow = vec!["echo".into()];
  // A server's `tools` limits what it offers anywhere.
  asking.tools = vec!["e*".into(), "f*".into(), "picture".into()];
  servers.insert("ask".to_owned(), asking);
  hub.configure(&servers);
  let asked: Arc<Mutex<Vec<ApprovalRequest>>> = Arc::default();
  let answer = Arc::new(Mutex::new(Decision::decline(Some("not now".into()))));
  let (a2, ans2) = (asked.clone(), answer.clone());
  let approver: Approver = Arc::new(move |req: ApprovalRequest| {
    a2.lock().unwrap().push(req);
    let d = ans2.lock().unwrap().clone();
    Box::pin(async move { d })
  });
  let mut gated = McpTools::new(&hub, &["ask".into()], scratch.clone());
  gated.approver = Some(approver);
  gated.prepare().await;
  let names: Vec<String> = gated.definitions().into_iter().map(|d| d.name).collect();
  assert_eq!(names, ["ask__echo", "ask__fail", "ask__picture"]);
  let o = gated
    .call(&thread, "ask__echo", &json!({"text": "free"}))
    .await;
  assert_eq!(o.output, "free");
  assert!(asked.lock().unwrap().is_empty());
  let o = gated.call(&thread, "ask__fail", &json!({"x": 1})).await;
  assert!(!o.ok);
  assert_eq!(o.output, "the user declined this call: not now");
  {
    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].session, "mcp:ask");
    assert_eq!(asked[0].command, "ask__fail {\"x\":1}");
    // "Always" would allow the tool, unprefixed, of that server.
    assert_eq!(asked[0].grant, ["fail"]);
    assert_eq!(asked[0].grant_server.as_deref(), Some("ask"));
    assert!(!asked[0].allowed_now());
  }
  *answer.lock().unwrap() = Decision::allow();
  let o = gated.call(&thread, "ask__picture", &json!({})).await;
  assert!(
    o.output.starts_with("(approved by the user)\n"),
    "{}",
    o.output
  );
  // Granted in the thread's place since ("always"): a question waiting
  // on it goes, and it asks nothing from then on; another server's tool
  // of that name is not granted.
  let granted: Arc<Mutex<std::collections::BTreeMap<String, Vec<String>>>> = Arc::default();
  let g = granted.clone();
  gated.live = Live::new(move || Granted {
    mcp: g.lock().unwrap().clone(),
    ..Granted::default()
  });
  granted
    .lock()
    .unwrap()
    .insert("tiny".to_owned(), vec!["fail".to_owned()]);
  let o = gated.call(&thread, "ask__fail", &json!({})).await;
  assert!(
    o.output.starts_with("(approved by the user)\n"),
    "{}",
    o.output
  );
  assert_eq!(asked.lock().unwrap().len(), 3);
  assert!(!asked.lock().unwrap()[2].allowed_now());
  granted
    .lock()
    .unwrap()
    .insert("ask".to_owned(), vec!["f*".to_owned()]);
  assert!(asked.lock().unwrap()[2].allowed_now());
  let o = gated.call(&thread, "ask__fail", &json!({})).await;
  assert_eq!(o.output, "it broke");
  assert_eq!(asked.lock().unwrap().len(), 3);
  // Yolo asks nothing.
  gated.yolo = true;
  let o = gated.call(&thread, "ask__fail", &json!({})).await;
  assert_eq!(o.output, "it broke");
  assert_eq!(asked.lock().unwrap().len(), 3);

  // A server that does not answer `tools/list` stalls no call meanwhile,
  // and is not asked again before its backoff: its tools stand.
  let mut stalling = settings(&log, 5);
  stalling.env.insert(STALL.to_owned(), "1".to_owned());
  stalling.startup_timeout_secs = Some(2);
  servers.insert("stall".to_owned(), stalling);
  hub.configure(&servers);
  let stalled = Arc::new(McpTools::new(&hub, &["stall".into()], scratch.clone()));
  stalled.prepare().await;
  assert!(stalled.has("stall__echo"));
  // It says its tools changed; listing them again never ends.
  let o = stalled.call(&thread, "stall__more", &json!({})).await;
  assert!(o.ok, "{o:?}");
  tokio::time::sleep(Duration::from_millis(200)).await;
  let s2 = stalled.clone();
  let listing = tokio::spawn(async move { s2.prepare().await });
  tokio::time::sleep(Duration::from_millis(300)).await;
  let started = std::time::Instant::now();
  let o = stalled
    .call(&thread, "stall__echo", &json!({"text": "meanwhile"}))
    .await;
  assert_eq!(o.output, "meanwhile");
  assert!(
    started.elapsed() < Duration::from_secs(1),
    "{:?}",
    started.elapsed()
  );
  listing.await.unwrap();
  assert!(stalled.has("stall__echo") && !stalled.has("stall__extra"));
  let started = std::time::Instant::now();
  stalled.prepare().await;
  assert!(
    started.elapsed() < Duration::from_millis(500),
    "listed again before its backoff: {:?}",
    started.elapsed()
  );
  assert!(stalled.has("stall__echo"));
  drop(stalled);

  // Stopped with the node: every server process ends, and what it
  // started (its process group) with it.
  let o = tools.call(&thread, "tiny__spawn", &json!({})).await;
  assert!(o.ok, "{o:?}");
  let lines = |prefix: &str| -> Vec<String> {
    log_of(&log)
      .lines()
      .filter_map(|l| l.strip_prefix(prefix).map(str::to_owned))
      .collect()
  };
  eventually("the child started", || !lines("child ").is_empty()).await;
  let child = lines("child ").pop().expect("the child's pid");
  assert!(alive(&child));
  let pids = lines("started ");
  // A loop still holding a server once they were stopped.
  let held = McpTools::new(&hub, &["tiny".into()], scratch.clone());
  held.prepare().await;
  drop((tools, few, gated));
  hub.stop_all().await;
  // Nothing starts again: not what a loop held, nor what is configured
  // after (a turn starting, a remote call, a publication).
  let o = held
    .call(&thread, "tiny__echo", &json!({"text": "after"}))
    .await;
  assert!(!o.ok && o.output.contains("was stopped"), "{o:?}");
  hub.configure(&servers);
  let after = McpTools::new(&hub, &["tiny".into()], scratch.clone());
  after.prepare().await;
  assert!(after.is_empty() && after.definitions().is_empty());
  drop((held, after));
  assert_eq!(lines("started ").len(), pids.len());
  for pid in pids.iter().chain([&child]) {
    eventually("the processes stopped", || !alive(pid)).await;
  }

  enabled_by_project();
  streamable_http().await;
  unreachable().await;
}

/// A server enabled for the thread that cannot be reached (an HTTP one
/// whose port refuses, never listed): its tools are missing, and the
/// thread is told so by a stub tool saying the error, which tries again;
/// a call of one of its tools says it too (issue a2vf).
async fn unreachable() {
  let dir = tempfile::tempdir().unwrap();
  let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
  let addr = listener.local_addr().unwrap();
  drop(listener);
  let mut servers = std::collections::BTreeMap::new();
  servers.insert(
    "mail".to_owned(),
    ServerSettings {
      url: Some(format!("http://{addr}/mcp")),
      approval: Some("allow".into()),
      ..ServerSettings::default()
    },
  );
  let hub = Mcp::new(dir.path());
  hub.configure(&servers);
  let thread = RecordId::random(jiff::Timestamp::now(), &"test".parse::<NodeName>().unwrap());
  let tools = McpTools::new(&hub, &["mail".into()], dir.path().join("downloads"));
  tools.prepare().await;
  let defs = tools.definitions();
  let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
  assert_eq!(names, ["mail__unavailable"]);
  assert!(
    defs[0].description.contains("could not be reached")
      && defs[0].description.contains("did not start"),
    "{}",
    defs[0].description
  );
  assert!(tools.has("mail__unavailable") && tools.is_here("mail__unavailable"));
  // Its tools, called anyway: said down, with the error.
  let why = tools.why_not(&thread, "mail__search").unwrap();
  assert!(
    why.contains("could not be reached") && why.contains("did not start"),
    "{why}"
  );
  // The stub tries again: still down.
  let o = tools.call(&thread, "mail__unavailable", &json!({})).await;
  assert!(!o.ok && o.output.contains("MCP server mail"), "{o:?}");

  // Back: the stub says so, and the next step lists its tools.
  let back = tokio::net::TcpListener::bind(addr).await.unwrap();
  http_endpoint_on(back);
  let mut o = tools.call(&thread, "mail__unavailable", &json!({})).await;
  for _ in 0..80 {
    if o.ok {
      break;
    }
    // Waiting out the start's backoff.
    tokio::time::sleep(Duration::from_millis(100)).await;
    o = tools.call(&thread, "mail__unavailable", &json!({})).await;
  }
  assert!(o.ok && o.output.contains("is back"), "{o:?}");
  tools.prepare().await;
  assert!(tools.has("mail__echo") && !tools.has("mail__unavailable"));
  drop(tools);
  hub.stop_all().await;
}

/// A streamable HTTP endpoint, by hand: JSON answers to POSTs (no SSE),
/// a session ID from `initialize` on, no stream on GET. `echo` answers
/// with the headers it was sent.
async fn http_endpoint() -> String {
  http_endpoint_on(tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap())
}

fn http_endpoint_on(listener: tokio::net::TcpListener) -> String {
  use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
  let url = format!("http://{}/mcp", listener.local_addr().unwrap());
  tokio::spawn(async move {
    loop {
      let Ok((stream, _)) = listener.accept().await else {
        return;
      };
      tokio::spawn(async move {
        let mut r = BufReader::new(stream);
        loop {
          let mut head = Vec::new();
          loop {
            let mut line = String::new();
            if r.read_line(&mut line).await.unwrap_or(0) == 0 {
              return;
            }
            if line == "\r\n" {
              break;
            }
            head.push(line.trim_end().to_owned());
          }
          let header = |name: &str| {
            head.iter().find_map(|h| {
              let (k, v) = h.split_once(':')?;
              k.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
            })
          };
          let len: usize = header("content-length")
            .and_then(|l| l.parse().ok())
            .unwrap_or(0);
          let mut body = vec![0; len];
          r.read_exact(&mut body).await.unwrap();
          let (status, extra, reply) = if !head[0].starts_with("POST") {
            ("405 Method Not Allowed", "", String::new())
          } else {
            let msg: Value = serde_json::from_slice(&body).unwrap();
            let id = msg.get("id").cloned();
            let result = match msg["method"].as_str().unwrap_or("") {
              "initialize" => Some(json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "tiny-http", "version": "1"}
              })),
              "tools/list" => Some(json!({"tools": [tool("echo", "Says the headers back.")]})),
              "tools/call" => Some(text(&format!(
                "auth={} custom={} session={}",
                header("authorization").unwrap_or_default(),
                header("x-custom").unwrap_or_default(),
                header("mcp-session-id").unwrap_or_default()
              ))),
              _ => None,
            };
            match (id, result) {
              (Some(id), Some(result)) => (
                "200 OK",
                "Content-Type: application/json\r\nMcp-Session-Id: s1\r\n",
                json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string(),
              ),
              _ => ("202 Accepted", "", String::new()),
            }
          };
          let out = format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\n\r\n{reply}",
            reply.len()
          );
          if r.get_mut().write_all(out.as_bytes()).await.is_err() {
            return;
          }
        }
      });
    }
  });
  url
}

async fn streamable_http() {
  let dir = tempfile::tempdir().unwrap();
  stitch_node::keys::set(
    dir.path(),
    "tiny-http",
    "tok",
    stitch_node::keys::Store::File,
  )
  .unwrap();
  let mut headers = std::collections::BTreeMap::new();
  headers.insert("X-Custom".to_owned(), "yes".to_owned());
  let mut servers = std::collections::BTreeMap::new();
  servers.insert(
    "web".to_owned(),
    ServerSettings {
      url: Some(http_endpoint().await),
      key: Some("tiny-http".into()),
      headers,
      approval: Some("allow".into()),
      ..ServerSettings::default()
    },
  );
  let hub = Mcp::new(dir.path());
  hub.configure(&servers);
  // A server named like a built-in group is `<name>__*` in a thread's set.
  let tools = McpTools::new(&hub, &["web__*".into()], dir.path().join("downloads"));
  tools.prepare().await;
  assert!(tools.has("web__echo"));
  let thread = RecordId::random(jiff::Timestamp::now(), &"test".parse::<NodeName>().unwrap());
  let o = tools.call(&thread, "web__echo", &json!({})).await;
  assert!(o.ok, "{o:?}");
  assert_eq!(o.output, "auth=Bearer tok custom=yes session=s1");
  drop(tools);
  hub.stop_all().await;
}

fn enabled_by_project() {
  let dir = tempfile::tempdir().unwrap();
  let project = dir.path().join("proj");
  std::fs::create_dir_all(project.join("sub")).unwrap();
  let text = format!(
    "enable = [\"fetch\"]\n[projects.\"{}\"]\nenable = [\"github__get_*\"]\n",
    project.display()
  );
  let s: McpSettings = toml::from_str(&text).unwrap();
  assert_eq!(s.enabled(None), ["fetch"]);
  assert_eq!(
    s.enabled(Some(&project.join("sub"))),
    ["fetch", "github__get_*"]
  );
  assert_eq!(s.enabled(Some(dir.path())), ["fetch"]);
  // These lists name servers: one named like a built-in group is the
  // server.
  let s: McpSettings = toml::from_str("enable = [\"memory\", \"-web\"]").unwrap();
  assert_eq!(s.enabled(None), ["memory__*", "-web__*"]);
}
