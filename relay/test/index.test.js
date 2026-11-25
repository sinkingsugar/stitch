// The relay's checks, the payload it builds, and its APNs token, with
// Node's own test runner (`node --test`): no dependencies.

import assert from "node:assert/strict";
import { test } from "node:test";

import relay, { MAX_CIPHERTEXT, PLACEHOLDER, headers, jwt, parsePush, payload } from "../src/index.js";

const token = "ab".repeat(32);

test("pushes are checked", () => {
  assert.deepEqual(parsePush({ token, env: "sandbox", e: "AAAA", badge: 2 }), {
    token,
    env: "sandbox",
    e: "AAAA",
    badge: 2,
  });
  for (const bad of [
    null,
    { token: "nothex", env: "sandbox", e: "AAAA" },
    { token: token.toUpperCase(), env: "sandbox", e: "AAAA" },
    { token, env: "staging", e: "AAAA" },
    { token, env: "sandbox", e: "" },
    { token, env: "sandbox", e: "A".repeat(MAX_CIPHERTEXT + 1) },
    { token, env: "sandbox", e: "<script>" },
    { token, env: "sandbox", e: "AAAA", badge: -1 },
    { token, env: "sandbox", e: "AAAA", collapse: "x".repeat(65) },
    { token, env: "sandbox", e: "AAAA", collapse: "a b" },
  ]) {
    assert.throws(() => parsePush(bad), JSON.stringify(bad));
  }
});

test("the payload shows only the relay's placeholder", () => {
  const p = payload({ token, env: "sandbox", e: "QUJD", badge: 1 });
  assert.deepEqual(p.aps.alert, PLACEHOLDER);
  assert.equal(p.aps["mutable-content"], 1);
  assert.equal(p.aps.badge, 1);
  assert.equal(p.e, "QUJD");
  assert.equal(payload({ token, env: "sandbox", e: "QUJD" }).aps.badge, undefined);
});

test("a question's notification is named by it; an answer's push shows nothing", () => {
  const q = parsePush({ token, env: "sandbox", e: "QUJD", collapse: "2026-10-18T083843Z-mbp-w39k" });
  assert.equal(headers(q, "T", "topic")["apns-collapse-id"], "2026-10-18T083843Z-mbp-w39k");
  assert.equal(headers(q, "T", "topic")["apns-push-type"], "alert");
  const a = { token, env: "sandbox", e: "QUJD", answered: true };
  assert.deepEqual(payload(a), { aps: { "content-available": 1 }, e: "QUJD" });
  const h = headers(a, "T", "topic");
  assert.equal(h["apns-push-type"], "background");
  assert.equal(h["apns-priority"], "5");
  assert.equal(h["apns-collapse-id"], undefined);
});

test("a call is a VoIP push to the app's .voip topic, its sealed call alone", async () => {
  const c = { token, env: "sandbox", e: "QUJD", voip: true };
  assert.deepEqual(payload(c), { e: "QUJD" });
  const h = headers(c, "T", "com.fragcolor.stitch", 1_795_000_000_000);
  assert.equal(h["apns-topic"], "com.fragcolor.stitch.voip");
  assert.equal(h["apns-push-type"], "voip");
  assert.equal(h["apns-priority"], "10");
  assert.equal(h["apns-expiration"], "1795000030");
  assert.equal(h["apns-collapse-id"], undefined);
  // Checked as any push, before APNs.
  const r = await relay.fetch(
    new Request("https://relay.example/voip", { method: "POST", body: JSON.stringify({ token: "x" }) }),
    {},
  );
  assert.equal(r.status, 400);
});

test("the APNs token is an ES256 JWT that verifies", async () => {
  const pair = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, [
    "sign",
    "verify",
  ]);
  const der = new Uint8Array(await crypto.subtle.exportKey("pkcs8", pair.privateKey));
  const pem = `-----BEGIN PRIVATE KEY-----\n${Buffer.from(der).toString("base64")}\n-----END PRIVATE KEY-----\n`;
  const t = await jwt(pem, "AB12CD34EF", "ABCDE12345", 1791000000);
  const [h, c, s] = t.split(".");
  const dec = (x) => Buffer.from(x, "base64url");
  assert.deepEqual(JSON.parse(dec(h)), { alg: "ES256", kid: "AB12CD34EF" });
  assert.deepEqual(JSON.parse(dec(c)), { iss: "ABCDE12345", iat: 1791000000 });
  const ok = await crypto.subtle.verify(
    { name: "ECDSA", hash: "SHA-256" },
    pair.publicKey,
    dec(s),
    new TextEncoder().encode(`${h}.${c}`),
  );
  assert.ok(ok);
});

test("only POST /push, and bad pushes are refused before APNs", async () => {
  const env = {};
  let r = await relay.fetch(new Request("https://relay.example/x"), env);
  assert.equal(r.status, 404);
  r = await relay.fetch(
    new Request("https://relay.example/push", { method: "POST", body: JSON.stringify({ token: "x" }) }),
    env,
  );
  assert.equal(r.status, 400);
  const limited = {
    LIMITER: { limit: async () => ({ success: false }) },
  };
  r = await relay.fetch(
    new Request("https://relay.example/push", {
      method: "POST",
      body: JSON.stringify({ token, env: "sandbox", e: "AAAA" }),
    }),
    limited,
  );
  assert.equal(r.status, 429);
});
