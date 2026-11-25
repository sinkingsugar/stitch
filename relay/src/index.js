// Stitch's push relay (design §5, milestone 4): a Cloudflare Worker that
// holds the app team's APNs key and forwards pushes from the user's nodes to
// their phones. It keeps nothing and reads nothing: a push's content is
// encrypted on the node for the phone (only the phone's notification
// service extension decrypts it), and the relay shows only its own
// placeholder, so it cannot be used to put arbitrary text on a phone.
//
// POST /push  {"token": "<hex>", "env": "sandbox"|"production",
//              "e": "<base64 ciphertext>", "badge": 3,
//              "collapse": "<question ID>"}
//   → 200 {"ok": true} | 4xx {"error": "..."} | 502 with APNs's reason
// POST /answered  {"token", "env", "e"}: a question was answered; a
//   background push (nothing shown) whose sealed content says which, so
//   the app takes its notification away. Same answers.
// POST /voip  {"token": "<PushKit token>", "env", "e"}: a call (an agent's
//   `call_user`): a VoIP push to `<APNS_TOPIC>.voip`, the sealed call its
//   whole payload, which the app opens and reports to CallKit as an
//   incoming call. Kept by APNs only while it could be answered. Same
//   answers. A node falls back to a notification when a relay from
//   before calls answers 404.
//
// `collapse` becomes the notification's identifier on the phone
// (`apns-collapse-id`): an opaque record ID, so the app can find it.
//
// A device token is the credential, as for APNs itself: only the phone and
// the user's nodes know it. Pushes are capped per token (`LIMITER`, the
// Workers rate limiting binding, when bound).
//
// Secrets (`wrangler secret put`): APNS_KEY (the .p8, PEM), APNS_KEY_ID,
// APNS_TEAM_ID. Vars: APNS_TOPIC (the app's bundle ID).

/** The largest ciphertext forwarded (APNs takes 4 KiB of payload). */
export const MAX_CIPHERTEXT = 3000;

/** What the phone shows when its extension cannot decrypt a push. */
export const PLACEHOLDER = { title: "Stitch", body: "Something waits on you" };

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const answered = url.pathname === "/answered";
    const voip = url.pathname === "/voip";
    if (request.method !== "POST" || !(answered || voip || url.pathname === "/push")) {
      return json({ error: "POST /push, /answered or /voip" }, 404);
    }
    let push;
    try {
      push = parsePush(await request.json());
      if (answered) push = { token: push.token, env: push.env, e: push.e, answered: true };
      if (voip) push = { token: push.token, env: push.env, e: push.e, voip: true };
    } catch (e) {
      return json({ error: String(e.message ?? e) }, 400);
    }
    if (env.LIMITER) {
      const { success } = await env.LIMITER.limit({ key: push.token });
      if (!success) return json({ error: "too many pushes for this device" }, 429);
    }
    const token = await providerToken(env);
    const host = push.env === "sandbox" ? "api.sandbox.push.apple.com" : "api.push.apple.com";
    const sent = await fetch(`https://${host}/3/device/${push.token}`, {
      method: "POST",
      headers: headers(push, token, env.APNS_TOPIC ?? "com.fragcolor.stitch"),
      body: JSON.stringify(payload(push)),
    });
    if (sent.ok) return json({ ok: true });
    const reason = await sent.json().then((b) => b.reason).catch(() => "");
    return json({ error: `APNs said ${sent.status} ${reason}`.trim() }, 502);
  },
};

/** A push as the relay takes it: checked, nothing else kept. */
export function parsePush(body) {
  if (!body || typeof body !== "object") throw new Error("a JSON object");
  const { token, env, e, badge, collapse } = body;
  if (typeof token !== "string" || !/^[0-9a-f]{64,200}$/.test(token)) {
    throw new Error("token: the device token, lowercase hex");
  }
  if (env !== "sandbox" && env !== "production") {
    throw new Error("env: sandbox or production");
  }
  if (typeof e !== "string" || e.length === 0 || e.length > MAX_CIPHERTEXT || !/^[A-Za-z0-9+/=]+$/.test(e)) {
    throw new Error(`e: the ciphertext, base64, at most ${MAX_CIPHERTEXT} characters`);
  }
  if (badge !== undefined && !(Number.isInteger(badge) && badge >= 0 && badge < 10000)) {
    throw new Error("badge: a count");
  }
  if (collapse !== undefined && !(typeof collapse === "string" && /^[A-Za-z0-9_.:-]{1,64}$/.test(collapse))) {
    throw new Error("collapse: a question's ID, at most 64 characters");
  }
  const push = { token, env, e, badge };
  if (collapse !== undefined) push.collapse = collapse;
  return push;
}

/** Seconds APNs keeps a call it cannot deliver now: past them it could
 *  not be answered anyway. */
export const RING_EXPIRES = 30;

/** APNs's headers: an alert (named by `collapse`), an answer's
 *  background push, or a call's VoIP push (its own topic). */
export function headers(push, token, topic, now = Date.now()) {
  if (push.voip) {
    return {
      authorization: `bearer ${token}`,
      "apns-topic": `${topic}.voip`,
      "apns-push-type": "voip",
      "apns-priority": "10",
      "apns-expiration": String(Math.floor(now / 1000) + RING_EXPIRES),
      "content-type": "application/json",
    };
  }
  const h = {
    authorization: `bearer ${token}`,
    "apns-topic": topic,
    "apns-push-type": push.answered ? "background" : "alert",
    "apns-priority": push.answered ? "5" : "10",
    "content-type": "application/json",
  };
  if (!push.answered && push.collapse !== undefined) h["apns-collapse-id"] = push.collapse;
  return h;
}

/** What APNs gets: the relay's own placeholder, the ciphertext for the
 *  phone's extension (`mutable-content`), the badge; for an answer, only
 *  the ciphertext, for the app woken in the background. */
export function payload(push) {
  // PushKit hands the payload to the app as it is: nothing is shown.
  if (push.voip) return { e: push.e };
  if (push.answered) return { aps: { "content-available": 1 }, e: push.e };
  const aps = {
    alert: PLACEHOLDER,
    "mutable-content": 1,
    sound: "default",
    "thread-id": "questions",
  };
  if (push.badge !== undefined) aps.badge = push.badge;
  return { aps, e: push.e };
}

/** The JWT APNs takes (ES256, the key's ID and the team's), made again
 *  after 50 minutes; per isolate. */
let cached = null;

export async function providerToken(env, now = Date.now()) {
  if (cached && now - cached.made < 50 * 60 * 1000 && cached.kid === env.APNS_KEY_ID) {
    return cached.token;
  }
  const token = await jwt(env.APNS_KEY, env.APNS_KEY_ID, env.APNS_TEAM_ID, Math.floor(now / 1000));
  cached = { token, made: now, kid: env.APNS_KEY_ID };
  return token;
}

export async function jwt(pem, keyId, teamId, issued) {
  const der = base64ToBytes(pem.replace(/-----[^-]+-----/g, "").replace(/\s+/g, ""));
  const key = await crypto.subtle.importKey(
    "pkcs8",
    der,
    { name: "ECDSA", namedCurve: "P-256" },
    false,
    ["sign"],
  );
  const input = `${b64url(JSON.stringify({ alg: "ES256", kid: keyId }))}.${b64url(
    JSON.stringify({ iss: teamId, iat: issued }),
  )}`;
  // WebCrypto gives ECDSA signatures as r ‖ s, which is what JWS wants.
  const sig = await crypto.subtle.sign(
    { name: "ECDSA", hash: "SHA-256" },
    key,
    new TextEncoder().encode(input),
  );
  return `${input}.${b64url(new Uint8Array(sig))}`;
}

function b64url(data) {
  const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function base64ToBytes(b64) {
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}
