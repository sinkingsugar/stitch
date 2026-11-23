//! The MCP client against real servers: the reference
//! `@modelcontextprotocol/server-everything` over stdio (through `npx`,
//! which may download it), and over streamable HTTP when
//! `STITCH_MCP_LIVE_HTTP` names its endpoint
//! (`PORT=3901 npx -y @modelcontextprotocol/server-everything
//! streamableHttp`, then `http://127.0.0.1:3901/mcp`). Skipped unless
//! `STITCH_MCP_LIVE` is set: it needs the network and Node.js.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::json;
use stitch_node::mcp::{Mcp, McpTools, ServerSettings};
use stitch_records::{NodeName, RecordId};

fn stdio() -> ServerSettings {
  ServerSettings {
    command: Some("npx".into()),
    args: vec![
      "-y".into(),
      "@modelcontextprotocol/server-everything".into(),
    ],
    approval: Some("allow".into()),
    timeout_secs: Some(3),
    startup_timeout_secs: Some(120),
    ..ServerSettings::default()
  }
}

/// The pids of processes whose command line contains `needle`, but this
/// test's own and the HTTP server's (started by hand, with
/// `streamableHttp` in its command).
fn running(needle: &str) -> Vec<String> {
  let out = std::process::Command::new("ps")
    .args(["-axo", "pid=,command="])
    .output()
    .unwrap();
  String::from_utf8_lossy(&out.stdout)
    .lines()
    .filter(|l| l.contains(needle) && !l.contains("streamableHttp"))
    .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
    .collect()
}

#[tokio::test]
async fn real_servers() {
  if std::env::var_os("STITCH_MCP_LIVE").is_none() {
    eprintln!("skipped: set STITCH_MCP_LIVE=1 to run against real MCP servers");
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let mut servers = BTreeMap::new();
  servers.insert("everything".to_owned(), stdio());
  let http = std::env::var("STITCH_MCP_LIVE_HTTP").ok();
  if let Some(url) = &http {
    servers.insert(
      "web".to_owned(),
      ServerSettings {
        url: Some(url.clone()),
        approval: Some("allow".into()),
        ..ServerSettings::default()
      },
    );
  }
  let hub = Mcp::new(dir.path());
  hub.configure(&servers);
  let enable: Vec<stitch_node::toolset::Entry> =
    servers.keys().map(|k| k.as_str().into()).collect();
  let tools = McpTools::new(&hub, &enable, dir.path().join("downloads"));
  tools.prepare().await;
  let names: Vec<String> = tools.definitions().into_iter().map(|d| d.name).collect();
  eprintln!("tools: {names:?}");
  assert!(names.contains(&"everything__echo".to_owned()), "{names:?}");
  // Their names keep their dashes.
  assert!(
    names.contains(&"everything__get-sum".to_owned()),
    "{names:?}"
  );
  let thread = RecordId::random(jiff::Timestamp::now(), &"test".parse::<NodeName>().unwrap());

  for server in std::iter::once("everything").chain(http.as_ref().map(|_| "web")) {
    let call = |tool: &str, args: serde_json::Value| {
      let name = format!("{server}__{tool}");
      let tools = &tools;
      let thread = &thread;
      async move {
        let o = tools.call(thread, &name, &args).await;
        eprintln!(
          "{name} {args} -> ok={} images={} {:?}",
          o.ok,
          o.images.len(),
          {
            let s: String = o.output.chars().take(300).collect();
            s
          }
        );
        o
      }
    };
    let o = call("echo", json!({"message": "hello from stitch"})).await;
    assert!(o.ok && o.output.contains("hello from stitch"), "{o:?}");
    let o = call("get-sum", json!({"a": 2, "b": 40})).await;
    assert!(o.ok && o.output.contains("42"), "{o:?}");
    let o = call("get-tiny-image", json!({})).await;
    assert!(o.ok, "{o:?}");
    assert_eq!(o.images.len(), 1, "{o:?}");
    assert!(o.images[0].path.is_file());
    let o = call("get-structured-content", json!({"location": "New York"})).await;
    assert!(o.ok, "{o:?}");
    let o = call(
      "get-resource-reference",
      json!({"resourceType": "Text", "resourceId": 1}),
    )
    .await;
    assert!(o.ok && o.output.contains("[resource "), "{o:?}");
    let o = call("get-resource-links", json!({"count": 2})).await;
    assert!(o.ok && o.output.contains("[resource link: "), "{o:?}");
    // Arguments the schema refuses: the server's error, as a failure.
    let o = call("get-sum", json!({"a": "two"})).await;
    assert!(!o.ok, "{o:?}");
    // A call past its timeout is cancelled, and the server goes on.
    if server == "everything" {
      let o = call(
        "trigger-long-running-operation",
        json!({"duration": 10, "steps": 5}),
      )
      .await;
      assert!(
        !o.ok && o.output.contains("did not answer within 3s"),
        "{o:?}"
      );
      let o = call("echo", json!({"message": "still here"})).await;
      assert!(o.ok && o.output.contains("still here"), "{o:?}");
    }
  }

  assert!(
    !running("server-everything").is_empty(),
    "the stdio server runs"
  );
  drop(tools);
  hub.stop_all().await;
  // Stopped with the node: npx and the server it started.
  for _ in 0..50 {
    if running("server-everything").is_empty() {
      break;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  assert_eq!(running("server-everything"), Vec::<String>::new());
}
