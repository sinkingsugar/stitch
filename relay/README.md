# Stitch's push relay

A Cloudflare Worker that forwards pushes from a user's nodes to their phones
through Apple's push service (design §5, milestone 4), for the Stitch app from
the store: only the team that owns the app's ID can push to it, so a user's
nodes, holding no key of that team's, push through this relay, which does.

It keeps nothing and cannot read a notification's content; it sees only the
device token and metadata such as the badge:

- A push's content is sealed on the node for the phone (X25519, HKDF-SHA256,
  ChaCha20-Poly1305: `crates/stitch-node/src/apns.rs`); the phone's
  notification service extension opens it. The relay forwards the ciphertext
  and shows only its own placeholder ("Something waits on you"), so it cannot
  be used to put arbitrary text on a phone.
- The device token is the credential, as it is for APNs itself: only the
  phone and the user's nodes know it. Pushes are capped at 30 a minute per
  token.

A build signed by a user's own team needs no relay: its nodes push with that
team's key (`stitch apns set`).

## Deploy

```sh
cd relay
wrangler secret put APNS_KEY      # the AuthKey_<id>.p8, whole
wrangler secret put APNS_KEY_ID   # its key ID
wrangler secret put APNS_TEAM_ID  # the team that owns com.fragcolor.stitch
wrangler deploy
```

Nodes push through `https://stitch-relay.clamor.workers.dev` by default
(`apns::RELAY`), with nothing to set up; another deployment is named on each
node with `stitch apns relay <url>`. `stitch apns test` pushes one.

## Test

```sh
npm test                              # the relay: checks, payload, its APNs token
# the app's push-interop check: a push sealed by the node opens with CryptoKit
# (`cargo test -p stitch-node --lib apns::tests::seal_for_cryptokit`, its Rust side)
```

## API

`POST /push` with `{"token": "<hex>", "env": "sandbox" | "production", "e":
"<base64 sealed content>", "badge": 3}`: 200 `{"ok": true}`; 400 for a push it
does not take, 429 past the cap, 502 with APNs's reason.
