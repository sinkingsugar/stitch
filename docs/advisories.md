# RustSEC advisories: the triage

`cargo deny check advisories` (`deny.toml`, `just deny`, and CI) fails on
vulnerabilities anywhere in the dependency graph, on unsound and
unmaintained notices for the crates ours depend on directly (cargo-deny's
default scope), and on yanked crates. An advisory that cannot be fixed yet,
or that does not reach what ships, goes in `deny.toml`'s `ignore` with its
reason and date, and here.

The first triage (2026-11-22; 13 vulnerabilities, 1 unsound, 2 yanked
reported). Fixed by `cargo update -p` within the semver range unless a bump
is named; reachability from `cargo tree -i <crate> -e normal` and our code:

| Crate | Advisory | Outcome | Why |
|---|---|---|---|
| rsa 0.10.0-rc.18 | RUSTSEC-2023-0071 (Marvin timing) | fixed: 0.10.0, with russh 0.65 | russh's `rsa` feature (RSA keys in `stitch-ssh`); russh 0.62 pinned the rc |
| webpki-roots 1.0.9 | RUSTSEC-2026-0381 (distrusted emSign roots) | fixed: 1.0.12 | reqwest's TLS roots in the node |
| rustls-webpki 0.103.15 | RUSTSEC-2026-0382 (distrusted intermediates) | fixed: 0.103.17 | rustls, every HTTPS client in the node |
| ctutils 0.4.3 | RUSTSEC-2026-0385 (non-constant-time) | fixed: 0.4.8 | RustCrypto under rsa/ssh-key |
| crypto-bigint 0.7.5 | RUSTSEC-2026-0387 (timing), and yanked | fixed: 0.7.11 | under rsa |
| calamine 0.37.0 | RUSTSEC-2026-0436 (CFB/OLE2 panic, OOM) | fixed: 0.38 (bump in `stitch-node`) | reachable: `web/files.rs` reads fetched `.xls`/`.xlsb` |
| rmcp 3.5.1 | RUSTSEC-2026-0457 (OAuth SSRF filter) | fixed: 3.11.0 | OAuth client feature not enabled |
| rmcp 3.5.1 | RUSTSEC-2026-0461 (server schema cache) | fixed: 3.11.0 | server transport not used |
| rmcp 3.5.1 | RUSTSEC-2026-0462 (unbounded response body) | fixed: 3.11.0 | reachable: MCP Streamable HTTP client |
| libsqlite3-sys 0.38.2 | RUSTSEC-2026-0459 (SQLite < 3.54, CVE-2026-119375) | fixed: 0.39.1 (rusqlite 0.41 in `stitch-store`/`stitch-node`, Otter pin `f3f9e0f`), 2026-11-23 | `links = "sqlite3"` allows one version, so it waited for `otter-sqlite` to move to rusqlite 0.41 first (ignored until then). Local only (a crafted database and `-wal` pair) |
| rustix 1.1.5 | RUSTSEC-2026-0486 (xattr, unsound) | fixed: 1.1.6 | xattr functions not called by us |
| russh 0.62.7 | RUSTSEC-2026-0492 (client callbacks for unopened channels) | fixed: 0.65.4 (bump in `stitch-ssh`) | reachable: the SSH client; `check_server_key` now takes `PublicKeyOrCertificate` (certificates refused: none advertised) |
| russh 0.62.7 | RUSTSEC-2026-0495 (agent server ignores confirm) | fixed: 0.65.4 | agent server not used (agent client only) |
| url 2.5.8 | RUSTSEC-2026-0592 (`Url::origin` on nested `blob:`) | fixed: 2.5.9 | everywhere URLs are parsed |
| async-executor 1.14.0 | yanked | fixed: 1.14.1 | |

With `unsound`/`unmaintained = "all"` the transitive graph adds more (heapless's unsound notices; paste, ttf-parser, zvariant, zbus_names, chrono unmaintained): not in the check, open for a decision.

With `unsound`/`unmaintained = "all"` the transitive graph adds more
(heapless's unsound notices; paste, ttf-parser, zvariant, zbus_names,
chrono unmaintained): not in the check, open for a decision.
