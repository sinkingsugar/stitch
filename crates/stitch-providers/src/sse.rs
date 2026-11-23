//! Server-sent events: `event:` and `data:` lines, blank line ends an event.
//! Comments (`:`) and `id:`/`retry:` are ignored; several `data:` lines join
//! with newlines. Bytes are fed as they arrive, split anywhere.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SseEvent {
  pub event: Option<String>,
  pub data: String,
}

#[derive(Default)]
pub struct SseParser {
  buf: Vec<u8>,
  event: Option<String>,
  data: Option<String>,
}

impl SseParser {
  /// Feeds bytes; returns the events they complete.
  pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
    self.buf.extend_from_slice(bytes);
    let mut out = Vec::new();
    while let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
      let mut line: Vec<u8> = self.buf.drain(..=nl).collect();
      line.pop();
      if line.last() == Some(&b'\r') {
        line.pop();
      }
      let line = String::from_utf8_lossy(&line);
      if line.is_empty() {
        if let Some(data) = self.data.take() {
          out.push(SseEvent {
            event: self.event.take(),
            data,
          });
        }
        self.event = None;
        continue;
      }
      if line.starts_with(':') {
        continue;
      }
      let (field, value) = match line.split_once(':') {
        Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
        None => (line.as_ref(), ""),
      };
      match field {
        "event" => self.event = Some(value.to_owned()),
        "data" => match &mut self.data {
          Some(d) => {
            d.push('\n');
            d.push_str(value);
          }
          None => self.data = Some(value.to_owned()),
        },
        _ => {}
      }
    }
    out
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn events_split_anywhere_are_reassembled() {
    let text =
      "event: a\ndata: {\"x\":1}\n\n: comment\ndata: one\r\ndata: two\r\n\r\nevent: b\ndata:\n\n";
    for split in 0..text.len() {
      let mut p = SseParser::default();
      let mut events = p.feed(&text.as_bytes()[..split]);
      events.extend(p.feed(&text.as_bytes()[split..]));
      assert_eq!(
        events,
        vec![
          SseEvent {
            event: Some("a".into()),
            data: "{\"x\":1}".into()
          },
          SseEvent {
            event: None,
            data: "one\ntwo".into()
          },
          SseEvent {
            event: Some("b".into()),
            data: String::new()
          },
        ],
        "split at {split}"
      );
    }
  }
}
