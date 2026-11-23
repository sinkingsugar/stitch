//! The user's calendar, reminders and places around them, from the
//! device (design §2.3, "Calendar, reminders and places"): where the app
//! lends the node its EventKit and MapKit (iOS, visionOS), the node offers
//! `calendar_events`, `calendar_add_event`, `calendar_delete_event`,
//! `reminders`, `reminder_add`, `reminder_complete`, `reminder_delete`
//! (the `calendar` group) and `find_nearby` (the `maps` group).
//!
//! **The host is kept dumb, the node keeps the tools:** schemas, checking
//! and normalizing input (dates), what asks the user, and the text the
//! model reads are here; the app only runs one operation (`Host::call`,
//! JSON in and out) against the system's stores, asking the system's
//! permission the first time. Nodes with no host (desktops, Linux) offer
//! none of these tools.

use std::sync::{Arc, RwLock};

use serde_json::{Map, Value, json};
use stitch_providers::ToolDef;
use stitch_records::RecordId;

use crate::tools::{Asker, ToolOutcome};

/// The `calendar` group's tools.
pub const CALENDAR_TOOLS: &[&str] = &[
  "calendar_events",
  "calendar_add_event",
  "calendar_delete_event",
  "reminders",
  "reminder_add",
  "reminder_complete",
  "reminder_delete",
];

/// The `maps` group's tools.
pub const MAPS_TOOLS: &[&str] = &["find_nearby"];

/// The device's personal data, lent by the app: one operation at a time,
/// blocking (the app may ask the system's permission, and waits for it).
///
/// Operations and their input (times are `When` objects, below):
/// - `calendars` `{type: "event"|"reminder"}` → `{calendars: [Calendar]}`
///   (a `Calendar`, a reminders list too: `{id, name, source, writable,
///   default}`, `id` EventKit's `calendarIdentifier`)
/// - `events` `{from?, to?, calendar_ids?, query?, limit}` →
///   `{calendars: [Calendar], from, to, events: [Event], total}`
/// - `get_event` `{id}` → `Event` (with `calendar` its name, `calendar_id`)
/// - `add_event` `{title, start, end?, all_day, location?, notes?,
///   calendar_id?, alarms: [minutes before], url?}` → `Event`
/// - `delete_event` `{id, span: "this"|"future"}` → `Event`
/// - `reminders` `{status: "incomplete"|"overdue"|"completed", from?, to?,
///   calendar_ids?, query?, limit}` → `{lists: [Calendar], reminders:
///   [Reminder], total}` (a `Reminder`'s `list`, `list_id`)
/// - `get_reminder` `{id}` → `Reminder`
/// - `add_reminder` `{title, due?, notes?, calendar_id?, priority, alarm}` →
///   `Reminder`
/// - `complete_reminder` `{id, completed}` → `Reminder`
/// - `delete_reminder` `{id}` → `Reminder`
/// - `find_nearby` `{query, category?, near?, radius_m, limit}` →
///   `{center: {name, latitude, longitude}, places: [Place]}`: every place
///   its searches found, kept to the radius as far as Apple Maps lets it
///   (`category` an `MKPointOfInterestCategory` without its prefix); the
///   node keeps those within it, nearest first, and cuts to `limit`.
///
/// Calendars and lists are named to the device by their ids: the node
/// resolves the names the model gives (`calendars`, first).
///
/// A `When` is `{"date": "YYYY-MM-DD"}` (a day, in the device's time
/// zone), `{"local": "YYYY-MM-DDTHH:MM:SS"}` (a wall-clock time there) or
/// `{"epoch": seconds}` (an instant). An `Event`'s `start` and `end` come
/// back as RFC 3339 with the device's offset, or as dates when all-day
/// (`end` the last day, inclusive); a `Reminder`'s `due` either way.
pub trait Host: Send + Sync + 'static {
  fn call(&self, op: &str, input: &Value) -> Result<Value, String>;
}

static HOST: RwLock<Option<Arc<dyn Host>>> = RwLock::new(None);

/// Lends the node the device's calendar, reminders and maps (the app
/// does, before it opens the node).
pub fn set_host(host: Arc<dyn Host>) {
  *HOST.write().unwrap_or_else(|e| e.into_inner()) = Some(host);
}

/// The device's, when lent.
pub(crate) fn host() -> Option<Arc<dyn Host>> {
  HOST.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// What a call does to the user's data, for the asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
  /// Reads: runs.
  Read,
  /// Adds or changes something easily undone (an event or reminder
  /// added, a reminder ticked): runs, but not for a read-only thread.
  Write,
  /// Deletes: asks the user (yolo allows it), refused when read-only.
  Delete,
}

/// A call checked and turned into the host's operation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Request {
  pub op: &'static str,
  pub input: Value,
  pub kind: Kind,
}

/// The tools' definitions, for the groups on, when the device lent its
/// data.
pub(crate) fn definitions(calendar: bool, maps: bool) -> Vec<ToolDef> {
  if host().is_none() {
    return Vec::new();
  }
  all_definitions()
    .into_iter()
    .filter(|d| {
      (calendar && CALENDAR_TOOLS.contains(&d.name.as_str()))
        || (maps && MAPS_TOOLS.contains(&d.name.as_str()))
    })
    .collect()
}

const WHEN: &str = "YYYY-MM-DD for a day, YYYY-MM-DDTHH:MM for a time where the user is (the device's time zone), or with an offset (…+01:00, …Z) for an exact instant.";

pub(crate) fn all_definitions() -> Vec<ToolDef> {
  let when = |what: &str| json!({"type": "string", "description": format!("{what}: {WHEN}")});
  vec![
    ToolDef {
      name: "calendar_events".into(),
      description: "The events of the user's calendars (the device's Calendar: iCloud, Google, Exchange and others it syncs) in a date range, with the calendars' names. Each event has an id for calendar_delete_event.".into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "from": when("Start of the range (default: today)"),
          "to": when("End of the range; a day counts whole (default: 7 days after from)"),
          "calendars": {"type": "array", "items": {"type": "string"}, "description": "Only these calendars: by name, or by the id listed where names repeat."},
          "query": {"type": "string", "description": "Only events whose title, location or notes contain this (any case)."},
          "limit": {"type": "integer", "minimum": 1, "maximum": 500, "description": "At most this many, soonest first (default 100)."}
        }
      }),
    },
    ToolDef {
      name: "calendar_add_event".into(),
      description: "Add an event to the user's calendar.".into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "title": {"type": "string"},
          "start": when("When it starts; a day alone makes it all-day"),
          "end": when("When it ends (default: an hour after a timed start; for all-day, the last day, default the start's)"),
          "all_day": {"type": "boolean", "description": "An all-day event (default: when start is a day)."},
          "location": {"type": "string"},
          "notes": {"type": "string"},
          "url": {"type": "string"},
          "calendar": {"type": "string", "description": "The calendar: its name, or the id calendar_events lists where two share a name (default: the user's default calendar)."},
          "alarms": {"type": "array", "items": {"type": "integer", "minimum": 0}, "description": "Alerts, in minutes before the start (0: at the start)."}
        },
        "required": ["title", "start"]
      }),
    },
    ToolDef {
      name: "calendar_delete_event".into(),
      description: "Delete an event from the user's calendar, by the id calendar_events gave. Asks the user.".into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "id": {"type": "string"},
          "span": {"type": "string", "enum": ["this", "future"], "description": "For a repeating event: this occurrence (default) or it and all after it."}
        },
        "required": ["id"]
      }),
    },
    ToolDef {
      name: "reminders".into(),
      description: "The user's reminders (the device's Reminders app), with the lists' names: incomplete (default), overdue, or completed; optionally those due (or, completed, completed) in a range. Each has an id for reminder_complete and reminder_delete.".into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "status": {"type": "string", "enum": ["incomplete", "overdue", "completed"], "description": "Default incomplete."},
          "from": when("Due (completed, for completed ones) from"),
          "to": when("Due (completed) until; a day counts whole"),
          "lists": {"type": "array", "items": {"type": "string"}, "description": "Only these lists: by name, or by the id listed where names repeat."},
          "query": {"type": "string", "description": "Only reminders whose title or notes contain this (any case)."},
          "limit": {"type": "integer", "minimum": 1, "maximum": 500, "description": "At most this many (default 100): by due date, undated last; completed ones latest first."}
        }
      }),
    },
    ToolDef {
      name: "reminder_add".into(),
      description: "Add a reminder to the user's Reminders.".into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "title": {"type": "string"},
          "due": when("When it is due (a time alerts then; a day alone has no alert)"),
          "notes": {"type": "string"},
          "list": {"type": "string", "description": "The list: its name, or the id reminders lists where two share a name (default: the user's default list)."},
          "priority": {"type": "string", "enum": ["none", "low", "medium", "high"]}
        },
        "required": ["title"]
      }),
    },
    ToolDef {
      name: "reminder_complete".into(),
      description: "Mark a reminder done, or not done again, by the id reminders gave.".into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "id": {"type": "string"},
          "completed": {"type": "boolean", "description": "false marks it not done (default true)."}
        },
        "required": ["id"]
      }),
    },
    ToolDef {
      name: "reminder_delete".into(),
      description: "Delete a reminder, by the id reminders gave. Asks the user.".into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {"id": {"type": "string"}},
        "required": ["id"]
      }),
    },
    ToolDef {
      name: "find_nearby".into(),
      description: "Search Apple Maps for places (shops, restaurants, services, addresses) around the user's current location, or around a place named in near; only places within radius_m, nearest first, with address, phone, website and distance. When nothing is within it, it searches wider in steps (at most three more searches, up to 50 km) until something is found, and says those places are beyond the requested radius and at what radius they were found.".into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "query": {"type": "string", "description": "What to look for, in natural language (\"pharmacy\", \"sushi\", \"Via Roma 3\")."},
          "near": {"type": "string", "description": "Search around this place (a city, an address, a landmark) instead of the user's location."},
          "radius_m": {"type": "integer", "minimum": 100, "maximum": 50000, "description": "Search radius in metres (default 4000)."},
          "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": "At most this many (default 15)."}
        },
        "required": ["query"]
      }),
    },
  ]
}

/// A time as the model wrote it, as the host takes it (`Host`).
pub(crate) fn when(s: &str) -> Result<Value, String> {
  let s = s.trim();
  let bad = || format!("`{s}` is not a date or time: {WHEN}");
  // A day.
  if s.len() == 10 {
    let d: jiff::civil::Date = s.parse().map_err(|_| bad())?;
    return Ok(json!({"date": d.to_string()}));
  }
  // An instant (an offset or Z).
  if let Ok(t) = s.parse::<jiff::Timestamp>() {
    return Ok(json!({"epoch": t.as_second()}));
  }
  // A time where the user is (no offset).
  let has_offset = s
    .get(10..)
    .is_some_and(|t| t.contains(['+', 'Z', 'z']) || t[1..].contains('-') || t.contains('['));
  if has_offset {
    return Err(bad());
  }
  let t: jiff::civil::DateTime = s.replacen(' ', "T", 1).parse().map_err(|_| bad())?;
  Ok(json!({"local": t.strftime("%Y-%m-%dT%H:%M:%S").to_string()}))
}

fn text<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
  input
    .get(key)
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|s| !s.is_empty())
}

fn required<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
  text(input, key).ok_or_else(|| format!("`{key}` is required"))
}

fn names(input: &Value, key: &str) -> Result<Option<Vec<String>>, String> {
  match input.get(key) {
    None | Some(Value::Null) => Ok(None),
    Some(Value::Array(a)) => a
      .iter()
      .map(|v| {
        v.as_str()
          .map(str::to_owned)
          .ok_or_else(|| format!("`{key}` is a list of names"))
      })
      .collect::<Result<Vec<_>, _>>()
      .map(|v| Some(v).filter(|v| !v.is_empty())),
    Some(Value::String(s)) => Ok(Some(vec![s.clone()])),
    Some(_) => Err(format!("`{key}` is a list of names")),
  }
}

fn int(input: &Value, key: &str, default: i64, min: i64, max: i64) -> Result<i64, String> {
  match input.get(key) {
    None | Some(Value::Null) => Ok(default),
    Some(v) => match v.as_i64() {
      Some(n) if (min..=max).contains(&n) => Ok(n),
      _ => Err(format!("`{key}` is a whole number from {min} to {max}")),
    },
  }
}

fn flag(input: &Value, key: &str) -> Result<Option<bool>, String> {
  match input.get(key) {
    None | Some(Value::Null) => Ok(None),
    Some(Value::Bool(b)) => Ok(Some(*b)),
    Some(_) => Err(format!("`{key}` is true or false")),
  }
}

/// Copies the optional texts `keys` of `input` into `out`.
fn texts(out: &mut Map<String, Value>, input: &Value, keys: &[&str]) {
  for k in keys {
    if let Some(v) = text(input, k) {
      out.insert((*k).into(), v.into());
    }
  }
}

fn times(out: &mut Map<String, Value>, input: &Value, keys: &[&str]) -> Result<(), String> {
  for k in keys {
    if let Some(v) = text(input, k) {
      out.insert((*k).into(), when(v).map_err(|e| format!("`{k}`: {e}"))?);
    }
  }
  Ok(())
}

/// Checks a call and turns it into the host's operation.
pub(crate) fn request(name: &str, input: &Value) -> Result<Request, String> {
  let mut out = Map::new();
  let (op, kind) = match name {
    "calendar_events" => {
      times(&mut out, input, &["from", "to"])?;
      if let Some(c) = names(input, "calendars")? {
        out.insert("calendars".into(), json!(c));
      }
      texts(&mut out, input, &["query"]);
      out.insert("limit".into(), int(input, "limit", 100, 1, 500)?.into());
      ("events", Kind::Read)
    }
    "calendar_add_event" => {
      out.insert("title".into(), required(input, "title")?.into());
      let start = when(required(input, "start")?).map_err(|e| format!("`start`: {e}"))?;
      let day = start.get("date").is_some();
      let all_day = flag(input, "all_day")?.unwrap_or(day);
      if day && !all_day {
        return Err("`start` is a day alone: give a time for an event that is not all-day".into());
      }
      out.insert("start".into(), start);
      times(&mut out, input, &["end"])?;
      out.insert("all_day".into(), all_day.into());
      texts(&mut out, input, &["location", "notes", "calendar", "url"]);
      let alarms = match input.get("alarms") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
          .iter()
          .map(|v| match v.as_i64() {
            Some(n) if (0..=40_320).contains(&n) => Ok(n),
            _ => Err("`alarms` are minutes before the start, from 0 to 40320 (4 weeks)".to_owned()),
          })
          .collect::<Result<_, _>>()?,
        Some(_) => return Err("`alarms` is a list of minutes before the start".into()),
      };
      out.insert("alarms".into(), json!(alarms));
      ("add_event", Kind::Write)
    }
    "calendar_delete_event" => {
      out.insert("id".into(), required(input, "id")?.into());
      let span = text(input, "span").unwrap_or("this");
      if !matches!(span, "this" | "future") {
        return Err("`span` is this or future".into());
      }
      out.insert("span".into(), span.into());
      ("delete_event", Kind::Delete)
    }
    "reminders" => {
      let status = text(input, "status").unwrap_or("incomplete");
      if !matches!(status, "incomplete" | "overdue" | "completed") {
        return Err("`status` is incomplete, overdue or completed".into());
      }
      out.insert("status".into(), status.into());
      times(&mut out, input, &["from", "to"])?;
      if let Some(l) = names(input, "lists")? {
        out.insert("lists".into(), json!(l));
      }
      texts(&mut out, input, &["query"]);
      out.insert("limit".into(), int(input, "limit", 100, 1, 500)?.into());
      ("reminders", Kind::Read)
    }
    "reminder_add" => {
      out.insert("title".into(), required(input, "title")?.into());
      times(&mut out, input, &["due"])?;
      texts(&mut out, input, &["notes", "list"]);
      // EventKit's: 1 high, 5 medium, 9 low, 0 none.
      let priority = match text(input, "priority").unwrap_or("none") {
        "none" => 0,
        "high" => 1,
        "medium" => 5,
        "low" => 9,
        _ => return Err("`priority` is none, low, medium or high".into()),
      };
      out.insert("priority".into(), priority.into());
      // A due time alerts, as the Reminders app's own do.
      let timed = out.get("due").is_some_and(|d| d.get("date").is_none());
      out.insert("alarm".into(), timed.into());
      ("add_reminder", Kind::Write)
    }
    "reminder_complete" => {
      out.insert("id".into(), required(input, "id")?.into());
      out.insert(
        "completed".into(),
        flag(input, "completed")?.unwrap_or(true).into(),
      );
      ("complete_reminder", Kind::Write)
    }
    "reminder_delete" => {
      out.insert("id".into(), required(input, "id")?.into());
      ("delete_reminder", Kind::Delete)
    }
    "find_nearby" => {
      let query = required(input, "query")?;
      out.insert("query".into(), query.into());
      if let Some(c) = category(query) {
        out.insert("category".into(), c.into());
      }
      texts(&mut out, input, &["near"]);
      out.insert(
        "radius_m".into(),
        int(input, "radius_m", 4000, 100, 50_000)?.into(),
      );
      out.insert("limit".into(), int(input, "limit", 15, 1, 50)?.into());
      ("find_nearby", Kind::Read)
    }
    _ => return Err(format!("no tool {name}")),
  };
  Ok(Request {
    op,
    input: Value::Object(out),
    kind,
  })
}

/// Runs a call against the device's data: checked, asked when it deletes,
/// run on a blocking thread (the app answers on its own).
pub(crate) async fn call(
  host: Arc<dyn Host>,
  asker: &Asker,
  thread: &RecordId,
  name: &str,
  input: &Value,
) -> ToolOutcome {
  let mut req = match request(name, input) {
    Ok(r) => r,
    Err(e) => return ToolOutcome::err(e),
  };
  if req.kind != Kind::Read && asker.read_only {
    return ToolOutcome::err(format!(
      "this thread is read-only: {name} changes the user's {} and is refused",
      if name.starts_with("calendar") {
        "calendar"
      } else {
        "reminders"
      }
    ));
  }
  if req.kind == Kind::Delete && !asker.yolo {
    // The question names what goes: the item read first.
    let get = if req.op == "delete_event" {
      "get_event"
    } else {
      "get_reminder"
    };
    let id = json!({"id": req.input["id"]});
    let item = match run(&host, get, id).await {
      Ok(v) => labelled(&host, v).await,
      Err(e) => return ToolOutcome::err(e),
    };
    let line = if req.op == "delete_event" {
      let mut l = format!("delete the event {}", event_line(&item));
      if req.input["span"] == "future" && item["recurring"] == true {
        l.push_str(", and every later occurrence");
      }
      l
    } else {
      format!("delete the reminder {}", reminder_line(&item))
    };
    let what = if req.op == "delete_event" {
      "calendar"
    } else {
      "reminders"
    };
    if let Err(e) = asker.ask_always(thread, what, &line).await {
      return ToolOutcome::err(e);
    }
  }
  if let Err(e) = by_id(&host, &mut req).await {
    return ToolOutcome::err(e);
  }
  if req.op == "find_nearby" {
    return match nearby(&host, req.input).await {
      Ok(text) => ToolOutcome::ok(text),
      Err(e) => ToolOutcome::err(e),
    };
  }
  match run(&host, req.op, req.input).await {
    Ok(v) => ToolOutcome::ok(render(req.op, &labelled(&host, v).await)),
    Err(e) => ToolOutcome::err(e),
  }
}

/// One event or reminder (what an add, a tick or a delete answers), its
/// calendar (list) named as the listings name it: with its short id when
/// another shares its name. The calendars are read for it; when they
/// cannot be, the item stays as it came.
async fn labelled(host: &Arc<dyn Host>, item: Value) -> Value {
  let (name, id, kind) = if !s(&item, "calendar_id").is_empty() {
    ("calendar", "calendar_id", "event")
  } else if !s(&item, "list_id").is_empty() {
    ("list", "list_id", "reminder")
  } else {
    return item;
  };
  match run(host, "calendars", json!({"type": kind})).await {
    Ok(v) => told_apart(
      &item,
      name,
      id,
      v["calendars"].as_array().map(Vec::as_slice).unwrap_or(&[]),
    ),
    Err(_) => item,
  }
}

async fn run(host: &Arc<dyn Host>, op: &'static str, input: Value) -> Result<Value, String> {
  let host = host.clone();
  tokio::task::spawn_blocking(move || host.call(op, &input))
    .await
    .map_err(|e| e.to_string())?
}

// Calendars and lists, by name or id.

/// Turns the calendars or lists a call names (`calendars`, `calendar`,
/// `lists`, `list`) into the device's identifiers (`calendar_ids`,
/// `calendar_id`), reading them from the device first: a name two of them
/// share is refused with both, so the agent names one by its id.
async fn by_id(host: &Arc<dyn Host>, req: &mut Request) -> Result<(), String> {
  let (kind, key, many, word) = match req.op {
    "events" => ("event", "calendars", true, "calendar"),
    "add_event" => ("event", "calendar", false, "calendar"),
    "reminders" => ("reminder", "lists", true, "list"),
    "add_reminder" => ("reminder", "list", false, "list"),
    _ => return Ok(()),
  };
  let Some(given) = req.input.as_object_mut().and_then(|o| o.remove(key)) else {
    return Ok(());
  };
  let listed = run(host, "calendars", json!({"type": kind})).await?;
  let all = listed["calendars"].as_array().cloned().unwrap_or_default();
  if many {
    let ids = given
      .as_array()
      .into_iter()
      .flatten()
      .filter_map(Value::as_str)
      .map(|n| resolve(&all, n, word))
      .collect::<Result<Vec<_>, _>>()?;
    req.input["calendar_ids"] = json!(ids);
  } else {
    let id = resolve(&all, given.as_str().unwrap_or(""), word)?;
    let c = all.iter().find(|c| s(c, "id") == id);
    if let Some(c) = c.filter(|c| c["writable"] == false) {
      return Err(format!("the {word} {} is read-only", plain(c)));
    }
    req.input["calendar_id"] = id.into();
  }
  Ok(())
}

/// Each collection's short id: the start of its identifier, as long as
/// it takes to tell it from the others (8 characters at least).
fn short_ids(all: &[Value]) -> Vec<String> {
  let ids: Vec<String> = all.iter().map(|c| s(c, "id").to_uppercase()).collect();
  ids
    .iter()
    .enumerate()
    .map(|(i, id)| {
      let chars: Vec<char> = id.chars().collect();
      let mut n = 8.min(chars.len());
      while n < chars.len() {
        let p: String = chars[..n].iter().collect();
        if !ids
          .iter()
          .enumerate()
          .any(|(j, o)| j != i && o.starts_with(&p))
        {
          break;
        }
        n += 1;
      }
      chars[..n].iter().collect()
    })
    .collect()
}

/// Whether another collection has `c`'s name (any case).
fn shared_name(all: &[Value], i: usize) -> bool {
  let name = s(&all[i], "name");
  all
    .iter()
    .enumerate()
    .any(|(j, o)| j != i && s(o, "name").eq_ignore_ascii_case(name))
}

/// A collection by its name and source, without id.
fn plain(c: &Value) -> String {
  if s(c, "source").is_empty() {
    s(c, "name").to_owned()
  } else {
    format!("{} ({})", s(c, "name"), s(c, "source"))
  }
}

/// A collection as listed: its name, then its source, whether it is the
/// default or read-only, and its short id where its name is shared.
fn collection_line(all: &[Value], shorts: &[String], i: usize) -> String {
  let c = &all[i];
  let mut notes = Vec::new();
  if !s(c, "source").is_empty() {
    notes.push(s(c, "source").to_owned());
  }
  if c["default"] == true {
    notes.push("default".into());
  }
  if c["writable"] == false {
    notes.push("read-only".into());
  }
  if shared_name(all, i) {
    notes.push(format!("id {}", shorts[i]));
  }
  if notes.is_empty() {
    s(c, "name").to_owned()
  } else {
    format!("{} ({})", s(c, "name"), notes.join(", "))
  }
}

/// The identifier of the calendar or list `given` names: its name (when
/// no other has it), its id, its short id, or its line as listed
/// (`Work (iCloud, id 1A2B3C4D)`).
pub(crate) fn resolve(all: &[Value], given: &str, word: &str) -> Result<String, String> {
  let g = given.trim();
  let shorts = short_ids(all);
  let listed = || {
    (0..all.len())
      .map(|i| collection_line(all, &shorts, i))
      .collect::<Vec<_>>()
      .join("; ")
  };
  if let Some(c) = all
    .iter()
    .find(|c| !s(c, "id").is_empty() && s(c, "id") == g)
  {
    return Ok(s(c, "id").to_owned());
  }
  let named: Vec<usize> = (0..all.len())
    .filter(|&i| s(&all[i], "name").eq_ignore_ascii_case(g))
    .collect();
  match named.as_slice() {
    [i] => return Ok(s(&all[*i], "id").to_owned()),
    [] => {}
    many => {
      return Err(format!(
        "{} {word}s are named “{g}”: {}. Name one by its id",
        many.len(),
        many
          .iter()
          .map(|&i| collection_line(all, &shorts, i))
          .collect::<Vec<_>>()
          .join("; ")
      ));
    }
  }
  // An id's start: the whole of what was given, or its last word (the
  // line as listed).
  let last = g
    .rsplit(|c: char| c.is_whitespace() || c == '(')
    .next()
    .unwrap_or("")
    .trim_matches(|c: char| matches!(c, ')' | '#' | ',' | '[' | ']'));
  for p in [g.trim_start_matches('#'), last] {
    if p.len() < 4 {
      continue;
    }
    let p = p.to_uppercase();
    let hits: Vec<&Value> = all
      .iter()
      .filter(|c| s(c, "id").to_uppercase().starts_with(&p))
      .collect();
    if let [c] = hits.as_slice() {
      return Ok(s(c, "id").to_owned());
    }
  }
  Err(format!("no {word} “{g}”; there are: {}", listed()))
}

// Places.

/// What a `find_nearby` query names among Apple Maps' categories
/// (`MKPointOfInterestCategory`'s, without the `MKPOICategory` prefix),
/// for a search kept to the places of that kind (English and Italian).
pub(crate) fn category(query: &str) -> Option<&'static str> {
  const KINDS: &[(&str, &[&str])] = &[
    (
      "Pharmacy",
      &[
        "pharmacy",
        "pharmacies",
        "chemist",
        "chemists",
        "drugstore",
        "drugstores",
        "farmacia",
        "farmacie",
      ],
    ),
    (
      "Hospital",
      &[
        "hospital",
        "hospitals",
        "ospedale",
        "ospedali",
        "pronto soccorso",
        "emergency room",
      ],
    ),
    ("ATM", &["atm", "atms", "cash machine", "bancomat"]),
    ("Bank", &["bank", "banks", "banca", "banche"]),
    (
      "Bakery",
      &["bakery", "bakeries", "panetteria", "panificio", "forno"],
    ),
    (
      "Cafe",
      &[
        "cafe",
        "cafes",
        "café",
        "cafés",
        "coffee",
        "coffee shop",
        "caffè",
        "caffe",
      ],
    ),
    (
      "Restaurant",
      &["restaurant", "restaurants", "ristorante", "ristoranti"],
    ),
    (
      "FoodMarket",
      &[
        "supermarket",
        "supermarkets",
        "grocery",
        "groceries",
        "grocery store",
        "supermercato",
        "supermercati",
      ],
    ),
    (
      "GasStation",
      &[
        "gas station",
        "gas stations",
        "petrol station",
        "petrol",
        "fuel",
        "benzinaio",
        "distributore",
        "distributore di benzina",
      ],
    ),
    (
      "EVCharger",
      &[
        "ev charger",
        "ev chargers",
        "charging station",
        "colonnina",
        "colonnine",
      ],
    ),
    (
      "Parking",
      &["parking", "car park", "parcheggio", "parcheggi"],
    ),
    (
      "PostOffice",
      &["post office", "post offices", "posta", "ufficio postale"],
    ),
    (
      "Police",
      &[
        "police",
        "police station",
        "polizia",
        "carabinieri",
        "questura",
      ],
    ),
    ("Hotel", &["hotel", "hotels", "albergo", "alberghi"]),
    (
      "FitnessCenter",
      &["gym", "gyms", "fitness", "palestra", "palestre"],
    ),
    (
      "Laundry",
      &["laundry", "laundromat", "lavanderia", "lavanderie"],
    ),
    ("Library", &["library", "libraries", "biblioteca"]),
    ("Museum", &["museum", "museums", "museo", "musei"]),
    ("Park", &["park", "parks", "parco", "parchi"]),
    ("School", &["school", "schools", "scuola", "scuole"]),
    (
      "PublicTransport",
      &[
        "train station",
        "metro",
        "subway",
        "stazione",
        "metropolitana",
      ],
    ),
  ];
  let q = query.trim().to_lowercase();
  KINDS
    .iter()
    .find(|(_, words)| words.contains(&q.as_str()))
    .map(|(c, _)| *c)
}

/// The widest `find_nearby` searches.
const NEARBY_MAX: f64 = 50_000.0;
/// At most this many searches after the first, when nothing is within.
const NEARBY_WIDENINGS: usize = 3;

/// The radii searched in turn when nothing was found within the one
/// asked: each at least three times the last, the last 50 km, at most
/// three of them (300 m: 1.7 km, 9.1 km, 50 km; 4 km: 12, 36, 50 km),
/// rounded to read well.
pub(crate) fn wider(radius: f64) -> Vec<f64> {
  if radius >= NEARBY_MAX {
    return Vec::new();
  }
  let step = (NEARBY_MAX / radius)
    .powf(1.0 / NEARBY_WIDENINGS as f64)
    .max(3.0);
  let mut out = Vec::new();
  let mut r = radius;
  while out.len() < NEARBY_WIDENINGS {
    r *= step;
    if r >= NEARBY_MAX * 0.99 {
      out.push(NEARBY_MAX);
      break;
    }
    out.push(if r < 1000.0 {
      (r / 10.0).round() * 10.0
    } else {
      (r / 100.0).round() * 100.0
    });
  }
  out
}

/// Searches around the place, keeps what lies within the radius; when
/// nothing does, searches wider in steps (`wider`) until something is
/// within the step's radius, and says those places are beyond the radius
/// asked and at what radius they were found.
async fn nearby(host: &Arc<dyn Host>, input: Value) -> Result<String, String> {
  let query = s(&input, "query").to_owned();
  let radius = input["radius_m"].as_f64().unwrap_or(4000.0);
  let limit = input["limit"].as_u64().unwrap_or(15) as usize;
  let first = run(host, "find_nearby", input.clone()).await?;
  let mut rounds: Vec<(f64, Value)> = Vec::new();
  if places_within(&first, radius).is_empty() {
    for wide in wider(radius) {
      let mut again = input.clone();
      again["radius_m"] = (wide as i64).into();
      let found = run(host, "find_nearby", again).await?;
      let done = !places_within(&found, wide).is_empty();
      rounds.push((wide, found));
      if done {
        break;
      }
    }
  }
  let rounds: Vec<(f64, &Value)> = rounds.iter().map(|(r, v)| (*r, v)).collect();
  Ok(nearby_answer(&query, radius, limit, &first, &rounds))
}

/// Metres between two points (haversine).
fn metres(a: (f64, f64), b: (f64, f64)) -> f64 {
  let (la1, lo1, la2, lo2) = (
    a.0.to_radians(),
    a.1.to_radians(),
    b.0.to_radians(),
    b.1.to_radians(),
  );
  let h =
    ((la2 - la1) / 2.0).sin().powi(2) + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
  2.0 * 6_371_000.0 * h.sqrt().asin()
}

/// A place's distance from the centre: the device's, else measured.
fn distance(v: &Value, p: &Value) -> Option<f64> {
  p["distance_m"].as_f64().or_else(|| {
    let c = &v["center"];
    Some(metres(
      (c["latitude"].as_f64()?, c["longitude"].as_f64()?),
      (p["latitude"].as_f64()?, p["longitude"].as_f64()?),
    ))
  })
}

/// The places of an answer, nearest first, each once (the device runs
/// two searches), with their distances.
fn places_sorted(v: &Value) -> Vec<(f64, Value)> {
  let mut all: Vec<(f64, Value)> = v["places"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|p| Some((distance(v, p)?, p.clone())))
    .collect();
  all.sort_by(|a, b| a.0.total_cmp(&b.0));
  let mut out: Vec<(f64, Value)> = Vec::new();
  for (d, p) in all {
    let same = out
      .iter()
      .any(|(e, q)| s(q, "name").eq_ignore_ascii_case(s(&p, "name")) && (e - d).abs() < 50.0);
    if !same {
      out.push((d, p));
    }
  }
  out
}

fn places_within(v: &Value, radius: f64) -> Vec<(f64, Value)> {
  places_sorted(v)
    .into_iter()
    .filter(|(d, _)| *d <= radius)
    .collect()
}

fn far(d: f64) -> String {
  if d < 1000.0 {
    format!("{d:.0} m")
  } else if d % 1000.0 == 0.0 {
    format!("{:.0} km", d / 1000.0)
  } else {
    format!("{:.1} km", d / 1000.0)
  }
}

fn place_line(d: f64, p: &Value) -> String {
  let mut l = format!("\n- {} · {}", s(p, "name"), far(d));
  for k in ["category", "address", "phone", "url"] {
    if !s(p, k).is_empty() {
      l.push_str(&format!(" · {}", s(p, k)));
    }
  }
  if let (Some(la), Some(lo)) = (p["latitude"].as_f64(), p["longitude"].as_f64()) {
    l.push_str(&format!(" · {la:.5},{lo:.5}"));
  }
  l
}

/// What the model reads of `find_nearby`: the places within `radius`,
/// nearest first, at most `limit`; when there are none, those of the
/// wider searches (`wider`: each one's radius and answer, in turn, the
/// last the one that found something, if any), said to be beyond it.
pub(crate) fn nearby_answer(
  query: &str,
  radius: f64,
  limit: usize,
  v: &Value,
  wider: &[(f64, &Value)],
) -> String {
  let c = &v["center"];
  let mut out = format!(
    "Around {} ({:.5}, {:.5}): ",
    match s(c, "name") {
      "" => "the user's location",
      n => n,
    },
    c["latitude"].as_f64().unwrap_or(0.0),
    c["longitude"].as_f64().unwrap_or(0.0),
  );
  let all = places_sorted(v);
  let within: Vec<&(f64, Value)> = all.iter().filter(|(d, _)| *d <= radius).collect();
  if !within.is_empty() {
    let shown = within.len().min(limit);
    out.push_str(&format!(
      "{shown} place{} for “{query}” within {}, nearest first",
      if shown == 1 { "" } else { "s" },
      far(radius)
    ));
    if within.len() > shown {
      out.push_str(&format!(
        " ({shown} shown of {}: raise limit)",
        within.len()
      ));
    }
    for (d, p) in within.iter().take(limit) {
      out.push_str(&place_line(*d, p));
    }
    let beyond = all.len() - within.len();
    if beyond > 0 {
      out.push_str(&format!(
        "\n({beyond} more found beyond {}, not shown: raise radius_m to see them)",
        far(radius)
      ));
    }
    return out;
  }
  // "1.7 km, 9.1 km or 50 km"
  let either = |radii: &[f64]| -> String {
    let all: Vec<String> = radii.iter().map(|r| far(*r)).collect();
    match all.split_last() {
      Some((last, [])) => last.clone(),
      Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
      None => String::new(),
    }
  };
  let radii: Vec<f64> = wider.iter().map(|(r, _)| *r).collect();
  let Some(&(wide, w)) = wider.last() else {
    out.push_str(&format!("nothing for “{query}” within {}", far(radius)));
    return out;
  };
  let found = places_within(w, wide);
  if found.is_empty() {
    out.push_str(&format!(
      "nothing for “{query}” within {}, nor within {}",
      far(radius),
      either(&radii)
    ));
    return out;
  }
  let shown = found.len().min(limit);
  let before = &radii[..radii.len() - 1];
  out.push_str(&format!(
    "nothing for “{query}” within the requested {}{}.\nWidened to {}: {} BEYOND the requested {} radius, nearest first",
    far(radius),
    if before.is_empty() {
      String::new()
    } else {
      format!(", nor within {}", either(before))
    },
    far(wide),
    if shown == 1 {
      "this place is".to_owned()
    } else {
      format!("these {shown} places are")
    },
    far(radius),
  ));
  if found.len() > shown {
    out.push_str(&format!(" ({shown} shown of {}: raise limit)", found.len()));
  }
  for (d, p) in found.iter().take(limit) {
    out.push_str(&place_line(*d, p));
  }
  out
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
  v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// `2026-11-12T15:00:00+01:00` as `2026-11-12 15:00 (+01:00)`, cut to what
/// reads: the date and the time, the offset once.
fn short(t: &str) -> String {
  if t.len() < 16 {
    return t.to_owned();
  }
  let offset = t.get(19..).unwrap_or("");
  let offset = if offset.is_empty() {
    String::new()
  } else {
    format!(" ({offset})")
  };
  format!("{} {}{offset}", &t[..10], &t[11..16])
}

fn notes(v: &Value) -> String {
  let n = s(v, "notes").trim();
  if n.is_empty() {
    return String::new();
  }
  let one: String = n.split_whitespace().collect::<Vec<_>>().join(" ");
  let cut: String = one.chars().take(300).collect();
  let more = if cut.len() < one.len() { "…" } else { "" };
  format!("\n  notes: {cut}{more}")
}

/// An event in a line: when, what, where, which calendar, its id.
fn event_line(e: &Value) -> String {
  let (start, end) = (s(e, "start"), s(e, "end"));
  let when = if e["all_day"] == true {
    if end.is_empty() || end == start {
      format!("{start} (all day)")
    } else {
      format!("{start} to {end} (all day)")
    }
  } else if end.len() >= 16 && start.len() >= 16 && end[..10] == start[..10] {
    let s = short(start);
    match s.split_once(" (") {
      Some((a, off)) => format!("{a}–{} ({off}", &end[11..16]),
      None => format!("{s}–{}", &end[11..16]),
    }
  } else if end.is_empty() {
    short(start)
  } else {
    format!("{} – {}", short(start), short(end))
  };
  let mut l = format!("“{}” {when}", s(e, "title"));
  if !s(e, "location").is_empty() {
    l.push_str(&format!(" at {}", s(e, "location")));
  }
  if !s(e, "calendar").is_empty() {
    l.push_str(&format!(" [{}]", s(e, "calendar")));
  }
  if e["recurring"] == true {
    l.push_str(" (repeats)");
  }
  l
}

fn priority(p: i64) -> &'static str {
  match p {
    1..=4 => " · high priority",
    5 => " · medium priority",
    6..=9 => " · low priority",
    _ => "",
  }
}

fn reminder_line(r: &Value) -> String {
  let mut l = format!("“{}”", s(r, "title"));
  if !s(r, "due").is_empty() {
    l.push_str(&format!(" due {}", short(s(r, "due"))));
    if r["overdue"] == true {
      l.push_str(" (overdue)");
    }
  }
  if r["completed"] == true {
    l.push_str(" · done");
    if !s(r, "completed_at").is_empty() {
      l.push_str(&format!(" {}", short(s(r, "completed_at"))));
    }
  }
  if !s(r, "list").is_empty() {
    l.push_str(&format!(" [{}]", s(r, "list")));
  }
  l.push_str(priority(r["priority"].as_i64().unwrap_or(0)));
  l
}

fn with_id(line: String, v: &Value) -> String {
  format!("{line} · id: {}", s(v, "id"))
}

fn collections(all: &[Value], word: &str) -> String {
  if all.is_empty() {
    return format!("{word}: none");
  }
  let shorts = short_ids(all);
  let lines: Vec<String> = (0..all.len())
    .map(|i| collection_line(all, &shorts, i))
    .collect();
  format!("{word}: {}", lines.join("; "))
}

/// An item whose calendar (list) shares its name with another, its name
/// followed by the calendar's short id, as the calendars are listed.
fn told_apart(item: &Value, name: &str, id: &str, all: &[Value]) -> Value {
  let shorts = short_ids(all);
  let Some(i) = all
    .iter()
    .position(|c| !s(item, id).is_empty() && s(c, "id") == s(item, id))
  else {
    return item.clone();
  };
  if !shared_name(all, i) {
    return item.clone();
  }
  let mut out = item.clone();
  out[name] = format!("{}, id {}", s(item, name), shorts[i]).into();
  out
}

fn counted(v: &Value, shown: usize) -> String {
  match v["total"].as_u64() {
    Some(t) if t as usize > shown => {
      format!(" ({shown} shown of {t}: narrow the range or raise limit)")
    }
    _ => String::new(),
  }
}

/// What the model reads of an operation's answer.
pub(crate) fn render(op: &str, v: &Value) -> String {
  match op {
    "events" => {
      let all = v["calendars"].as_array().cloned().unwrap_or_default();
      let events: Vec<Value> = v["events"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| told_apart(e, "calendar", "calendar_id", &all))
        .collect();
      let mut out = collections(&all, "Calendars");
      out.push_str(&format!(
        "\nEvents from {} to {}: {}{}",
        short(s(v, "from")),
        short(s(v, "to")),
        events.len(),
        counted(v, events.len())
      ));
      for e in &events {
        out.push_str(&format!("\n- {}{}", with_id(event_line(e), e), notes(e)));
      }
      out
    }
    "reminders" => {
      let all = v["lists"].as_array().cloned().unwrap_or_default();
      let items: Vec<Value> = v["reminders"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| told_apart(r, "list", "list_id", &all))
        .collect();
      let mut out = collections(&all, "Lists");
      out.push_str(&format!(
        "\n{} reminders: {}{}",
        match s(v, "status") {
          "overdue" => "Overdue",
          "completed" => "Completed",
          _ => "Incomplete",
        },
        items.len(),
        counted(v, items.len())
      ));
      for r in &items {
        out.push_str(&format!("\n- {}{}", with_id(reminder_line(r), r), notes(r)));
      }
      out
    }
    "add_event" => format!("Added the event {}", with_id(event_line(v), v)),
    "delete_event" => format!("Deleted the event {}", event_line(v)),
    "get_event" => with_id(event_line(v), v),
    "add_reminder" => format!("Added the reminder {}", with_id(reminder_line(v), v)),
    "complete_reminder" => {
      let done = if v["completed"] == true {
        "Done"
      } else {
        "Not done again"
      };
      format!("{done}: {}", with_id(reminder_line(v), v))
    }
    "delete_reminder" => format!("Deleted the reminder {}", reminder_line(v)),
    "get_reminder" => with_id(reminder_line(v), v),
    _ => v.to_string(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn times_as_the_host_takes_them() {
    assert_eq!(when("2026-11-12").unwrap(), json!({"date": "2026-11-12"}));
    assert_eq!(
      when("2026-11-12T15:00").unwrap(),
      json!({"local": "2026-11-12T15:00:00"})
    );
    assert_eq!(
      when("2026-11-12 15:30:10").unwrap(),
      json!({"local": "2026-11-12T15:30:10"})
    );
    assert_eq!(
      when("2026-11-12T15:00:00Z").unwrap(),
      json!({"epoch": 1_794_495_600})
    );
    assert_eq!(
      when("2026-11-12T16:00:00+01:00").unwrap(),
      json!({"epoch": 1_794_495_600})
    );
    for bad in [
      "tomorrow",
      "2026-13-01",
      "12/11/2026",
      "2026-11-12T25:00",
      "",
    ] {
      assert!(when(bad).is_err(), "{bad}");
    }
  }

  #[test]
  fn every_tool_is_in_a_group_and_defined() {
    let defs = all_definitions();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    let mut grouped: Vec<&str> = CALENDAR_TOOLS.iter().chain(MAPS_TOOLS).copied().collect();
    grouped.sort();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(sorted, grouped);
    assert_eq!(crate::toolset::group_of("reminder_add"), Some("calendar"));
    assert_eq!(crate::toolset::group_of("find_nearby"), Some("maps"));
    for d in &defs {
      assert_eq!(d.input_schema["type"], "object", "{}", d.name);
      assert_eq!(d.input_schema["additionalProperties"], false, "{}", d.name);
      // Every required key is a property, every call checks.
      for r in d.input_schema["required"].as_array().into_iter().flatten() {
        assert!(
          d.input_schema["properties"][r.as_str().unwrap()].is_object(),
          "{}: {r}",
          d.name
        );
      }
      assert!(request(&d.name, &json!({})).is_ok() == d.input_schema.get("required").is_none());
    }
  }

  #[test]
  fn no_host_no_tools() {
    // No test lends one: a node with none (a desktop, Linux) offers none.
    assert!(host().is_none());
    assert!(definitions(true, true).is_empty());
  }

  #[test]
  fn calls_checked_and_normalized() {
    let r = request(
      "calendar_add_event",
      &json!({"title": " Dentist ", "start": "2026-11-12T15:00", "alarms": [10, 0], "calendar": "Home"}),
    )
    .unwrap();
    assert_eq!(r.op, "add_event");
    assert_eq!(r.kind, Kind::Write);
    assert_eq!(
      r.input,
      json!({"title": "Dentist", "start": {"local": "2026-11-12T15:00:00"}, "all_day": false,
             "calendar": "Home", "alarms": [10, 0]})
    );
    let r = request(
      "calendar_add_event",
      &json!({"title": "Trip", "start": "2026-12-01", "end": "2026-12-03"}),
    )
    .unwrap();
    assert_eq!(r.input["all_day"], true);
    assert_eq!(r.input["end"], json!({"date": "2026-12-03"}));
    assert!(
      request(
        "calendar_add_event",
        &json!({"title": "x", "start": "2026-12-01", "all_day": false})
      )
      .is_err()
    );
    assert!(
      request(
        "calendar_add_event",
        &json!({"title": "x", "start": "soon"})
      )
      .unwrap_err()
      .contains("`start`")
    );
    assert!(
      request(
        "calendar_add_event",
        &json!({"title": "x", "start": "2026-12-01", "alarms": [-5]})
      )
      .is_err()
    );

    let r = request("calendar_delete_event", &json!({"id": "E1"})).unwrap();
    assert_eq!((r.op, r.kind), ("delete_event", Kind::Delete));
    assert_eq!(r.input, json!({"id": "E1", "span": "this"}));
    assert!(request("calendar_delete_event", &json!({"id": "E1", "span": "all"})).is_err());

    let r = request(
      "reminders",
      &json!({"status": "overdue", "lists": "Shopping"}),
    )
    .unwrap();
    assert_eq!(
      r.input,
      json!({"status": "overdue", "lists": ["Shopping"], "limit": 100})
    );
    assert!(request("reminders", &json!({"status": "late"})).is_err());

    let r = request(
      "reminder_add",
      &json!({"title": "Milk", "due": "2026-11-10T18:00", "priority": "high"}),
    )
    .unwrap();
    assert_eq!(r.input["priority"], 1);
    assert_eq!(r.input["alarm"], true);
    let r = request(
      "reminder_add",
      &json!({"title": "Milk", "due": "2026-11-10"}),
    )
    .unwrap();
    assert_eq!(r.input["alarm"], false);

    let r = request("reminder_complete", &json!({"id": "R1"})).unwrap();
    assert_eq!(
      (r.input["completed"].clone(), r.kind),
      (json!(true), Kind::Write)
    );
    assert_eq!(
      request("reminder_delete", &json!({"id": "R1"}))
        .unwrap()
        .kind,
      Kind::Delete
    );

    let r = request("find_nearby", &json!({"query": "pharmacy"})).unwrap();
    assert_eq!(
      r.input,
      json!({"query": "pharmacy", "category": "Pharmacy", "radius_m": 4000, "limit": 15})
    );
    assert!(request("find_nearby", &json!({"query": "x", "radius_m": 1})).is_err());
  }

  /// A device that records the operations asked of it.
  #[derive(Default)]
  struct Device(std::sync::Mutex<Vec<String>>);

  impl Host for Device {
    fn call(&self, op: &str, input: &Value) -> Result<Value, String> {
      self.0.lock().unwrap().push(op.to_owned());
      match op {
        "get_event" | "delete_event" => Ok(json!({"id": input["id"], "title": "Dentist",
          "start": "2026-11-12T15:00:00+01:00", "end": "2026-11-12T16:00:00+01:00", "recurring": true})),
        "add_reminder" => Ok(json!({"id": "R9", "title": input["title"]})),
        _ => Err(format!("Stitch has no access to the calendar ({op})")),
      }
    }
  }

  fn asker(
    read_only: bool,
    yolo: bool,
    answer: Option<bool>,
  ) -> (Asker, Arc<std::sync::Mutex<Vec<String>>>) {
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = asked.clone();
    let approver: Option<crate::tools::Approver> = answer.map(|allow| {
      let f: crate::tools::Approver = Arc::new(move |r: crate::tools::ApprovalRequest| {
        log
          .lock()
          .unwrap()
          .push(format!("{}: {}", r.session, r.command));
        Box::pin(async move {
          if allow {
            crate::tools::Decision::allow()
          } else {
            crate::tools::Decision::default()
          }
        }) as crate::tools::BoxFuture<'static, _>
      });
      f
    });
    (
      Asker {
        approver,
        read_only,
        yolo,
      },
      asked,
    )
  }

  #[tokio::test]
  async fn deletes_ask_and_read_only_threads_only_read() {
    let thread: RecordId = "2026-11-10T152637Z-mbp-4ppq".parse().unwrap();
    let del = json!({"id": "E1", "span": "future"});

    // Asked, with what goes; declined: nothing deleted.
    let device = Arc::new(Device::default());
    let (a, asked) = asker(false, false, Some(false));
    let out = call(device.clone(), &a, &thread, "calendar_delete_event", &del).await;
    assert!(!out.ok && out.output.contains("declined"), "{}", out.output);
    assert_eq!(*device.0.lock().unwrap(), ["get_event"]);
    assert_eq!(
      *asked.lock().unwrap(),
      [
        "calendar: delete the event “Dentist” 2026-11-12 15:00–16:00 (+01:00) (repeats), and every later occurrence"
      ]
    );

    // Allowed: deleted.
    let device = Arc::new(Device::default());
    let (a, _) = asker(false, false, Some(true));
    let out = call(device.clone(), &a, &thread, "calendar_delete_event", &del).await;
    assert!(
      out.ok && out.output.starts_with("Deleted the event “Dentist”"),
      "{}",
      out.output
    );
    assert_eq!(*device.0.lock().unwrap(), ["get_event", "delete_event"]);

    // Nobody to ask: refused. Yolo: not asked.
    let (a, _) = asker(false, false, None);
    assert!(
      !call(
        Arc::new(Device::default()),
        &a,
        &thread,
        "reminder_delete",
        &json!({"id": "R1"})
      )
      .await
      .ok
    );
    let device = Arc::new(Device::default());
    let (a, asked) = asker(false, true, Some(false));
    assert!(
      call(device.clone(), &a, &thread, "calendar_delete_event", &del)
        .await
        .ok
    );
    assert!(asked.lock().unwrap().is_empty());

    // Adds run without asking; a read-only thread is refused them.
    let (a, asked) = asker(false, false, Some(false));
    let out = call(
      Arc::new(Device::default()),
      &a,
      &thread,
      "reminder_add",
      &json!({"title": "Milk"}),
    )
    .await;
    assert_eq!(out.output, "Added the reminder “Milk” · id: R9");
    assert!(asked.lock().unwrap().is_empty());
    let device = Arc::new(Device::default());
    let (a, _) = asker(true, false, Some(true));
    for (name, input) in [
      ("reminder_add", json!({"title": "Milk"})),
      ("reminder_complete", json!({"id": "R1"})),
      ("calendar_delete_event", del.clone()),
    ] {
      let out = call(device.clone(), &a, &thread, name, &input).await;
      assert!(
        !out.ok && out.output.contains("read-only"),
        "{name}: {}",
        out.output
      );
    }
    assert!(device.0.lock().unwrap().is_empty());

    // The device's refusal (a permission denied) is the tool's error.
    let out = call(
      Arc::new(Device::default()),
      &a,
      &thread,
      "calendar_events",
      &json!({}),
    )
    .await;
    assert_eq!(
      (out.ok, out.output.as_str()),
      (false, "Stitch has no access to the calendar (events)")
    );
  }

  #[test]
  fn answers_read_well() {
    let v = json!({
      "calendars": [{"name": "Home", "source": "iCloud", "default": true, "writable": true},
                    {"name": "Holidays", "source": "Other", "writable": false}],
      "from": "2026-11-10T00:00:00+01:00", "to": "2026-11-17T00:00:00+01:00", "total": 3,
      "events": [
        {"id": "E1", "title": "Dentist", "start": "2026-11-12T15:00:00+01:00", "end": "2026-11-12T16:00:00+01:00",
         "all_day": false, "location": "Via Roma 1", "calendar": "Home", "notes": "bring\nthe card"},
        {"id": "E2#1765000000", "title": "Trip", "start": "2026-12-01", "end": "2026-12-03", "all_day": true,
         "calendar": "Home", "recurring": true}
      ]
    });
    assert_eq!(
      render("events", &v),
      "Calendars: Home (iCloud, default); Holidays (Other, read-only)\n\
       Events from 2026-11-10 00:00 (+01:00) to 2026-11-17 00:00 (+01:00): 2 (2 shown of 3: narrow the range or raise limit)\n\
       - “Dentist” 2026-11-12 15:00–16:00 (+01:00) at Via Roma 1 [Home] · id: E1\n  notes: bring the card\n\
       - “Trip” 2026-12-01 to 2026-12-03 (all day) [Home] (repeats) · id: E2#1765000000"
    );
    let r = json!({"status": "overdue", "lists": [{"name": "Shopping"}], "reminders": [
      {"id": "R1", "title": "Milk", "due": "2026-11-09T18:00:00+01:00", "overdue": true, "list": "Shopping", "priority": 1}
    ]});
    assert_eq!(
      render("reminders", &r),
      "Lists: Shopping\nOverdue reminders: 1\n- “Milk” due 2026-11-09 18:00 (+01:00) (overdue) [Shopping] · high priority · id: R1"
    );
    let p = json!({"center": {"latitude": 45.75, "longitude": 4.85}, "places": [
      {"name": "Pharmacie", "distance_m": 230.4, "address": "Rue X 1", "phone": "+33 4", "latitude": 45.751, "longitude": 4.851},
      {"name": "Far", "distance_m": 2500.0}
    ]});
    assert_eq!(
      nearby_answer("pharmacy", 4000.0, 15, &p, &[]),
      "Around the user's location (45.75000, 4.85000): 2 places for “pharmacy” within 4 km, nearest first\n\
       - Pharmacie · 230 m · Rue X 1 · +33 4 · 45.75100,4.85100\n- Far · 2.5 km"
    );
  }

  /// A suburb, where Apple Maps answered “pharmacy” with the city centre's.
  fn suburb(places: Value) -> Value {
    json!({"center": {"latitude": 45.76000, "longitude": 4.84000}, "places": places})
  }

  #[test]
  fn nearby_keeps_to_the_radius_nearest_first() {
    let v = suburb(json!([
      {"name": "Pharmacie Centrale", "distance_m": 6600.0},
      {"name": "Pharmacie du Parc", "distance_m": 2100.0},
      {"name": "Boots", "distance_m": 7100.0},
      {"name": "Pharmacie du Quartier", "distance_m": 800.0},
      // The same place from both of the device's searches.
      {"name": "Pharmacie du Quartier", "distance_m": 800.4},
      // No distance: measured from the centre.
      {"name": "Pharmacie de la Gare", "latitude": 45.7410, "longitude": 4.81235},
    ]));
    assert_eq!(
      nearby_answer("pharmacy", 4000.0, 15, &v, &[]),
      "Around the user's location (45.76000, 4.84000): 3 places for “pharmacy” within 4 km, nearest first\n\
       - Pharmacie du Quartier · 800 m\n- Pharmacie du Parc · 2.1 km\n- Pharmacie de la Gare · 3.0 km · 45.74100,4.81235\n\
       (2 more found beyond 4 km, not shown: raise radius_m to see them)"
    );
    // The limit cuts the nearest.
    assert_eq!(
      nearby_answer("pharmacy", 4000.0, 1, &v, &[]),
      "Around the user's location (45.76000, 4.84000): 1 place for “pharmacy” within 4 km, nearest first (1 shown of 3: raise limit)\n\
       - Pharmacie du Quartier · 800 m\n\
       (2 more found beyond 4 km, not shown: raise radius_m to see them)"
    );
  }

  #[test]
  fn nearby_widened_says_beyond() {
    let far = suburb(json!([
      {"name": "Boots", "distance_m": 7100.0},
      {"name": "Pharmacie Centrale", "distance_m": 6600.0},
    ]));
    let wider = suburb(json!([
      {"name": "Boots", "distance_m": 7100.0},
      {"name": "Pharmacie Centrale", "distance_m": 6600.0},
      {"name": "Pharmacie Lointaine", "distance_m": 13000.0},
    ]));
    assert_eq!(
      nearby_answer("pharmacy", 4000.0, 15, &far, &[(12000.0, &wider)]),
      "Around the user's location (45.76000, 4.84000): nothing for “pharmacy” within the requested 4 km.\n\
       Widened to 12 km: these 2 places are BEYOND the requested 4 km radius, nearest first\n\
       - Pharmacie Centrale · 6.6 km\n- Boots · 7.1 km"
    );
    let none = suburb(json!([]));
    assert_eq!(
      nearby_answer("pharmacy", 4000.0, 15, &none, &[(12000.0, &none)]),
      "Around the user's location (45.76000, 4.84000): nothing for “pharmacy” within 4 km, nor within 12 km"
    );
    assert_eq!(
      nearby_answer("pharmacy", 50000.0, 15, &none, &[]),
      "Around the user's location (45.76000, 4.84000): nothing for “pharmacy” within 50 km"
    );
  }

  #[test]
  fn nearby_names_apple_maps_categories() {
    assert_eq!(category("pharmacy"), Some("Pharmacy"));
    assert_eq!(category(" Farmacia "), Some("Pharmacy"));
    assert_eq!(category("pharmacies"), Some("Pharmacy"));
    assert_eq!(category("gas station"), Some("GasStation"));
    assert_eq!(category("sushi"), None);
    assert_eq!(category("Via Roma 3"), None);
    let r = request(
      "find_nearby",
      &json!({"query": "Pharmacy", "near": "Villeurbanne"}),
    )
    .unwrap();
    assert_eq!(r.input["category"], "Pharmacy");
    let r = request("find_nearby", &json!({"query": "sushi"})).unwrap();
    assert!(r.input.get("category").is_none());
  }

  /// A phone with two calendars of one name, and Apple Maps answering by
  /// the radius asked.
  #[derive(Default)]
  struct Phone(std::sync::Mutex<Vec<Value>>);

  impl Host for Phone {
    fn call(&self, op: &str, input: &Value) -> Result<Value, String> {
      self
        .0
        .lock()
        .unwrap()
        .push(json!({"op": op, "input": input}));
      match op {
        "calendars" => Ok(json!({"calendars": [
          {"id": "1A2B3C4D-0000-4000-8000-000000000001", "name": "Work", "source": "iCloud", "writable": true},
          {"id": "5E6F7A8B-0000-4000-8000-000000000002", "name": "Work", "source": "iCloud", "writable": true},
          {"id": "9C9C9C9C-0000-4000-8000-000000000003", "name": "Home", "source": "iCloud", "writable": true, "default": true},
          {"id": "77777777-0000-4000-8000-000000000004", "name": "Holidays", "source": "Other", "writable": false},
        ]})),
        "add_event" => {
          let (name, id) = match input["calendar_id"].as_str() {
            Some(id @ "9C9C9C9C-0000-4000-8000-000000000003") => ("Home", id),
            Some(id) => ("Work", id),
            None => ("Home", "9C9C9C9C-0000-4000-8000-000000000003"),
          };
          Ok(
            json!({"id": "E7", "title": input["title"], "start": "2026-11-12T15:00:00+01:00",
            "end": "2026-11-12T16:00:00+01:00", "calendar": name, "calendar_id": id}),
          )
        }
        "get_event" | "delete_event" => Ok(
          json!({"id": input["id"], "title": "Review", "start": "2026-11-12T15:00:00+01:00",
          "end": "2026-11-12T16:00:00+01:00", "calendar": "Work",
          "calendar_id": "5E6F7A8B-0000-4000-8000-000000000002"}),
        ),
        "add_reminder" | "get_reminder" | "complete_reminder" | "delete_reminder" => Ok(
          json!({"id": "R3", "title": "Invoice", "completed": op == "complete_reminder",
          "list": "Work", "list_id": "1A2B3C4D-0000-4000-8000-000000000001"}),
        ),
        "events" => Ok(json!({"calendars": [], "events": []})),
        "find_nearby" => {
          if input["radius_m"] == 4000 {
            Ok(suburb(
              json!([{"name": "Pharmacie Centrale", "distance_m": 6600.0}]),
            ))
          } else {
            Ok(suburb(json!([
              {"name": "Pharmacie Centrale", "distance_m": 6600.0},
              {"name": "Boots", "distance_m": 7100.0}
            ])))
          }
        }
        _ => Err(format!("no {op}")),
      }
    }
  }

  #[tokio::test]
  async fn nearby_asks_wider_only_when_nothing_is_within() {
    let thread: RecordId = "2026-11-10T152637Z-mbp-4ppq".parse().unwrap();
    let (a, _) = asker(false, false, None);
    let phone = Arc::new(Phone::default());
    let out = call(
      phone.clone(),
      &a,
      &thread,
      "find_nearby",
      &json!({"query": "pharmacy"}),
    )
    .await;
    assert!(out.ok, "{}", out.output);
    assert!(
      out
        .output
        .contains("Widened to 12 km: these 2 places are BEYOND the requested 4 km radius"),
      "{}",
      out.output
    );
    let asked: Vec<Value> = phone
      .0
      .lock()
      .unwrap()
      .iter()
      .map(|c| c["input"]["radius_m"].clone())
      .collect();
    assert_eq!(asked, [json!(4000), json!(12000)]);
    assert_eq!(phone.0.lock().unwrap()[0]["input"]["category"], "Pharmacy");
  }

  #[test]
  fn calendars_with_one_name_are_told_apart() {
    let all = json!([
      {"id": "1A2B3C4D-0000-4000-8000-000000000001", "name": "Work", "source": "iCloud"},
      {"id": "1A2B3C4E-0000-4000-8000-000000000002", "name": "Work", "source": "iCloud"},
      {"id": "9C9C9C9C-0000-4000-8000-000000000003", "name": "Home", "source": "iCloud"},
    ]);
    let all = all.as_array().unwrap();
    let ids = |r: Result<String, String>| r.map(|id| id[..8].to_owned());
    assert_eq!(ids(resolve(all, "home", "calendar")), Ok("9C9C9C9C".into()));
    // The short id, the whole id, the line as listed.
    assert_eq!(
      ids(resolve(all, "1a2b3c4e", "calendar")),
      Ok("1A2B3C4E".into())
    );
    assert_eq!(
      ids(resolve(
        all,
        "1A2B3C4D-0000-4000-8000-000000000001",
        "calendar"
      )),
      Ok("1A2B3C4D".into())
    );
    assert_eq!(
      ids(resolve(all, "Work (iCloud, id 1A2B3C4E)", "calendar")),
      Ok("1A2B3C4E".into())
    );
    assert_eq!(
      resolve(all, "Work", "calendar"),
      Err(
        "2 calendars are named “Work”: Work (iCloud, id 1A2B3C4D); Work (iCloud, id 1A2B3C4E). Name one by its id"
          .into()
      )
    );
    assert_eq!(
      resolve(all, "Gym", "calendar"),
      Err(
        "no calendar “Gym”; there are: Work (iCloud, id 1A2B3C4D); Work (iCloud, id 1A2B3C4E); Home (iCloud)"
          .into()
      )
    );
    // A prefix shared by both is no id.
    assert!(resolve(all, "1A2B", "calendar").is_err());

    // Listed with their ids, where names repeat; events say which.
    let v = json!({"calendars": all, "from": "2026-11-10", "to": "2026-11-11", "events": [
      {"id": "E1", "title": "Standup", "start": "2026-11-10", "all_day": true, "calendar": "Work",
       "calendar_id": "1A2B3C4E-0000-4000-8000-000000000002"},
      {"id": "E2", "title": "Dinner", "start": "2026-11-10", "all_day": true, "calendar": "Home",
       "calendar_id": "9C9C9C9C-0000-4000-8000-000000000003"}
    ]});
    assert_eq!(
      render("events", &v),
      "Calendars: Work (iCloud, id 1A2B3C4D); Work (iCloud, id 1A2B3C4E); Home (iCloud)\n\
       Events from 2026-11-10 to 2026-11-11: 2\n\
       - “Standup” 2026-11-10 (all day) [Work, id 1A2B3C4E] · id: E1\n\
       - “Dinner” 2026-11-10 (all day) [Home] · id: E2"
    );
    let r = json!({"lists": [
      {"id": "AAAA1111-x", "name": "Shopping"}, {"id": "BBBB2222-y", "name": "Shopping", "default": true}
    ], "reminders": [{"id": "R1", "title": "Milk", "list": "Shopping", "list_id": "BBBB2222-y"}]});
    assert_eq!(
      render("reminders", &r),
      "Lists: Shopping (id AAAA1111); Shopping (default, id BBBB2222)\nIncomplete reminders: 1\n\
       - “Milk” [Shopping, id BBBB2222] · id: R1"
    );
  }

  #[tokio::test]
  async fn an_ambiguous_calendar_is_refused_with_the_candidates() {
    let thread: RecordId = "2026-11-10T152637Z-mbp-4ppq".parse().unwrap();
    let (a, _) = asker(false, false, None);
    let add = |calendar: &str| json!({"title": "Review", "start": "2026-11-12T15:00", "calendar": calendar});

    let phone = Arc::new(Phone::default());
    let out = call(
      phone.clone(),
      &a,
      &thread,
      "calendar_add_event",
      &add("Work"),
    )
    .await;
    assert!(!out.ok, "{}", out.output);
    assert!(
      out.output.contains("2 calendars are named “Work”")
        && out.output.contains("id 1A2B3C4D")
        && out.output.contains("id 5E6F7A8B"),
      "{}",
      out.output
    );
    let ops: Vec<Value> = phone
      .0
      .lock()
      .unwrap()
      .iter()
      .map(|c| c["op"].clone())
      .collect();
    assert_eq!(ops, [json!("calendars")], "nothing added");
    assert_eq!(phone.0.lock().unwrap()[0]["input"]["type"], "event");

    // Its id picks it; the device gets the calendar's whole id.
    let phone = Arc::new(Phone::default());
    let out = call(
      phone.clone(),
      &a,
      &thread,
      "calendar_add_event",
      &add("5E6F7A8B"),
    )
    .await;
    assert!(out.ok, "{}", out.output);
    assert_eq!(
      out.output,
      "Added the event “Review” 2026-11-12 15:00–16:00 (+01:00) [Work, id 5E6F7A8B] · id: E7"
    );
    let calls = phone.0.lock().unwrap().clone();
    assert_eq!(calls[1]["op"], "add_event");
    assert_eq!(
      calls[1]["input"]["calendar_id"],
      "5E6F7A8B-0000-4000-8000-000000000002"
    );
    assert!(calls[1]["input"].get("calendar").is_none());

    // Read-only refused before the device is asked to add; filters resolve too.
    let phone = Arc::new(Phone::default());
    let out = call(
      phone.clone(),
      &a,
      &thread,
      "calendar_add_event",
      &add("Holidays"),
    )
    .await;
    assert_eq!(
      (out.ok, out.output.as_str()),
      (false, "the calendar Holidays (Other) is read-only")
    );
    let phone = Arc::new(Phone::default());
    let out = call(
      phone.clone(),
      &a,
      &thread,
      "calendar_events",
      &json!({"calendars": ["Home", "1A2B3C4D"]}),
    )
    .await;
    assert!(out.ok, "{}", out.output);
    assert_eq!(
      phone.0.lock().unwrap()[1]["input"]["calendar_ids"],
      json!([
        "9C9C9C9C-0000-4000-8000-000000000003",
        "1A2B3C4D-0000-4000-8000-000000000001"
      ])
    );
  }

  /// Confirmations and the delete question name a calendar (list) whose
  /// name is shared as the listings do: with its short id.
  #[tokio::test]
  async fn changes_name_shared_calendars_as_listed() {
    let thread: RecordId = "2026-11-10T152637Z-mbp-4ppq".parse().unwrap();
    let (a, asked) = asker(false, false, Some(true));
    let phone = Arc::new(Phone::default());
    let run = |name: &'static str, input: Value| {
      let (phone, a, thread) = (phone.clone(), &a, &thread);
      async move { call(phone, a, thread, name, &input).await.output }
    };
    // A name no other has stays plain.
    assert_eq!(
      run(
        "calendar_add_event",
        json!({"title": "Lunch", "start": "2026-11-12T15:00", "calendar": "Home"})
      )
      .await,
      "Added the event “Lunch” 2026-11-12 15:00–16:00 (+01:00) [Home] · id: E7"
    );
    assert_eq!(
      run("calendar_delete_event", json!({"id": "E7"})).await,
      "Deleted the event “Review” 2026-11-12 15:00–16:00 (+01:00) [Work, id 5E6F7A8B]"
    );
    assert_eq!(
      *asked.lock().unwrap(),
      ["calendar: delete the event “Review” 2026-11-12 15:00–16:00 (+01:00) [Work, id 5E6F7A8B]"]
    );
    assert_eq!(
      run(
        "reminder_add",
        json!({"title": "Invoice", "list": "1A2B3C4D"})
      )
      .await,
      "Added the reminder “Invoice” [Work, id 1A2B3C4D] · id: R3"
    );
    assert_eq!(
      run("reminder_complete", json!({"id": "R3"})).await,
      "Done: “Invoice” · done [Work, id 1A2B3C4D] · id: R3"
    );
    assert_eq!(
      run("reminder_delete", json!({"id": "R3"})).await,
      "Deleted the reminder “Invoice” [Work, id 1A2B3C4D]"
    );
    assert_eq!(
      asked.lock().unwrap()[1],
      "reminders: delete the reminder “Invoice” [Work, id 1A2B3C4D]"
    );
    // The lists are read as reminders' calendars.
    let types: Vec<Value> = phone
      .0
      .lock()
      .unwrap()
      .iter()
      .filter(|c| c["op"] == "calendars")
      .map(|c| c["input"]["type"].clone())
      .collect();
    assert!(types.ends_with(&[json!("reminder")]), "{types:?}");
  }

  /// Apple Maps keeping to the region asked: the places within the
  /// radius of each search.
  struct Maps(Vec<f64>, std::sync::Mutex<Vec<i64>>);

  impl Host for Maps {
    fn call(&self, op: &str, input: &Value) -> Result<Value, String> {
      assert_eq!(op, "find_nearby");
      let r = input["radius_m"].as_i64().unwrap();
      self.1.lock().unwrap().push(r);
      let places: Vec<Value> = self
        .0
        .iter()
        .filter(|d| **d <= r as f64)
        .map(|d| json!({"name": "Apple Store", "distance_m": d}))
        .collect();
      Ok(suburb(json!(places)))
    }
  }

  #[tokio::test]
  async fn nearby_widens_in_steps_up_to_50_km() {
    let thread: RecordId = "2026-11-10T152637Z-mbp-4ppq".parse().unwrap();
    let (a, _) = asker(false, false, None);
    let find = |at: Vec<f64>, radius: i64| {
      let a = &a;
      let thread = &thread;
      async move {
        let maps = Arc::new(Maps(at, Default::default()));
        let out = call(
          maps.clone(),
          a,
          thread,
          "find_nearby",
          &json!({"query": "Apple Store", "radius_m": radius}),
        )
        .await;
        assert!(out.ok, "{}", out.output);
        let radii = maps.1.lock().unwrap().clone();
        (out.output, radii)
      }
    };
    // Rare: found only at the last step, four searches in all.
    let (out, radii) = find(vec![30_000.0], 300).await;
    assert_eq!(radii, [300, 1700, 9100, 50_000]);
    assert_eq!(
      out,
      "Around the user's location (45.76000, 4.84000): nothing for “Apple Store” within the requested 300 m, nor within 1.7 km or 9.1 km.\n\
       Widened to 50 km: this place is BEYOND the requested 300 m radius, nearest first\n\
       - Apple Store · 30 km"
    );
    // Found at the first step: no more searches.
    let (out, radii) = find(vec![900.0, 1200.0], 300).await;
    assert_eq!(radii, [300, 1700]);
    assert_eq!(
      out,
      "Around the user's location (45.76000, 4.84000): nothing for “Apple Store” within the requested 300 m.\n\
       Widened to 1.7 km: these 2 places are BEYOND the requested 300 m radius, nearest first\n\
       - Apple Store · 900 m\n- Apple Store · 1.2 km"
    );
    // Nothing anywhere: every radius tried is said.
    let (out, radii) = find(vec![], 4000).await;
    assert_eq!(radii, [4000, 12_000, 36_000, 50_000]);
    assert_eq!(
      out,
      "Around the user's location (45.76000, 4.84000): nothing for “Apple Store” within 4 km, nor within 12 km, 36 km or 50 km"
    );
    // Within the radius: one search.
    let (_, radii) = find(vec![100.0], 300).await;
    assert_eq!(radii, [300]);
    // Already wide: one step to the cap; at the cap, none.
    let (_, radii) = find(vec![], 20_000).await;
    assert_eq!(radii, [20_000, 50_000]);
    let (_, radii) = find(vec![], 50_000).await;
    assert_eq!(radii, [50_000]);
  }
}
