//! The web as an app's thread reaches it (design §2.22): public addresses
//! only, checked when connecting, after DNS. A client of its own, with no
//! proxy, whose resolver gives only public addresses (a name that resolves
//! to any other one is refused whole, so a rebinding answer mixing them
//! gets nothing), and whose redirect policy checks each hop to an IP
//! literal (which no resolver sees). Pages are read without JavaScript:
//! the headless browser has no such filter for a page's own requests.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

/// What an app's thread is told when it reaches for anything but the
/// public internet.
pub(crate) const REFUSED: &str =
  "an app's thread reaches no address of this machine or its network";

/// Largest page or file an app's thread fetches.
const MAX_BYTES: usize = 10 * 1024 * 1024;

/// Whether an address is on the public internet: not this machine, its
/// networks, or a range that leads to them (mapped and translated IPv6
/// whose IPv4 is not public).
pub(crate) fn public_ip(ip: IpAddr) -> bool {
  match ip {
    IpAddr::V4(v4) => public_v4(v4),
    IpAddr::V6(v6) => public_v6(v6),
  }
}

fn public_v4(ip: Ipv4Addr) -> bool {
  let o = ip.octets();
  !(ip.is_unspecified()
    || ip.is_loopback()
    || ip.is_private()
    || ip.is_link_local()
    || ip.is_broadcast()
    || ip.is_multicast()
    || ip.is_documentation()
    // "This network", CGNAT and Tailscale, IETF protocol assignments,
    // benchmarking, reserved.
    || o[0] == 0
    || (o[0] == 100 && (o[1] & 0xc0) == 64)
    || (o[0] == 192 && o[1] == 0 && o[2] == 0)
    || (o[0] == 198 && (o[1] & 0xfe) == 18)
    || o[0] >= 240)
}

fn public_v6(ip: Ipv6Addr) -> bool {
  if let Some(v4) = ip.to_ipv4_mapped() {
    return public_v4(v4);
  }
  let s = ip.segments();
  let v4 = |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
  // `::/96`: unspecified, loopback and the old IPv4-compatible ones;
  // `::ffff:0:0:0/96`, IPv4-translated (never routed).
  if s[..6] == [0; 6] || (s[..4] == [0; 4] && s[4] == 0xffff && s[5] == 0) {
    return false;
  }
  // NAT64 (`64:ff9b::/96`): as the IPv4 it translates to; its local-use
  // prefix (`64:ff9b:1::/48`) is a network's own.
  if s[0] == 0x64 && s[1] == 0xff9b {
    return s[2..6] == [0; 4] && public_v4(v4(s[6], s[7]));
  }
  // 6to4 (`2002::/16`): as its IPv4.
  if s[0] == 0x2002 {
    return public_v4(v4(s[1], s[2]));
  }
  !(ip.is_multicast()
    // ULA, link-local, site-local (deprecated, still routed by some).
    || (s[0] & 0xfe00) == 0xfc00
    || (s[0] & 0xffc0) == 0xfe80
    || (s[0] & 0xffc0) == 0xfec0
    // Discard, documentation, Teredo, benchmarking, ORCHID.
    || (s[0] == 0x0100 && s[1..4] == [0; 3])
    || (s[0] == 0x2001 && s[1] == 0x0db8)
    || (s[0] == 0x2001 && s[1] == 0)
    || (s[0] == 0x2001 && s[1] == 2 && s[2] == 0)
    || (s[0] == 0x2001 && (s[1] & 0xfff0) == 0x0010)
    || (s[0] == 0x3fff && (s[1] & 0xf000) == 0))
}

/// Whether a URL may be asked for: HTTP(S), no credentials, and a host
/// that is a public address or a name (names are checked once resolved).
pub(crate) fn url_ok(u: &url::Url) -> Result<(), String> {
  if !matches!(u.scheme(), "http" | "https") || !u.username().is_empty() || u.password().is_some() {
    return Err("url must be HTTP(S), without credentials in it".into());
  }
  match u.host() {
    Some(url::Host::Ipv4(ip)) if !public_v4(ip) => Err(format!("{u}: {REFUSED}")),
    Some(url::Host::Ipv6(ip)) if !public_v6(ip) => Err(format!("{u}: {REFUSED}")),
    Some(url::Host::Domain(d)) => {
      let d = d.trim_end_matches('.').to_ascii_lowercase();
      if d == "localhost" || d.ends_with(".localhost") {
        Err(format!("{u}: {REFUSED}"))
      } else {
        Ok(())
      }
    }
    Some(_) => Ok(()),
    None => Err("url must have a host".into()),
  }
}

/// A resolver that gives a name's addresses only when every one is public.
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
  fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
    let host = name.as_str().to_owned();
    Box::pin(async move {
      let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
      if addrs.is_empty() || !addrs.iter().all(|a| public_ip(a.ip())) {
        let e: Box<dyn std::error::Error + Send + Sync> =
          format!("{host} resolves to an address that is not public: {REFUSED}").into();
        return Err(e);
      }
      Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
    })
  }
}

/// The client an app's thread fetches with.
pub(crate) fn client() -> reqwest::Client {
  let _ = rustls::crypto::ring::default_provider().install_default();
  reqwest::Client::builder()
    .user_agent(concat!(
      "Mozilla/5.0 (compatible; Stitch/",
      env!("CARGO_PKG_VERSION"),
      ")"
    ))
    .no_proxy()
    .dns_resolver(Arc::new(PublicOnly))
    .connect_timeout(Duration::from_secs(20))
    .timeout(Duration::from_secs(60))
    .redirect(reqwest::redirect::Policy::custom(|a| {
      if a.previous().len() >= 10 {
        return a.error("more than 10 redirects");
      }
      match url_ok(a.url()) {
        Ok(()) => a.follow(),
        Err(e) => a.error(e),
      }
    }))
    .build()
    .expect("an HTTP client with these settings builds")
}

/// What a fetch brought: its final URL, type and bytes.
pub(crate) struct Got {
  pub url: String,
  pub status: u16,
  pub mime: String,
  pub bytes: Vec<u8>,
}

/// Fetches a URL for an app's thread.
pub(crate) async fn get(http: &reqwest::Client, url: &str) -> Result<Got, String> {
  let parsed = url::Url::parse(url.trim()).map_err(|e| format!("invalid URL: {e}"))?;
  url_ok(&parsed)?;
  let mut response = http.get(parsed).send().await.map_err(|e| describe(&e))?;
  let status = response.status().as_u16();
  let final_url = response.url().to_string();
  let mime = response
    .headers()
    .get(reqwest::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .and_then(|v| v.split(';').next())
    .unwrap_or("")
    .trim()
    .to_ascii_lowercase();
  let mut bytes = Vec::new();
  while let Some(chunk) = response.chunk().await.map_err(|e| describe(&e))? {
    if bytes.len() + chunk.len() > MAX_BYTES {
      return Err(format!("{final_url} is over {} MiB", MAX_BYTES >> 20));
    }
    bytes.extend_from_slice(&chunk);
  }
  Ok(Got {
    url: final_url,
    status,
    mime,
    bytes,
  })
}

/// A request's error with its causes, which say why (the resolver's
/// refusal is a cause).
fn describe(e: &reqwest::Error) -> String {
  let mut out = e.to_string();
  let mut source = std::error::Error::source(e);
  while let Some(s) = source {
    out.push_str(&format!(": {s}"));
    source = s.source();
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_public_addresses_are_reached() {
    for local in [
      "127.0.0.1",
      "0.0.0.0",
      "10.1.2.3",
      "172.16.0.1",
      "192.168.0.113",
      "169.254.169.254",
      "100.64.0.1",
      "100.100.100.100",
      "255.255.255.255",
      "224.0.0.1",
      "198.18.0.1",
      "192.0.0.8",
      "240.0.0.1",
      "::1",
      "::",
      "::ffff:127.0.0.1",
      "::ffff:192.168.0.1",
      "::127.0.0.1",
      "::ffff:0:7f00:1",
      "64:ff9b::7f00:1",
      "64:ff9b::c0a8:1",
      "64:ff9b:1::1",
      "2002:7f00:1::",
      "2002:c0a8:101::1",
      "fd00::1",
      "fe80::1",
      "fec0::1",
      "ff02::1",
      "2001:db8::1",
      "2001:0:4136:e378::1",
    ] {
      let ip: IpAddr = local.parse().unwrap();
      assert!(!public_ip(ip), "{local}");
    }
    for public in [
      "93.184.216.34",
      "1.1.1.1",
      "2606:4700:4700::1111",
      "::ffff:1.1.1.1",
      "64:ff9b::101:101",
      "2002:101:101::1",
    ] {
      let ip: IpAddr = public.parse().unwrap();
      assert!(public_ip(ip), "{public}");
    }
  }

  #[test]
  fn urls_naming_this_machine_are_refused_before_any_request() {
    for u in [
      "http://127.0.0.1:7177/",
      "http://[::ffff:127.0.0.1]/",
      "http://2130706433/",
      "http://0x7f.1/",
      "http://localhost:7177/",
      "http://app.localhost/",
      "http://[fec0::1]/",
      "http://100.64.1.1/",
      "file:///etc/passwd",
      "https://user:pw@example.com/",
    ] {
      assert!(url_ok(&url::Url::parse(u).unwrap()).is_err(), "{u}");
    }
    assert!(url_ok(&url::Url::parse("https://example.com/a").unwrap()).is_ok());
  }

  #[tokio::test]
  async fn names_resolving_here_are_refused_after_dns() {
    // A server on loopback: neither its address nor a name resolving to
    // it reaches it.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let http = client();
    // `localhost` resolves to loopback: the resolver refuses it (the URL
    // check would too, so ask the resolver directly).
    let r = reqwest::dns::Resolve::resolve(&PublicOnly, "localhost".parse().unwrap()).await;
    assert!(r.is_err());
    let e = get(&http, &format!("http://127.0.0.1:{port}/"))
      .await
      .err()
      .unwrap();
    assert!(e.contains(REFUSED), "{e}");
    drop(listener);
  }
}
