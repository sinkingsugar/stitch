# Vault prototype protocol v3

Local experimental format, not a production compatibility/security promise; it may change again before it freezes. v3 (2026-10-27) replaced one-object-per-item with content-defined chunks in padded packs. No connection to a running Stitch store. `stitch-vault` handles bytes and returns an inert read-only recovery dataset; the hosted service (a Cloudflare Worker, Fragcolor's, not in this repository) is tested under Miniflare (real workerd, emulated R2 + SQLite DO). v2 (2026-10-27) replaced v1 after Claude's adversarial review: v1 recovery accepted replayed commits and commits from never-enrolled keys, and one reused name made every recovery fail.

## Keys

ring's OS randomness, HKDF-SHA256, HMAC-SHA256, SHA256, ChaCha20-Poly1305 and Ed25519; x25519-dalek for sealing. Three kinds of secret, never mixed:

- **Epoch roots** (32 random bytes each): content keys. HKDF salt = vault ID, input = the epoch's root, info `stitch vault v3 {purpose}` (`dedup`, `aead`). Every pack and commit names its epoch; chunk IDs and keys are per epoch.
- **Device keys**: an Ed25519 signing key (commits, and statements if the device is an admin) and an X25519 sealing key (opens envelopes sealed to it).
- **The recovery kit**: one 32-byte seed, independent of the roots. HKDF (salt `stitch vault kit v3`) derives the **anchor** (Ed25519, may sign membership) and the kit's X25519 sealing key. Text `sv2:{vault}:{seed}:{checksum}` (checksum = first 8 bytes of SHA256 of the rest; catches transcription errors only). The **vault ID is the first 16 bytes of SHA256 of the first kit's anchor**, so the vault ID alone identifies the true genesis: no other trust anchor is needed by a device or a replacement kit.

Roots travel only inside **envelopes**: ephemeral X25519 to the recipient's key, HKDF (salt vault, input shared ‖ ephemeral ‖ recipient, info `envelope {to}`), ChaCha20-Poly1305 with AAD `stitch vault envelope v3\n{vault}\n{to}\n`; the plaintext lists every root the recipient may read. Secrets live in zeroizing memory, are never Debug/Serialize, logged or sent in the clear. Possession of a kit grants no service access by itself.

Cross-language known-answer vector: `crates/stitch-vault/fixtures/crypto-v3.json`, computed by Node/OpenSSL (`crypto.test.js`) and checked by Rust (kit anchor and sealing key, vault ID from the anchor, epoch subkeys, chunk ID, per-chunk key, Padmé frame, AEAD).

## Object format

Designed against the measured mix of a real Stitch store (gamer, 2026-10-27, stats only, nothing read or copied): 19,476 files, 1.62 GB; records p50 610 B / p99 21 KB, transcript segments p50 13 KB / p99 193 KB, blobs p50 4.8 KB / p99 3 MB, max 14 MB; only 55 files of 4 MiB or more, but 21% of the bytes. Scaled to 100 GB that is ~1.2M items.

- **Items** (a record, transcript segment, blob or file, `kind` informational) are cut into **chunks** by FastCDC (v2020; min 1 MiB, average 4 MiB, max 16 MiB, the algorithm's largest bounds). Items up to 1 MiB are one chunk. An edit re-uploads only the chunks around it (tested: a 40 MiB file with an insertion in the middle uploads < 1/3 again; unit test: at most 2 chunks change).
- **Chunk ID** = HMAC(dedup key, `chunk` || NUL || SHA256(plaintext)): vault-keyed, kind-free, so identical bytes are stored once per vault; nothing is comparable across vaults.
- **Sealed chunk** = 12-byte random nonce || ChaCha20-Poly1305(frame), key = HMAC(aead key, `stitch vault chunk key v3\n{epoch}\n{id}\n`), AAD `stitch vault chunk v3\n{vault}\n{epoch}\n{id}\n`. Frame = data || 0x80 || zeros to the **Padmé** length of len+1 (leaks O(log log n) bits; overhead with AEAD on the measured mix: 1.31%). Opening checks AEAD, padding and the keyed ID. One key per chunk identity: random nonces only repeat across encryptions of the same plaintext.
- **Pack** = sealed chunks back to back, zero filler, then the sealed **index** (`{version: 3, chunks: [{id, offset, length}]}`, same sealing with `pack index` labels and the pack ID). The filler pads the whole object to a Padmé length. A pack is closed when the next chunk would pass **16 MiB** (one 16 MiB chunk may stand alone); objects are at most 40 MiB. Pack IDs are random (packs are arbitrary groupings). The server sees pack sizes (Padmé-rounded), counts and timing, and at recovery which byte ranges are read.
- **Manifest**: encrypted JSON `{version: 3, entries: [{name, item}]}`, `item` = `{kind, size, chunks: [chunk IDs]}` or `null` (a tombstone). Stored as a chunk in one of its commit's packs. Items name chunk IDs, never locations, so repacking (phase 3) moves chunks without rewriting manifests: locations come from pack indexes.
- A writer keeps the set of chunk IDs already in the vault and packs only new chunks; the set advances only when the commit is accepted. Concurrent devices may both store a chunk (harmless; repacking deduplicates).

**Pack size (16 MiB)**, modelled on the measured mix at 100 GB: unpacked ~1.2M PUTs ($5.43 once); 4 MiB packs ~24,000 ($0.11), 8 MiB ~12,000 ($0.05), **16 MiB ~6,000 ($0.03)**, 32 MiB ~3,000. 16 MiB keeps PUTs on the order of 10^4, uploads in one streamed request (Workers accept 100 MB bodies, so no multipart), and a lost upload costs at most one pack. Recovery never needs whole packs: one ranged read per index, per manifest, and per run of consecutive chunks of an item (runs up to 32 MiB).

## Membership, admission and rotation

A hash-chained log of statements (`members.rs`); one signed-text shape for all:

```
stitch vault member v3\n{vault}\n{seq}\n{previous}\n{action}\n{signer}\n{device}\n{role}\n{seal}\n{through}\n{epoch}\n{anchor}\n
{to}\t{sealed}\n   (one line per envelope, sorted by recipient)
```

- `genesis` (seq 1, signed by the anchor it names, which the vault ID must commit to): the kit's sealing key and epoch 1's root sealed to `kit`.
- `add`: a device, its role (`admin`, `writer`, `reader`) and sealing key; seals it every current root. An `admin` signed by an admin device alone is admitted as a `writer` and promoted at `at + 7 d` (a delayed action, cancellable; applied only if the device is still current); at once when the kit signs or co-signs, or two admins co-sign under `approvals2`.
- `revoke`: a device and `through`, its last valid commit sequence.
- `epoch`: rotation. A fresh root for epoch e+1, sealed together with all earlier roots to **every** current device and the kit (anything else is refused). Old epochs stay readable to those holders; the revoked device holds only the roots it had.
- `anchor`: kit replacement. A new kit's anchor and sealing key; the old kit stops signing and opens nothing sealed after it.

**Who may sign:** the current anchor (the kit) or a current admin device. Ordinary writers and readers cannot: the service refuses to store such a statement (403) and recovery rejects it. **Who posts:** an admin device with its own service credential; the service checks signature, chain and transition, applies revocations to credentials, and never authors a statement. The local bootstrap token (`ADMIN_TOKEN`, `LOCAL_PROTOTYPE` only) remains to post a vault's genesis and admit and enroll its first admin; production enrollment replaces it (phase 5).

**Delayed destructive actions.** The signed text continues after `{anchor}` with `{at}\n{target}\n{cosigner}\n{commitment}\n` (signing time in ms, the statement a `cancel` cancels, an optional co-signer whose `cosignature` covers the same text). Actions `delete` (the vault), `settings` that shortens retention, `revoke` of an admin, `add` of an admin by an admin device alone, and `anchor` are destructive: they join the state's `pending` list and take effect at `at + 7 days`, in statement order, if not cancelled and if their signer is then still the anchor or a current admin; with `approvals2` and a valid co-signature by a distinct admin (or the kit) they apply at once. `cancel` (any admin or the kit) removes a pending action. `freeze` (immediate, any admin or the kit; `through` = last valid commit sequence) suspends a device: no access, no signing, left out of `epoch` envelopes; `unfreeze` lifts it and seals the device the keyring, but a freeze by the kit only the kit can lift. Statement times must be within ±5 minutes of the service clock and not before the previous statement's. Before applying a statement, every pending action due by its time is applied; a verifier finally applies those due by its own clock. `GET /members` returns `{members, pending, deleted}`; a deleted vault answers 410 everywhere.

**Recovery kit after rotation:** no new kit is needed. Every `epoch` statement seals the full keyring to the kit's sealing key, and, while a kit replacement waits, to the next kit (`nextkit`); `genesis` and `epoch` carry `commitment` = SHA256(`stitch vault root v3\n{vault}\n{epoch}\n` ‖ root), which every holder checks for every root it opens (and never replaces a root it holds). The kit's envelope is deterministic (ephemeral key and nonce derived from the epoch root, the statement and the plaintext), so devices holding the roots can recompute it and detect a garbled one. A lost or exposed kit is replaced with an `anchor` statement.

**What this protects:** commits by keys never admitted, statements by non-admins, and content after a rotation are rejected or unreadable for revoked devices. A revoked device and a colluding service can still present a forked log from before the revocation; callers pin `minimum_members` to catch it.

## Commits and device chains

```
stitch vault commit v3\n{vault}\n{epoch}\n{device}\n{sequence}\n{previous}\n{manifest}\n
{pack}\t{digest}\t{size}\t{index_offset}\t{index_length}\t{epoch}\n   (one line per new pack, sorted by ID, 1..=1024)
uses\t{pack}\n   (existing packs the manifest's items reuse chunks from, sorted, 0..=1024)
```

Each device signs its own chain: `sequence` counts from 1, `previous` is SHA256 of the device's previous commit's signed text (64 zeros for sequence 1). `manifest` is the manifest's chunk ID, which must be in one of the commit's packs; the pack lines are the packs this commit introduces (digest of the whole object, the index location, `index_offset + index_length = size`). Commit JSON is at most 256 KiB. A device must publish the commits it makes in order (retrying the same commit is idempotent); its head is local durable state (`Device::resume`), never taken from the service's word alone.

Names are opaque strings (1..=1024 bytes), unique within a manifest, never filesystem paths. An item's chunks must lie in the commit's new packs, its `uses`, or (after a starting checkpoint) that checkpoint's packs; recovery checks this, and the service refuses a commit whose `uses` names a pack it no longer has. Later commits (by global cursor) supersede earlier ones; across devices the service chooses that order (it cannot reorder one device's chain). The service never sees names, chunk IDs or plaintext.

## Service routes

Authentication: production uses signed requests only (`SIGNED_AUTH`): `x-vault-auth: {ed25519 public}:{unix ms}:{signature}` over `stitch vault request v3\n{vault}\n{METHOD}\n{path?query}\n{unix ms}\n{x-content-sha256 or empty}\n`, accepted within ±5 minutes and **once**: the DO spends each `{unix ms}:{signature}` in a nonce table kept for twice the window, so a captured request replayed is 401 (clients make their times strictly increase, since Ed25519 signs the same text identically); the body must hash to `x-content-sha256`. A key is authorized by its standing in the membership log (role, not revoked); the anchor may read and post membership statements; `/devices` and `/clock` do not exist. The bearer descriptions below apply to the local prototype only.

Vault-scoped under `/v1/vaults/{vault}`; device bearers are stored as SHA256 only.

- `POST /members` (an admin device; the local bootstrap token for a vault's first statements): the service verifies the signer is the anchor or a current admin (403 otherwise), the chain and the transition (409); the same statement again is idempotent.
- `GET /members` (any device): `{members: [...], pending: [...], deleted}` (the service's view of the pending actions; clients recompute both from the verified log). Budget: ordinary statements 1024 and 6 MiB; cancel and freeze (≤ 2 KiB each) and kit-signed statements (6 MiB) remain possible after it is spent; 3072 statements in all (507 past a bound). A revoke or freeze whose `through` is below the device's accepted head needs the kit, or two admins under `approvals2` (409).
- `PUT /devices/{public-key}` (an admin device, or the bootstrap token): body `{token_hash, role}`. Needs a current `add` statement with that role (403 otherwise); the admin token's own hash is refused; identity/role cannot be silently replaced; revoked identities are not re-enrolled. Local stand-in for independent step-up enrollment, not a production account API.
- `DELETE /devices/{public-key}` (admin): immediate credential revocation (emergency; the cryptographic revocation is a `revoke` statement); unknown device 404.
- `PUT /objects/{id}` (writer): a pack, `x-content-sha256`, `content-length` up to 40 MiB. Same ID+digest is a retry (200); different ciphertext 409 (client fetches and authenticates the indexed winner); 503 + `retry-after` while another upload holds the ID's reservation; 408 if the body does not complete within the deadline; 507 over quota. `?room=repack` (admin devices only, used by checkpoint repacking): may pass the quota by the *repack room*, max(2 × 40 MiB, 2% of the quota), so a full vault can still repack before GC frees the old packs; ordinary uploads never use it.
- `GET /objects/{id}` (reader/writer): indexed objects only (in-flight reservations and stray R2 bytes are 404). With `Range: bytes=first-last` (one range, both ends) it answers 206 with `content-range`, read from R2 as a ranged get; malformed or out-of-bounds ranges are 416.
- `POST /commits` (writer): the signer must be the authenticated device; `sequence`/`previous` must continue the device's last accepted commit exactly (forks and gaps are 409; the same commit again returns its cursor); every listed pack must be indexed with the exact digest/size. Global cursor assigned in one SQL transaction.
- `GET /commits?after=N&through=H` (reader/writer): pages of 100, `{head, entries}`. Omit `through` on the first page to pin the current head; later pages request exactly that prefix.

## Service concurrency and crash consistency

Object bytes never pass through the DO. `PUT /objects/{id}` is handled by the stateless Worker: (1) `reserve` RPC to the vault's DO; (2) stream the body into a create-only R2 put through a `FixedLengthStream` of the declared `content-length` (required: 411 without), with R2's own `sha256` check and the body deadline; (3) `finalize` RPC. A short, long, slow (408) or mismatched (400) body releases its reservation when R2 holds nothing. `GET /objects/{id}`: `authorizeRead` RPC (device and indexed), then the Worker streams from R2. An already indexed ID+digest answers 200 without reading the body.

The DO holds no gate across caller I/O (the first prototype wrapped each request in `blockConcurrencyWhile`; a writer trickling a body blocked, and after the 30 s limit reset, every other request including its own revocation: reproduced in review). Requests authenticate synchronously before reading anything (strangers cannot occupy the reader), read the bounded body with a deadline (`BODY_DEADLINE_MS`, default 15 s, then 408), and change state only in synchronous SQLite sections, which are atomic in a DO, rechecking the device after every await. A revocation wins against every request that has not reached its final section.

Upload bookkeeping: (1) reserve, in one SQL transaction: recheck the writer, return 200/409 if indexed, refuse while a live reservation holds the ID (503), take over an expired one, check quota against running totals, insert a reservation with a random lease and expiry (`RESERVATION_LEASE_MS`, default 120 s) and add its size to the totals; `storage.sync()` makes it durable before any bytes leave. (2) the Worker's R2 create-only put (`onlyIf: etagDoesNotMatch "*"`, plus `sha256`): never overwrites; if bytes already exist (a crash after an earlier put, or a stray object) their recorded digest/size are adopted. (3) finalize: if the lease is still ours, index the winner and move its size from reservation to object; then a writer revoked during the transfer gets 401 (its bytes are accounted, it cannot publish), and a different winner 409. The lease (`RESERVATION_LEASE_MS`) must exceed the body deadline in deployment; if it lapses mid-transfer, the first complete create-only put wins and the other side adopts it. A crash between (1) and (3) leaves a reservation that still counts against quota until a retry after its lease takes it over and adopts or writes the bytes: nothing is under-counted. Running totals (`usage`) replace per-upload `SUM`/`COUNT` scans. Abandoned reservations and uncommitted objects still count and are not collected (no GC). Conditional put and `sha256` behave this way in workerd; confirming the same on production R2 is open.

Test-only fault injection (`FAULT_INJECTION=true` plus an `x-fault-inject` header, local prototype only): the Worker fails after the reservation or after the R2 put, or the DO resets itself (`ctx.abort()`) at finalization (`vault/test/faults.test.js`).

## Recovery

`Client::recover(key, options)`: verify the membership log (above), then read the commit log through a chosen cursor (`through`, default the current head) and, per commit: parse, verify the signature, require the signer to be a trusted writer, not past its revocation `through`, and the exact next link of its chain (otherwise `replayed or forked`, `sequence gap` or `broken chain`; once a chain breaks, that device's later commits are rejected too). For each accepted commit, each new pack's index is read (one ranged read) and its chunk locations recorded, then the manifest chunk is read and verified; an item naming a chunk with no known location makes the commit incomplete. Name changes apply in cursor order. Then each live item is fetched once (ranged reads of runs of consecutive chunks), every chunk authenticated, and its size checked.

Problems are reported, not fatal: `rejected` (commits not applied), `incomplete` (authentic commits whose content could not be verified: their changes are missing, so later state may be stale), `missing` (pinned devices or revoked devices whose chain stops short, and live names whose object is unavailable or corrupt). `complete()` is true only with none. Errors remain for transport failures, malformed pages, a head below `minimum_cursor`, a membership log shorter than `minimum_members`, and resource limits. Output is staged into a new directory the caller names (it must not exist): `items/<key>` holds each live item's verified plaintext once (key = SHA256 of its chunk-ID list) (0600, written then renamed), `index.json` the report and the live entries (name, item key, kind, plaintext size, cursor); the directory is 0700. Entry names are never used as paths. Nothing is imported and no node starts; a future preview/import step is the only way anything leaves staging. On error, staging may hold verified plaintext without `index.json`; the caller removes it.

Limits (`Limits`, defaults sized for the 100 GB plan): commits read, chunk locations held, manifest entries processed (superseded and deleted ones included), name bytes, download bytes (every ranged read, charged before it is made) and staged plaintext bytes; exceeding one is `Error::Limit` naming it. The staged set doubles as the verified-item cache: one fetch per item however many names share it, and superseded or deleted items are never fetched (tested: recovering one small note next to a deleted 12 MiB file reads under 64 KiB). The live name map stays in memory, bounded by the entry and name-byte limits.

What a malicious service can still do: present an older consistent prefix (caught only by the caller's pins: `minimum_cursor`, `minimum_members`, per-device `pins`), withhold the newest commits of devices nobody pinned, choose the cross-device order of commits (which write to the same name wins), and see metadata (sizes, timing, object sharing, which device published what). Fresh kit-only recovery has no pins, so it cannot prove it saw the newest history. This verifies the declared manifests, not a node store's inventory: the future store adapter must enumerate every original attachment/system-prompt/source reference before calling a backup complete.

## Checkpoints, retention and GC

```
stitch vault checkpoint v3\n{vault}\n{cursor}\n{manifest}\n{manifest_epoch}\n{signer}\n
{pack}\t{digest}\t{size}\t{index_offset}\t{index_length}\t{epoch}\n   (every pack the state needs, sorted)
```

A checkpoint's manifest lists every live item at `cursor`. Routes (all admin devices except reads): `POST /checkpoints` (signer must be the anchor or a current admin, packs indexed with exact digest/size, cursor at most the head), `GET /checkpoints` (any device), `POST /gc` (403 unless `GC_ENABLED`), and the test-only `POST /clock` (`TEST_CLOCK`, bootstrap token) that moves the service clock. Membership gains `settings` (`through` = retention days 1..=3650, `role` = `approvals1`/`approvals2`). The GC rule, retention and repacking are in [vault.md](vault.md); there is no per-item erase; fault injection `gc-before-r2-delete` resets the object between unindexing and deleting.

## Quota, retention and destructive operations

Bounds: pack size 40 MiB, 1024 packs and 1024 `uses` per commit, commit JSON 256 KiB, checkpoints up to 100,000 packs, pages of 100, data bytes. Control (enrollment, membership, revocation, deletes, GC) never consumes data quota. Prototype caps: 1,000,000 objects (packs), 1,000,000 commits, 64 device credentials. Quota rejects new ciphertext rather than pruning history. GC is off unless `GC_ENABLED`; ordinary devices cannot checkpoint, collect, erase or change retention.

## Required before production

External crypto/security review and frozen test vectors; an account layer; independently authenticated administration and recovery;  production measurement of checkpoint and GC cost; independent service disaster recovery; real node sync adapter and UI; account/billing/distribution policy. Cloudflare deployments/spending are not part of this slice.
