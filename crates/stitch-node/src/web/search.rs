//! `search_web`: Brave's web search (the default) or Perplexity's, with the
//! key stored as `brave` or `perplexity` (`stitch key set brave`).

use serde_json::{Value, json};

/// The providers `[search] provider` names.
pub(super) fn key_name(provider: &str) -> Result<&'static str, String> {
  match provider {
    "brave" => Ok("brave"),
    "perplexity" => Ok("perplexity"),
    other => Err(format!(
      "search provider `{other}`: use brave or perplexity"
    )),
  }
}

/// Searches; the results as text for the model.
pub(super) async fn search(
  http: &reqwest::Client,
  provider: &str,
  key: &str,
  query: &str,
) -> Result<String, String> {
  match provider {
    "perplexity" => perplexity(http, key, query).await,
    _ => brave(http, key, query).await,
  }
}

async fn json(response: Result<reqwest::Response, reqwest::Error>) -> Result<Value, String> {
  let response = response.map_err(|e| e.to_string())?;
  let status = response.status();
  let body = response.text().await.map_err(|e| e.to_string())?;
  if !status.is_success() {
    return Err(format!(
      "the search failed: HTTP {status}: {}",
      body.chars().take(500).collect::<String>()
    ));
  }
  serde_json::from_str(&body).map_err(|e| format!("the search answered unreadably: {e}"))
}

async fn brave(http: &reqwest::Client, key: &str, query: &str) -> Result<String, String> {
  let url = url::Url::parse_with_params(
    "https://api.search.brave.com/res/v1/web/search",
    &[("q", query), ("count", "10"), ("extra_snippets", "true")],
  )
  .map_err(|e| e.to_string())?;
  let v = json(
    http
      .get(url)
      .header("Accept", "application/json")
      .header("X-Subscription-Token", key)
      .send()
      .await,
  )
  .await?;
  Ok(brave_results(&v))
}

fn brave_results(v: &Value) -> String {
  let results = v["web"]["results"].as_array().cloned().unwrap_or_default();
  if results.is_empty() {
    return "No results.".into();
  }
  let mut out = Vec::new();
  for (i, r) in results.iter().enumerate() {
    let s = |k: &str| r[k].as_str().unwrap_or("").trim().to_owned();
    let mut entry = format!("{}. {}\n   {}", i + 1, strip_tags(&s("title")), s("url"));
    let mut snippets = vec![strip_tags(&s("description"))];
    snippets.extend(
      r["extra_snippets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(3)
        .map(strip_tags),
    );
    for snippet in snippets.into_iter().filter(|s| !s.is_empty()) {
      entry.push_str(&format!("\n   {snippet}"));
    }
    if let Some(age) = r["age"].as_str() {
      entry.push_str(&format!("\n   ({age})"));
    }
    out.push(entry);
  }
  out.join("\n\n")
}

/// Brave marks matches with `<strong>`.
fn strip_tags(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  let mut in_tag = false;
  for c in s.chars() {
    match c {
      '<' => in_tag = true,
      '>' if in_tag => in_tag = false,
      c if !in_tag => out.push(c),
      _ => {}
    }
  }
  out
    .replace("&amp;", "&")
    .replace("&quot;", "\"")
    .replace("&#x27;", "'")
    .replace("&lt;", "<")
    .replace("&gt;", ">")
}

async fn perplexity(http: &reqwest::Client, key: &str, query: &str) -> Result<String, String> {
  let v = json(
    http
      .post("https://api.perplexity.ai/chat/completions")
      .bearer_auth(key)
      .json(&json!({"model": "sonar", "messages": [{"role": "user", "content": query}]}))
      .send()
      .await,
  )
  .await?;
  let answer = v["choices"][0]["message"]["content"]
    .as_str()
    .unwrap_or("")
    .trim()
    .to_owned();
  let sources: Vec<String> = v["search_results"]
    .as_array()
    .map(|rs| {
      rs.iter()
        .map(|r| {
          format!(
            "{} {}",
            r["title"].as_str().unwrap_or(""),
            r["url"].as_str().unwrap_or("")
          )
        })
        .collect()
    })
    .or_else(|| {
      v["citations"].as_array().map(|cs| {
        cs.iter()
          .filter_map(Value::as_str)
          .map(str::to_owned)
          .collect()
      })
    })
    .unwrap_or_default();
  let mut out = answer;
  if !sources.is_empty() {
    out.push_str("\n\nSources:");
    for (i, s) in sources.iter().enumerate() {
      out.push_str(&format!("\n[{}] {}", i + 1, s.trim()));
    }
  }
  Ok(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn brave_results_read_as_a_list() {
    let v = json!({"web": {"results": [
      {"title": "Rust <strong>async</strong>", "url": "https://rust-lang.org", "description": "The &quot;book&quot;", "extra_snippets": ["More"], "age": "2 days ago"},
      {"title": "Second", "url": "https://example.com", "description": ""}
    ]}});
    assert_eq!(
      brave_results(&v),
      "1. Rust async\n   https://rust-lang.org\n   The \"book\"\n   More\n   (2 days ago)\n\n2. Second\n   https://example.com"
    );
    assert_eq!(brave_results(&json!({})), "No results.");
  }
}
