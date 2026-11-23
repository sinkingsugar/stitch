//! Push notifications to the user's phones (design §5, milestone 4): a
//! question asked while the app is not running reaches the phone through
//! Apple's push service (APNs).
//!
//! A phone registers for pushes and writes a `device` record in the home
//! workspace: its device token, APNs environment, and the public half of an
//! X25519 key it made (the private half stays in its keychain). A node that
//! asks the user something pushes to every phone: the content (the
//! question, the thread, the node) sealed for that phone, so only it reads
//! it; its notification service extension opens it and shows it. What APNs
//! and anyone between see is a placeholder.
//!
//! Two ways to reach APNs: with the app team's key on the node itself (a
//! build signed by the user's own team: `stitch apns set`), or, by
//! default, through the app team's relay holding the key (the Worker in
//! `relay/`; `stitch apns relay <url>` names another). Either takes the
//! same sealed content.
//!
//! Sealing: an ephemeral X25519 key, its agreement with the phone's,
//! HKDF-SHA256 (salt: both public keys; info: `stitch push v1`) to a
//! ChaCha20-Poly1305 key; the wire is the ephemeral public key, the nonce
//! and the sealed text, base64 (CryptoKit opens it on the phone).

use std::path::Path;
use std::sync::Mutex;

use base64::Engine;
use ring::rand::SecureRandom;
use serde_json::{Value, json};
use stitch_records::{Author, Draft, NodeName};
use stitch_store::{Index, RecordRoot};

use crate::config::ApnsSettings;
use crate::{Error, Result};

/// The record kind of a phone's push registration.
pub const DEVICE: &str = "device";

/// The key the APNs key is stored as.
pub const KEY: &str = "apns";

/// The app pushes are for, unless `[apns] topic` says otherwise.
pub const TOPIC: &str = "com.fragcolor.stitch";

/// The relay a node pushes through when it has no key of its own and
/// `[apns] relay` names none: the app team's (relay/, on Cloudflare).
pub const RELAY: &str = "https://stitch-relay.clamor.workers.dev";

/// What APNs, the relay and anyone between see of a push.
pub const PLACEHOLDER: (&str, &str) = ("Stitch", "Something waits on you");

/// HKDF's info for a push's key.
const INFO: &[u8] = b"stitch push v1";

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// A phone to push to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
  pub node: String,
  pub token: String,
  /// `sandbox` (a development build) or `production`.
  pub env: String,
  /// The public half of its X25519 key, base64: what pushes are sealed
  /// for.
  pub key: Option<String>,
}

/// The phones, as their latest `device` records say: one per node.
pub fn devices(index: &Index) -> Vec<Device> {
  let mut out: Vec<Device> = Vec::new();
  for r in index.records(Some(DEVICE)).unwrap_or_default() {
    let s = |k: &str| r.fields.get(k).and_then(Value::as_str).map(str::to_owned);
    let Some(token) = s("token") else { continue };
    // Oldest first: a later record of a node replaces its earlier.
    out.retain(|d| d.node != r.node);
    out.push(Device {
      node: r.node.clone(),
      token,
      env: s("env").unwrap_or_else(|| "production".into()),
      key: s("key"),
    });
  }
  out
}

/// Writes this phone's registration, unless its latest record already
/// says it; whether it wrote one.
pub fn register(
  home: &RecordRoot,
  index: &Mutex<Index>,
  node: &NodeName,
  token: &str,
  env: &str,
  key: &str,
) -> Result<bool> {
  let token = token.trim().to_ascii_lowercase();
  if token.is_empty() || !token.chars().all(|c| c.is_ascii_hexdigit()) {
    return Err(Error::Config("a device token is hex digits".into()));
  }
  if env != "sandbox" && env != "production" {
    return Err(Error::Config(
      "an APNs environment is sandbox or production".into(),
    ));
  }
  if B64.decode(key).map(|k| k.len()) != Ok(32) {
    return Err(Error::Config(
      "a push key is an X25519 public key, base64".into(),
    ));
  }
  {
    let index = index.lock().expect("index lock");
    if devices(&index).iter().any(|d| {
      d.node == node.as_str() && d.token == token && d.env == env && d.key.as_deref() == Some(key)
    }) {
      return Ok(false);
    }
  }
  let draft = Draft::new(DEVICE, Author::node(node.clone()))?
    .field("token", token.as_str())?
    .field("env", env)?
    .field("key", key)?;
  home.publish(node, draft)?;
  index.lock().expect("index lock").refresh(home)?;
  Ok(true)
}

/// Seals `content` for the phone whose X25519 public key is `key`
/// (base64): the wire form its extension opens.
pub fn seal(key: &str, content: &[u8]) -> Result<String> {
  seal_with(INFO, key, content)
}

/// The sealing of a push with another HKDF info (secrets synced between
/// nodes, `crate::secrets`, use `stitch secret v1`), so what is sealed for
/// one use never opens as the other.
pub(crate) fn seal_with(info: &[u8], key: &str, content: &[u8]) -> Result<String> {
  use ring::{aead, agreement, hkdf};
  let theirs = B64
    .decode(key)
    .ok()
    .filter(|k| k.len() == 32)
    .ok_or_else(|| Error::Config("the phone's push key is not an X25519 key".into()))?;
  let rng = ring::rand::SystemRandom::new();
  let ephemeral = agreement::EphemeralPrivateKey::generate(&agreement::X25519, &rng)
    .map_err(|_| Error::Config("no randomness for a push key".into()))?;
  let ours = ephemeral
    .compute_public_key()
    .map_err(|_| Error::Config("no push key".into()))?;
  let mut secret = [0u8; 32];
  agreement::agree_ephemeral(
    ephemeral,
    &agreement::UnparsedPublicKey::new(&agreement::X25519, &theirs),
    |shared| {
      let salt = [ours.as_ref(), theirs.as_slice()].concat();
      hkdf::Salt::new(hkdf::HKDF_SHA256, &salt)
        .extract(shared)
        .expand(&[info], hkdf::HKDF_SHA256)
        .and_then(|okm| okm.fill(&mut secret))
    },
  )
  .map_err(|_| Error::Config("the phone's push key does not agree".into()))?
  .map_err(|_| Error::Config("deriving the push key failed".into()))?;
  let sealing = aead::LessSafeKey::new(
    aead::UnboundKey::new(&aead::CHACHA20_POLY1305, &secret)
      .map_err(|_| Error::Config("no push key".into()))?,
  );
  let mut nonce = [0u8; 12];
  rng
    .fill(&mut nonce)
    .map_err(|_| Error::Config("no randomness for a nonce".into()))?;
  let mut sealed = content.to_vec();
  sealing
    .seal_in_place_append_tag(
      aead::Nonce::assume_unique_for_key(nonce),
      aead::Aad::empty(),
      &mut sealed,
    )
    .map_err(|_| Error::Config("sealing a push failed".into()))?;
  Ok(B64.encode([ours.as_ref(), &nonce, &sealed].concat()))
}

/// The X25519 public key of a private one.
pub(crate) fn public_of(private: &[u8; 32]) -> [u8; 32] {
  x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(*private)).to_bytes()
}

/// Opens what `seal_with(info, …)` sealed for the key whose private half
/// is `private`, as the phone's extension does with CryptoKit; `None` when
/// it does not open (another key, another info, tampered).
pub(crate) fn open_with(info: &[u8], private: &[u8; 32], wire: &str) -> Option<Vec<u8>> {
  use ring::{aead, hkdf};
  let wire = B64.decode(wire).ok()?;
  if wire.len() < 32 + 12 + 16 {
    return None;
  }
  let (eph, rest) = wire.split_at(32);
  let (nonce, sealed) = rest.split_at(12);
  let eph: [u8; 32] = eph.try_into().ok()?;
  let shared =
    x25519_dalek::StaticSecret::from(*private).diffie_hellman(&x25519_dalek::PublicKey::from(eph));
  let salt = [eph.as_slice(), &public_of(private)].concat();
  let mut secret = [0u8; 32];
  hkdf::Salt::new(hkdf::HKDF_SHA256, &salt)
    .extract(shared.as_bytes())
    .expand(&[info], hkdf::HKDF_SHA256)
    .ok()?
    .fill(&mut secret)
    .ok()?;
  let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::CHACHA20_POLY1305, &secret).ok()?);
  let mut buf = sealed.to_vec();
  key
    .open_in_place(
      aead::Nonce::try_assume_unique_for_key(nonce).ok()?,
      aead::Aad::empty(),
      &mut buf,
    )
    .ok()
    .map(|p| p.to_vec())
}

/// The signing key, as Apple gives it (`AuthKey_<id>.p8`, a PKCS#8 PEM).
fn signing_key(pem: &str) -> Result<ring::signature::EcdsaKeyPair> {
  let body: String = pem
    .lines()
    .filter(|l| !l.starts_with("-----"))
    .map(str::trim)
    .collect();
  let der = B64
    .decode(body)
    .map_err(|_| Error::Config("the APNs key is not a .p8 file Apple gave".into()))?;
  ring::signature::EcdsaKeyPair::from_pkcs8(
    &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
    &der,
    &ring::rand::SystemRandom::new(),
  )
  .map_err(|e| Error::Config(format!("the APNs key is not a P-256 key: {e}")))
}

/// The token APNs takes: a JWT, ES256, of the key's ID and the team's.
pub fn jwt(pem: &str, key_id: &str, team_id: &str, issued: i64) -> Result<String> {
  let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
  let header = b64(
    json!({"alg": "ES256", "kid": key_id})
      .to_string()
      .as_bytes(),
  );
  let claims = b64(
    json!({"iss": team_id, "iat": issued})
      .to_string()
      .as_bytes(),
  );
  let input = format!("{header}.{claims}");
  let sig = signing_key(pem)?
    .sign(&ring::rand::SystemRandom::new(), input.as_bytes())
    .map_err(|_| Error::Config("signing the APNs token failed".into()))?;
  Ok(format!("{input}.{}", b64(sig.as_ref())))
}

/// What a push says, sealed for each phone.
pub struct Push<'a> {
  pub title: &'a str,
  pub subtitle: Option<&'a str>,
  pub body: &'a str,
  /// The app's badge: how many questions wait.
  pub badge: Option<usize>,
  /// The question it is for, so the app opens the approvals (and its
  /// notification is the question's: `apns-collapse-id`).
  pub question: Option<&'a str>,
  /// The node that pushes: the phone syncs with it first, for the
  /// question.
  pub node: &'a str,
  /// The question was answered or withdrawn: a background push, nothing
  /// shown, that takes its notification away (design §5, milestone 4).
  pub answered: bool,
  /// The thread it is about (a notice an agent sent, `notices`): opening
  /// it opens that chat.
  pub thread: Option<&'a str>,
}

impl Push<'_> {
  /// What is sealed for the phone.
  fn content(&self) -> String {
    if self.answered {
      json!({"answered": self.question, "badge": self.badge})
    } else {
      json!({
        "title": self.title,
        "subtitle": self.subtitle,
        "body": self.body,
        "question": self.question,
        "node": self.node,
        "thread": self.thread,
      })
    }
    .to_string()
  }

  /// The payload APNs gets directly (the relay builds the same).
  fn payload(&self, sealed: &str) -> Value {
    if self.answered {
      return json!({"aps": {"content-available": 1}, "e": sealed});
    }
    let mut aps = json!({
      "alert": {"title": PLACEHOLDER.0, "body": PLACEHOLDER.1},
      "mutable-content": 1,
      "sound": "default",
      "thread-id": "questions",
    });
    if let Some(n) = self.badge {
      aps["badge"] = n.into();
    }
    json!({"aps": aps, "e": sealed})
  }

  /// What the relay takes (`/push`, or `/answered` for an answer's).
  fn relay_body(&self, d: &Device, sealed: &str) -> Value {
    let mut body = json!({"token": d.token, "env": d.env, "e": sealed});
    if self.answered {
      return body;
    }
    if let Some(n) = self.badge {
      body["badge"] = n.into();
    }
    if let Some(q) = self.question {
      body["collapse"] = q.into();
    }
    body
  }
}

/// How a node reaches APNs.
enum Route {
  /// The app team's key, here.
  Direct {
    pem: String,
    key_id: String,
    team_id: String,
    topic: String,
  },
  /// A relay that holds it.
  Relay(String),
}

/// Sends pushes for a node, keeping its APNs token an hour or so.
#[derive(Default)]
pub(crate) struct Pusher {
  /// The provider token and its `iat` (Unix seconds).
  token: Mutex<Option<(String, i64)>>,
}

impl Pusher {
  fn route(data: &Path, settings: &ApnsSettings) -> Result<Route> {
    if let (Some(key_id), Some(team_id)) = (&settings.key_id, &settings.team_id)
      && let Some(pem) = crate::keys::get(data, KEY, KEY)
    {
      return Ok(Route::Direct {
        pem,
        key_id: key_id.clone(),
        team_id: team_id.clone(),
        topic: settings.topic.clone().unwrap_or_else(|| TOPIC.into()),
      });
    }
    Ok(Route::Relay(
      settings
        .relay
        .as_deref()
        .unwrap_or(RELAY)
        .trim_end_matches('/')
        .to_owned(),
    ))
  }

  fn token(&self, pem: &str, key_id: &str, team_id: &str) -> Result<String> {
    // Aged by the wall clock, as APNs does (an hour from `iat`): a
    // monotonic `Instant` stops while a Mac sleeps, so a token made before
    // a night's sleep looked fresh and APNs said ExpiredProviderToken.
    let now = jiff::Timestamp::now().as_second();
    let mut held = self.token.lock().expect("apns token lock");
    if let Some((t, issued)) = held.as_ref()
      && (0..50 * 60).contains(&(now - issued))
    {
      return Ok(t.clone());
    }
    let t = jwt(pem, key_id, team_id, now)?;
    *held = Some((t.clone(), now));
    Ok(t)
  }

  /// Pushes to each phone; a line for each.
  pub(crate) async fn send(
    &self,
    data: &Path,
    settings: &ApnsSettings,
    devices: &[Device],
    push: &Push<'_>,
  ) -> Vec<String> {
    let route = match Self::route(data, settings) {
      Ok(r) => r,
      Err(e) => return vec![e.to_string()],
    };
    let content = push.content();
    let client = crate::github::http();
    let mut out = Vec::new();
    for d in devices {
      let Some(key) = &d.key else {
        out.push(format!(
          "{}: no push key (an older app): not pushed",
          d.node
        ));
        continue;
      };
      let sealed = match seal(key, content.as_bytes()) {
        Ok(s) => s,
        Err(e) => {
          out.push(format!("{}: {e}", d.node));
          continue;
        }
      };
      let sent = match &route {
        Route::Direct {
          pem,
          key_id,
          team_id,
          topic,
        } => {
          let token = match self.token(pem, key_id, team_id) {
            Ok(t) => t,
            Err(e) => return vec![e.to_string()],
          };
          let host = if d.env == "sandbox" {
            "api.sandbox.push.apple.com"
          } else {
            "api.push.apple.com"
          };
          let mut req = client
            .post(format!("https://{host}/3/device/{}", d.token))
            .bearer_auth(&token)
            .header("apns-topic", topic.as_str());
          req = if push.answered {
            req
              .header("apns-push-type", "background")
              .header("apns-priority", "5")
          } else {
            req
              .header("apns-push-type", "alert")
              .header("apns-priority", "10")
          };
          if let (false, Some(q)) = (push.answered, push.question) {
            // The notification's identifier on the phone: the question's.
            req = req.header("apns-collapse-id", q);
          }
          req.json(&push.payload(&sealed)).send().await
        }
        Route::Relay(url) => {
          // An answer's push has a path of its own: a relay from before
          // answers it 404, rather than showing its placeholder for it.
          let path = if push.answered { "answered" } else { "push" };
          let body = push.relay_body(d, &sealed);
          client
            .post(format!("{url}/{path}"))
            .json(&body)
            .send()
            .await
        }
      };
      out.push(match sent {
        Ok(r) if r.status().is_success() => format!("{}: pushed", d.node),
        Ok(r) => {
          let status = r.status();
          let v = r.json::<Value>().await.unwrap_or(Value::Null);
          let why = v
            .get("reason")
            .or_else(|| v.get("error"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
          format!("{}: {status} {why}", d.node).trim_end().to_owned()
        }
        Err(e) => format!("{}: {e}", d.node),
      });
    }
    out
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A P-256 key in the shape Apple gives one (made here; not a real one).
  fn pem() -> String {
    let rng = ring::rand::SystemRandom::new();
    let doc = ring::signature::EcdsaKeyPair::generate_pkcs8(
      &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
      &rng,
    )
    .unwrap();
    format!(
      "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
      B64.encode(doc.as_ref())
    )
  }

  #[test]
  fn tokens_are_es256_jwts_that_verify() {
    use ring::signature::KeyPair;
    let pem = pem();
    let t = jwt(&pem, "AB12CD34EF", "ABCDE12345", 1_791_000_000).unwrap();
    let parts: Vec<&str> = t.split('.').collect();
    assert_eq!(parts.len(), 3);
    let dec = |s: &str| {
      base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .unwrap()
    };
    let header: Value = serde_json::from_slice(&dec(parts[0])).unwrap();
    assert_eq!(header, json!({"alg": "ES256", "kid": "AB12CD34EF"}));
    let claims: Value = serde_json::from_slice(&dec(parts[1])).unwrap();
    assert_eq!(claims, json!({"iss": "ABCDE12345", "iat": 1_791_000_000}));
    let public = signing_key(&pem).unwrap().public_key().as_ref().to_vec();
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, public)
      .verify(
        format!("{}.{}", parts[0], parts[1]).as_bytes(),
        &dec(parts[2]),
      )
      .expect("the signature verifies");
    assert!(jwt("not a key", "k", "t", 0).is_err());
  }

  fn public(private: &[u8; 32]) -> [u8; 32] {
    super::public_of(private)
  }

  fn open(private: &[u8; 32], wire: &str) -> Option<Vec<u8>> {
    super::open_with(INFO, private, wire)
  }

  #[test]
  fn pushes_are_sealed_for_one_phone() {
    let private = [7u8; 32];
    let wire = seal(&B64.encode(public(&private)), b"{\"body\":\"git push\"}").unwrap();
    assert_eq!(
      open(&private, &wire).as_deref(),
      Some(&b"{\"body\":\"git push\"}"[..])
    );
    // Another phone's key does not open it; nor does tampering.
    assert_eq!(open(&[8u8; 32], &wire), None);
    let mut bad = B64.decode(&wire).unwrap();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert_eq!(open(&private, &B64.encode(bad)), None);
    assert!(seal("not a key", b"x").is_err());
  }

  #[test]
  fn a_question_and_its_answer_push_differently() {
    let phone = Device {
      node: "iphone".into(),
      token: "ab".repeat(32),
      env: "sandbox".into(),
      key: None,
    };
    let mut push = Push {
      title: "Approve on mbp?",
      subtitle: Some("a thread"),
      body: "git push",
      badge: Some(2),
      question: Some("2026-10-18T083843Z-mbp-w39k"),
      node: "mbp",
      answered: false,
      thread: None,
    };
    // Asked: an alert, its notification named by the question.
    let asked: Value = serde_json::from_str(&push.content()).unwrap();
    assert_eq!(asked["question"], "2026-10-18T083843Z-mbp-w39k");
    assert_eq!(asked["body"], "git push");
    assert_eq!(asked["node"], "mbp");
    let p = push.payload("SEALED");
    assert_eq!(p["aps"]["mutable-content"], 1);
    assert_eq!(p["aps"]["badge"], 2);
    let r = push.relay_body(&phone, "SEALED");
    assert_eq!(r["collapse"], "2026-10-18T083843Z-mbp-w39k");
    assert_eq!(r["badge"], 2);
    // Answered: a background push saying only which, nothing shown.
    push.answered = true;
    push.badge = Some(1);
    let answered: Value = serde_json::from_str(&push.content()).unwrap();
    assert_eq!(answered["answered"], "2026-10-18T083843Z-mbp-w39k");
    assert_eq!(answered["badge"], 1);
    assert!(answered.get("body").is_none());
    let p = push.payload("SEALED");
    assert_eq!(p, json!({"aps": {"content-available": 1}, "e": "SEALED"}));
    let r = push.relay_body(&phone, "SEALED");
    assert!(r.get("badge").is_none() && r.get("collapse").is_none());
  }

  #[test]
  fn a_notice_names_its_thread() {
    let push = Push {
      title: "CI on main",
      subtitle: Some("nightly checks"),
      body: "The build is red again.",
      badge: None,
      question: None,
      node: "gamer",
      answered: false,
      thread: Some("2026-10-19T080000Z-gamer-abcd"),
    };
    let said: Value = serde_json::from_str(&push.content()).unwrap();
    assert_eq!(said["thread"], "2026-10-19T080000Z-gamer-abcd");
    assert_eq!(said["title"], "CI on main");
    assert!(said["question"].is_null());
    // Not a question's: no collapse, so notices do not replace each other.
    let phone = Device {
      node: "iphone".into(),
      token: "ab".repeat(32),
      env: "sandbox".into(),
      key: None,
    };
    assert!(push.relay_body(&phone, "SEALED").get("collapse").is_none());
  }

  /// For the app's push-interop check (Rust seals, CryptoKit opens): seals for the key CryptoKit made
  /// (none given: nothing to do).
  #[test]
  fn seal_for_cryptokit() {
    if let Ok(key) = std::env::var("STITCH_PUSH_INTEROP_KEY") {
      println!("SEALED {}", seal(&key, b"{\"body\":\"interop\"}").unwrap());
    }
  }

  #[test]
  fn a_phone_registers_once_per_token_and_key() {
    let dir = tempfile::tempdir().unwrap();
    let home = RecordRoot::init(
      dir.path(),
      "home".parse().unwrap(),
      stitch_store::Role::Home,
    )
    .unwrap();
    let index = Mutex::new(Index::open(&dir.path().join("index.sqlite")).unwrap());
    let phone: NodeName = "iphone".parse().unwrap();
    let key = B64.encode([1u8; 32]);
    // Records are ordered by when they were made, to the millisecond:
    // written apart, so "the latest" is never a tie.
    let apart = || std::thread::sleep(std::time::Duration::from_millis(5));
    assert!(register(&home, &index, &phone, "ABCDEF01", "sandbox", &key).unwrap());
    apart();
    assert!(
      !register(&home, &index, &phone, "abcdef01", "sandbox", &key).unwrap(),
      "the same"
    );
    apart();
    assert!(
      register(&home, &index, &phone, "abcdef02", "sandbox", &key).unwrap(),
      "a new token"
    );
    assert!(register(&home, &index, &phone, "zz", "sandbox", &key).is_err());
    assert!(register(&home, &index, &phone, "ab", "staging", &key).is_err());
    assert!(register(&home, &index, &phone, "ab", "sandbox", "short").is_err());
    assert_eq!(
      devices(&index.lock().unwrap()),
      [Device {
        node: "iphone".into(),
        token: "abcdef02".into(),
        env: "sandbox".into(),
        key: Some(key),
      }]
    );
  }
}
