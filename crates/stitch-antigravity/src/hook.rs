//! The PreToolUse hook that keeps agy to Stitch's tools: `agy acp`
//! ignores the permission rules of its settings, so every tool call goes
//! through this gate. Tools served over MCP (`mcp_<server>_<tool>`) run
//! without asking (Stitch's own rules and approvals apply when the call
//! reaches it); agy's own tools (files, commands, the web, subagents) are
//! refused, so nothing runs outside Stitch's records.
//!
//! The hook is a hidden `stitch` subcommand (`stitch antigravity-hook`):
//! agy runs it with the call as JSON on its input, and reads the
//! decision it prints.

use serde_json::{Value, json};

/// The decision for a hook input (`{"toolCall": {"name": …}, …}`).
pub fn decision(input: &Value) -> Value {
  let name = input
    .pointer("/toolCall/name")
    .and_then(Value::as_str)
    .unwrap_or("");
  if name.starts_with("mcp_") {
    json!({"decision": "auto_approve"})
  } else {
    let name = if name.is_empty() { "this tool" } else { name };
    json!({
      "decision": "deny",
      "reason": format!("{name} is not available here; use the provided tools"),
    })
  }
}

/// Runs the hook: the call on standard input, the decision printed. An
/// input that does not parse is refused.
pub fn run() -> i32 {
  use std::io::Read as _;
  let mut text = String::new();
  let _ = std::io::stdin().read_to_string(&mut text);
  let input: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
  println!("{}", decision(&input));
  0
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_mcp_tools_pass() {
    let d = |n: &str| decision(&json!({"toolCall": {"name": n}, "conversationId": "c"}));
    assert_eq!(
      d("mcp_stitch_shell_run"),
      json!({"decision": "auto_approve"})
    );
    for native in [
      "read_file",
      "run_command",
      "search_web",
      "call_mcp_tool",
      "browser_subagent",
    ] {
      let out = d(native);
      assert_eq!(out["decision"], "deny");
      assert_eq!(
        out["reason"],
        format!("{native} is not available here; use the provided tools")
      );
    }
    assert_eq!(decision(&Value::Null)["decision"], "deny");
  }
}
