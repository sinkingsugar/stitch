// content.js's fill() against a stub DOM, in Node:
// `node apps/chrome/test/content.test.mjs` (crates/stitch-node/tests/extension.rs
// runs it). Exit status 0 when every case passes.

import { readFileSync } from "node:fs";
import vm from "node:vm";

let src = readFileSync(new URL("../content.js", import.meta.url), "utf8");
if (!/\}\)\(\);\s*$/.test(src)) throw new Error("content.js no longer ends with its IIFE");
src = src.replace(/\}\)\(\);\s*$/, "globalThis.__test = { ref, fill, pageText };\n})();");

// An element as far as fill() looks.
function el(tag, attrs = {}, extra = {}) {
  const e = {
    nodeType: 1, tagName: tag, isConnected: true, clicks: 0, type: undefined, autocomplete: "", name: "", id: "",
    isContentEditable: false, ownerDocument: null,
    getAttribute: k => (k in attrs ? attrs[k] : null),
    hasAttribute: k => k in attrs,
    focus() {},
    dispatchEvent() {},
    click() {
      this.clicks++;
      attrs["aria-checked"] = attrs["aria-checked"] === "true" ? "false" : "true";
    },
    ...extra,
  };
  return e;
}

function load() {
  class HTMLInputElement {}
  class HTMLTextAreaElement {}
  for (const C of [HTMLInputElement, HTMLTextAreaElement]) {
    let v = "";
    Object.defineProperty(C.prototype, "value", {
      get() { return v; },
      set(x) {
        if (!(this instanceof C)) throw new TypeError("Illegal invocation");
        v = x;
      },
    });
  }
  const noop = () => {};
  const doc = { readyState: "complete", addEventListener: noop, title: "t", querySelectorAll: () => [], body: { innerText: "" } };
  const ctx = {
    chrome: { runtime: { onMessage: { addListener: noop }, sendMessage: () => Promise.resolve(null) } },
    document: doc, location: { href: "https://x/" },
    addEventListener: noop, removeEventListener: noop,
    Event: class {}, HTMLInputElement, HTMLTextAreaElement,
    console, setTimeout, clearTimeout, setInterval: () => 0, clearInterval: noop, URL, Math, Date,
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(src, ctx, { filename: "content.js" });
  return { doc, ...ctx.__test, HTMLInputElement, HTMLTextAreaElement };
}

const cases = {
  "whole: true reads the body, not only main (overlays, dialogs)"() {
    const { doc, pageText } = load();
    const main = { innerText: "x".repeat(600) };
    doc.querySelectorAll = () => [main];
    doc.body.innerText = "x".repeat(600) + "\nCookie dialog Economy a partire da 50,31";
    if (pageText(5000).text.includes("Cookie")) throw new Error("the default read saw outside main");
    if (!pageText(5000, true).text.includes("Cookie dialog")) throw new Error("whole read missed the dialog");
  },

  "a role=checkbox div is toggled by a click"() {
    const { ref, fill } = load();
    const d = el("DIV", { role: "checkbox", "aria-checked": "false" });
    const r = ref(d);
    const out = fill({ ref: r, value: true });
    if (d.clicks !== 1 || out.value !== true) throw new Error(`clicks ${d.clicks}, ${JSON.stringify(out)}`);
    // Already as wanted: no click.
    fill({ ref: r, value: true });
    if (d.clicks !== 1) throw new Error("clicked again");
    fill({ ref: r, value: false });
    if (d.clicks !== 2) throw new Error("not turned off");
  },

  "a role=switch button is a toggle too"() {
    const { ref, fill } = load();
    const b = el("BUTTON", { role: "switch", "aria-checked": "true" });
    fill({ ref: ref(b), value: false });
    if (b.clicks !== 1) throw new Error("not clicked");
  },

  "a non-field element is refused clearly, not with Illegal invocation"() {
    const { ref, fill } = load();
    const p = el("SPAN", { role: "button" });
    try {
      fill({ ref: ref(p), value: "x" });
    } catch (e) {
      if (!/not a fillable field \(span role=button\): click it/.test(e.message)) throw e;
      return;
    }
    throw new Error("filled a span");
  },

  "a text input still gets its value"() {
    const { ref, fill, HTMLInputElement } = load();
    const i = el("INPUT", {}, { type: "text" });
    Object.setPrototypeOf(i, HTMLInputElement.prototype);
    const out = fill({ ref: ref(i), value: "hi" });
    if (out.value !== "hi") throw new Error(JSON.stringify(out));
  },
};

let failed = 0;
for (const [name, run] of Object.entries(cases)) {
  try {
    run();
    console.log(`ok   ${name}`);
  } catch (e) {
    failed++;
    console.log(`FAIL ${name}: ${e?.stack || e}`);
  }
}
process.exit(failed ? 1 : 0);
