// Stitch's extension: the script in every page (an isolated world). In a
// Stitch session's tab it:
// - builds the snapshot the agent reads, with refs to elements, and
//   resolves refs to viewport coordinates for CDP input (B0, §15.7: in the
//   page is 30–150 times faster than CDP's accessibility tree);
// - draws the overlay (glow, the agent's cursor and label, Stop, Take over /
//   Hand back), hidden while a screenshot is taken;
// - reports the user's input. CDP input is trusted too, so the agent marks
//   its own: `timestamp: 1` reaches the page as `event.timeStamp === 0`,
//   which real input never has (B0, §15.5).
// In other tabs it does nothing until the service worker says the tab is a
// session's.

(() => {
  if (globalThis.__stitch) return;
  globalThis.__stitch = true;

  let session = null; // set once the service worker says this tab is a session's
  let held = false;
  const DOC = Math.random().toString(36).slice(2, 10);

  // An element of this page or of a same-origin frame (another realm,
  // where `instanceof Element` is false).
  const isEl = n => n != null && n.nodeType === 1;

  // An element's shadow root, closed ones too (chrome.dom reaches those
  // from an extension's script). Never our own overlay's.
  function shadowOf(el) {
    if (el === host) return null;
    try {
      return chrome.dom?.openOrClosedShadowRoot?.(el) || el.shadowRoot || null;
    } catch {
      return el.shadowRoot || null;
    }
  }

  // Every element under a root, inside shadow roots too, in tree order.
  function deepElements(root, out = []) {
    for (const el of root.querySelectorAll("*")) {
      out.push(el);
      const sr = shadowOf(el);
      if (sr) deepElements(sr, out);
    }
    return out;
  }

  // Another extension's frame: Chrome refuses the debugger on a tab that
  // holds one (sw.js).
  const extensionOf = el => /^chrome-extension:\/\/([a-p]{32})/i.exec(String(el.src || ""))?.[1] || null;

  // --- refs ---------------------------------------------------------------
  let nextRef = 1;
  const refOf = new WeakMap(); // element -> "ref_N"
  const byRef = new Map(); // "ref_N" -> WeakRef(element)

  function ref(el) {
    let r = refOf.get(el);
    if (!r) {
      r = `ref_${nextRef++}`;
      refOf.set(el, r);
      byRef.set(r, new WeakRef(el));
    }
    return r;
  }

  function element(r, doc) {
    if (doc && doc !== DOC) throw new Error(`${r} is from an earlier page: take a new snapshot`);
    const el = byRef.get(r)?.deref();
    if (!el || !el.isConnected) throw new Error(`${r} is gone from the page: take a new snapshot`);
    return el;
  }

  // --- roles and names --------------------------------------------------------
  const INPUT_ROLES = {
    button: "button", submit: "button", reset: "button", image: "button", checkbox: "checkbox", radio: "radio",
    range: "slider", search: "searchbox", email: "textbox", tel: "textbox", url: "textbox", text: "textbox",
    password: "textbox", number: "spinbutton", date: "textbox", "datetime-local": "textbox", month: "textbox",
    time: "textbox", week: "textbox", color: "button", file: "button",
  };
  const INTERACTIVE = new Set(["button", "link", "textbox", "searchbox", "checkbox", "radio", "combobox", "listbox",
    "option", "menuitem", "menuitemcheckbox", "menuitemradio", "tab", "switch", "slider", "spinbutton", "treeitem",
    "clickable"]);
  const LANDMARKS = { NAV: "navigation", MAIN: "main", HEADER: "banner", FOOTER: "contentinfo", ASIDE: "complementary",
    FORM: "form", DIALOG: "dialog", TABLE: "table" };

  function roleOf(el) {
    const explicit = el.getAttribute("role");
    if (explicit) return explicit.split(" ")[0];
    const tag = el.tagName;
    switch (tag) {
      case "A": return el.hasAttribute("href") ? "link" : null;
      case "BUTTON": case "SUMMARY": return "button";
      case "INPUT": return el.type === "hidden" ? null : INPUT_ROLES[el.type] || "textbox";
      case "TEXTAREA": return "textbox";
      case "SELECT": return el.multiple ? "listbox" : "combobox";
      case "OPTION": return null; // shown with its select
      case "H1": case "H2": case "H3": case "H4": case "H5": case "H6": return "heading";
      case "IMG": return el.alt ? "img" : null;
      case "IFRAME": return "iframe";
    }
    if (LANDMARKS[tag]) return LANDMARKS[tag];
    if (el.isContentEditable && el.getAttribute("contenteditable") !== null) return "textbox";
    if (el.hasAttribute("onclick") || (el.tabIndex >= 0 && el.hasAttribute("tabindex"))) {
      return getComputedStyle(el).cursor === "pointer" ? "clickable" : null;
    }
    return null;
  }

  function clean(s, n = 100) {
    s = (s || "").replace(/\s+/g, " ").trim();
    return s.length > n ? s.slice(0, n - 1) + "…" : s;
  }

  function nameOf(el, role) {
    const aria = el.getAttribute("aria-label");
    if (aria) return clean(aria);
    const by = el.getAttribute("aria-labelledby");
    if (by) {
      const t = by.split(/\s+/).map(id => document.getElementById(id)?.innerText || "").join(" ");
      if (t.trim()) return clean(t);
    }
    if (el.labels?.length) return clean([...el.labels].map(l => l.innerText).join(" "));
    if (el.tagName === "IMG") return clean(el.alt);
    if (el.tagName === "INPUT" && /^(button|submit|reset)$/.test(el.type)) return clean(el.value);
    if (role === "textbox" || role === "searchbox" || role === "combobox" || role === "spinbutton") {
      return clean(el.placeholder || el.title || el.getAttribute("aria-placeholder") || "");
    }
    return clean(el.innerText || el.textContent || el.title || "", role === "heading" ? 150 : 100);
  }

  function hidden(el) {
    if (el.hidden || el.getAttribute("aria-hidden") === "true") return true;
    const st = (el.ownerDocument.defaultView || window).getComputedStyle(el);
    if (st.display === "none" || st.visibility === "hidden" || st.visibility === "collapse") return true;
    return false;
  }

  function sensitive(el) {
    return el.type === "password" || /^(cc-|one-time-code)/.test(el.autocomplete || "") ||
      /(password|passcode|otp|cvv|cvc|card.?number)/i.test(el.name || el.id || "");
  }

  function describe(el, role, name) {
    let line = `${role}${name ? ` "${name.replace(/"/g, "'")}"` : ""}`;
    if (role === "heading") line = `heading[${el.tagName[1] || el.getAttribute("aria-level") || ""}]` + line.slice(7);
    const v = [];
    if ("value" in el && /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) && el.type !== "checkbox" && el.type !== "radio") {
      if (el.tagName === "SELECT") {
        v.push(`value="${clean(el.selectedOptions[0]?.text || "", 60)}"`);
        const opts = [...el.options].slice(0, 12).map(o => clean(o.text, 30));
        v.push(`options=[${opts.map(o => `"${o}"`).join(", ")}${el.options.length > 12 ? `, …${el.options.length - 12} more` : ""}]`);
      } else if (el.value) {
        v.push(sensitive(el) ? 'value="••••"' : `value="${clean(el.value, 80)}"`);
      }
    } else if (el.isContentEditable && el.innerText.trim()) {
      v.push(`value="${clean(el.innerText, 80)}"`);
    }
    if (el.checked || el.getAttribute("aria-checked") === "true") v.push("checked");
    if (el.disabled || el.getAttribute("aria-disabled") === "true") v.push("disabled");
    const exp = el.getAttribute("aria-expanded");
    if (exp) v.push(exp === "true" ? "expanded" : "collapsed");
    if (el.getAttribute("aria-selected") === "true") v.push("selected");
    if (sensitive(el)) v.push("sensitive");
    if (el.required) v.push("required");
    if (role === "link") {
      const href = el.getAttribute("href") || "";
      if (href && !href.startsWith("javascript:")) v.push(`href="${clean(href, 80)}"`);
    }
    return line + (v.length ? " " + v.join(" ") : "");
  }

  // Same-origin iframes are read as part of the page: an element's offset
  // from the top viewport is its frames' offsets added up.
  function frameOffset(el) {
    let x = 0, y = 0;
    let win = el.ownerDocument?.defaultView;
    while (win && win !== window) {
      const fe = win.frameElement;
      if (!fe) break;
      const r = fe.getBoundingClientRect();
      x += r.left + fe.clientLeft;
      y += r.top + fe.clientTop;
      win = fe.ownerDocument.defaultView;
    }
    return { x, y };
  }

  function topRect(el) {
    const r = el.getBoundingClientRect();
    const o = frameOffset(el);
    return { left: r.left + o.x, top: r.top + o.y, right: r.right + o.x, bottom: r.bottom + o.y, width: r.width, height: r.height };
  }

  // A same-origin iframe's document; none for others.
  function frameDoc(el) {
    try {
      return el.contentDocument?.body ? el.contentDocument : null;
    } catch {
      return null;
    }
  }

  function inViewport(el) {
    const r = topRect(el);
    return r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth && r.width > 0 && r.height > 0;
  }

  // The tree: landmarks and headings for structure, interactive elements
  // with refs, and (unless interactive only) short text blocks.
  function snapshot({ interactive = false, root = null, max_chars = 32000, viewport = false } = {}) {
    // The whole document, not only its body: other extensions put their
    // elements next to it (on <html>).
    const start = root ? element(root.ref, root.doc) : document.documentElement;
    const lines = [];
    let chars = 0;
    let cut = false;
    const TEXT = new Set(["P", "LI", "TD", "TH", "DT", "DD", "LABEL", "BLOCKQUOTE", "FIGCAPTION", "PRE", "SPAN", "DIV"]);
    function push(depth, line) {
      if (cut) return;
      const s = "  ".repeat(depth) + line;
      if (chars + s.length + 1 > max_chars) {
        cut = true;
        return;
      }
      chars += s.length + 1;
      lines.push(s);
    }
    function walk(el, depth) {
      if (cut || !isEl(el)) return;
      if (el.tagName === "SCRIPT" || el.tagName === "STYLE" || el.tagName === "NOSCRIPT" || el.tagName === "TEMPLATE") return;
      if (el.localName === "stitch-overlay") return;
      if (hidden(el)) return;
      const role = roleOf(el);
      let d = depth;
      if (role && (!viewport || inViewport(el) || LANDMARKS[el.tagName])) {
        const name = nameOf(el, role);
        if (role === "iframe") {
          const ext = extensionOf(el);
          if (ext && ext !== chrome.runtime.id) {
            push(depth, `iframe src="${clean(el.src, 80)}" (another extension's frame, extension ${ext}: while it is in the page Chrome may refuse Stitch's input and screenshots) [${ref(el)}]`);
            return;
          }
          const inner = frameDoc(el);
          push(depth, `iframe${name ? ` "${name.replace(/"/g, "'")}"` : ""}${inner ? "" : ` src="${clean(el.src, 80)}" (another site's frame: not read; screenshot it)`} [${ref(el)}]`);
          if (inner) walk(inner.body, depth + 1);
          return;
        }
        if (INTERACTIVE.has(role)) {
          push(depth, `${describe(el, role, name)} [${ref(el)}]${viewport || inViewport(el) ? "" : " (offscreen)"}`);
          // Inside a link or button: its text is its name already.
          if (role !== "listbox" && role !== "combobox") return;
        } else if (role === "heading" || role === "img") {
          if (!interactive || role === "heading") push(depth, `${describe(el, role, name)} [${ref(el)}]`);
          return;
        } else if (!interactive || LANDMARKS[el.tagName] || el.getAttribute("role")) {
          push(depth, `${role}${el.getAttribute("aria-label") ? ` "${clean(el.getAttribute("aria-label"))}"` : ""}`);
          d = depth + 1;
        }
      } else if (!interactive && TEXT.has(el.tagName)) {
        // Text directly in this element (not in children that are listed).
        let own = "";
        for (const n of el.childNodes) if (n.nodeType === 3) own += n.textContent;
        own = clean(own, 200);
        if (own.length > 1 && (!viewport || inViewport(el))) push(depth, `text "${own.replace(/"/g, "'")}"`);
      }
      const sr = shadowOf(el);
      if (sr) for (const c of sr.children) walk(c, d);
      for (const c of el.children) walk(c, d);
    }
    walk(start, 0);
    return {
      doc: DOC, url: location.href, title: document.title, text: lines.join("\n"), truncated: cut,
      scroll: { x: Math.round(scrollX), y: Math.round(scrollY), height: document.documentElement.scrollHeight, viewport: innerHeight },
      focused: focusedLine(),
    };
  }

  function focusedLine() {
    const el = document.activeElement;
    if (!el || el === document.body || el === document.documentElement || el === host) return null;
    return describeShort(el);
  }

  function describeShort(el) {
    const role = roleOf(el) || el.tagName.toLowerCase();
    const r = refOf.get(el);
    return `${role} "${nameOf(el, role)}"${r ? ` [${r}]` : ""}`;
  }

  function pageText(max_chars = 32000, whole = false) {
    // The main part with the most text: the first match can be empty or
    // hidden (a store's page: a card's <article> before the product). The
    // whole body when none has much.
    const textOf = el => (el?.innerText || "").replace(/\n{3,}/g, "\n\n").trim();
    let t = "";
    // `whole`: a wait looks everywhere (dialogs and overlays sit outside main).
    if (whole) {
      t = textOf(document.body);
      return { doc: DOC, url: location.href, title: document.title, text: t.slice(0, max_chars), truncated: t.length > max_chars };
    }
    for (const el of document.querySelectorAll("main, [role=main], article")) {
      const s = textOf(el);
      if (s.length > t.length) t = s;
    }
    if (t.length < 500) {
      const all = textOf(document.body);
      if (all.length > t.length) t = all;
    }
    return { doc: DOC, url: location.href, title: document.title, text: t.slice(0, max_chars), truncated: t.length > max_chars };
  }

  function find({ text, role, max = 20 }) {
    const want = (text || "").toLowerCase();
    const out = [];
    const docs = [document];
    for (let i = 0; i < docs.length; i++) {
      for (const f of deepElements(docs[i]).filter(e => e.tagName === "IFRAME")) {
        const d = frameDoc(f);
        if (d) docs.push(d);
      }
    }
    const all = docs.flatMap(d => deepElements(d));
    for (const el of all) {
      if (out.length >= max) break;
      const r = roleOf(el);
      if (!r || el.localName === "stitch-overlay") continue;
      if (role && r !== role) continue;
      const name = nameOf(el, r);
      const hay = `${name} ${el.getAttribute("placeholder") || ""} ${el.getAttribute("title") || ""} ${el.id || ""} ${el.name || ""}`.toLowerCase();
      if (want && !hay.includes(want)) continue;
      if (hidden(el)) continue;
      out.push(`${describe(el, r, name)} [${ref(el)}]${inViewport(el) ? "" : " (offscreen)"}`);
    }
    return { doc: DOC, url: location.href, matches: out };
  }

  // A ref's point for CDP input, in CSS pixels of the viewport, after
  // scrolling it into view. Says what covers it, if anything does.
  function resolve({ ref: r, doc, scroll = true }) {
    const el = element(r, doc);
    if (scroll && !inViewport(el)) el.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
    const rects = [...el.getClientRects()].filter(x => x.width > 0 && x.height > 0);
    const local = rects[0] || el.getBoundingClientRect();
    if (!local.width || !local.height) {
      const href = el.closest && el.closest("a[href]")?.href;
      const usable = href && !/^(javascript:|#)/i.test(el.closest("a[href]").getAttribute("href").trim()) && href.split("#")[0] !== location.href.split("#")[0];
      throw new Error(
        `${r} is hidden (no size on the page), likely inside a closed menu: open its menu first (click or hover the parent control), then look again.` +
          (usable ? ` It is a link: browser_navigate to ${href} also works.` : "")
      );
    }
    const o = frameOffset(el);
    const rect = { left: local.left + o.x, top: local.top + o.y, width: local.width, height: local.height };
    const x = Math.min(Math.max(rect.left + rect.width / 2, 1), innerWidth - 1);
    const y = Math.min(Math.max(rect.top + rect.height / 2, 1), innerHeight - 1);
    // What is there, asked of the element's own document (its frame).
    // In a shadow root, asked of the root: the document would answer its host.
    const rootNode = el.getRootNode();
    const at = (rootNode.elementFromPoint ? rootNode : el.ownerDocument).elementFromPoint(x - o.x, y - o.y);
    const covered = at && at !== el && !el.contains(at) && !at.contains(el) ? describeShort(at) : null;
    return {
      x: Math.round(x), y: Math.round(y), rect: { x: Math.round(rect.left), y: Math.round(rect.top), w: Math.round(rect.width), h: Math.round(rect.height) },
      covered, sensitive: sensitive(el), what: describeShort(el), tag: el.tagName.toLowerCase(), type: el.type || null,
      form: el.form ? { action: el.form.action, method: el.form.method } : null, href: el.closest("a")?.href || null,
      text: clean(el.innerText || el.value || "", 80), autocomplete: el.autocomplete || null,
    };
  }

  // Sets a field's value the way frameworks notice (the native setter, then
  // input and change).
  function fill({ ref: r, doc, value }) {
    const el = element(r, doc);
    if (sensitive(el)) throw new Error(`${r} is a password, code or card field: ask the user to fill it (browser_handoff)`);
    el.focus();
    if (el.tagName === "SELECT") {
      const opt = [...el.options].find(o => o.value === value || o.text.trim() === value);
      if (!opt) throw new Error(`no option "${value}" in ${r}`);
      el.value = opt.value;
    } else if (el.type === "checkbox" || el.type === "radio") {
      const want = value === true || value === "true" || value === "on" || value === "checked";
      if (el.checked !== want) el.click();
      return { value: el.checked };
    } else if (/^(checkbox|switch|radio)$/.test(el.getAttribute("role") || "")) {
      // An ARIA toggle (a div, a button, a custom element): clicked when its
      // state differs; it has no value to set.
      const want = value === true || value === "true" || value === "on" || value === "checked";
      if ((el.getAttribute("aria-checked") === "true") !== want) el.click();
      return { value: el.getAttribute("aria-checked") === "true" };
    } else if (el.isContentEditable) {
      el.textContent = String(value);
    } else if (el.tagName === "TEXTAREA" || el.tagName === "INPUT") {
      const win = el.ownerDocument?.defaultView || window; // the element's own realm (a frame's)
      const proto = el.tagName === "TEXTAREA" ? win.HTMLTextAreaElement.prototype : win.HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(proto, "value").set.call(el, String(value));
    } else {
      const role = el.getAttribute("role");
      throw new Error(`${r} is not a fillable field (${el.tagName.toLowerCase()}${role ? ` role=${role}` : ""}): click it`);
    }
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { value: sensitive(el) ? "••••" : clean(el.value ?? el.textContent, 80) };
  }

  // --- input without the debugger --------------------------------------------------
  // When Chrome refuses the debugger on this tab (another extension's frame
  // in it, sw.js), the CDP Input.* commands the node sends are played here
  // as DOM events. They are untrusted (`isTrusted` false): the user-input
  // watcher below ignores them, so they never move the tab's epoch, and
  // some pages ignore them too. Sensitive fields are never typed into.

  // The element at a point of the top viewport, through same-origin frames
  // and shadow roots (closed ones too), with the point in its document.
  function deepAt(x, y) {
    let doc = document, ox = 0, oy = 0;
    for (;;) {
      let at = doc.elementFromPoint(x - ox, y - oy);
      if (at === host) return { el: document.documentElement, x, y };
      for (let sr = at && shadowOf(at); sr; sr = shadowOf(at)) {
        const inner = sr.elementFromPoint(x - ox, y - oy);
        if (!inner || inner === at) break;
        at = inner;
      }
      if (!at) return { el: doc.documentElement, x: x - ox, y: y - oy };
      if (at.tagName === "IFRAME" || at.tagName === "FRAME") {
        const inner = frameDoc(at);
        if (!inner) throw new Error(`a frame of another site is at (${Math.round(x)}, ${Math.round(y)}): it cannot be reached without the debugger`);
        const r = at.getBoundingClientRect();
        ox += r.left + at.clientLeft;
        oy += r.top + at.clientTop;
        doc = inner;
        continue;
      }
      return { el: at, x: x - ox, y: y - oy };
    }
  }

  // The focused element, through shadow roots and same-origin frames.
  function deepActive() {
    let el = document.activeElement;
    for (;;) {
      const sr = el && shadowOf(el);
      if (sr?.activeElement) {
        el = sr.activeElement;
        continue;
      }
      if (el && (el.tagName === "IFRAME" || el.tagName === "FRAME") && frameDoc(el)?.activeElement) {
        el = frameDoc(el).activeElement;
        continue;
      }
      return el || document.body;
    }
  }

  const BUTTONS = { left: 0, middle: 1, right: 2, back: 3, forward: 4 };
  const BUTTON_BITS = { left: 1, right: 2, middle: 4 };
  const FOCUSABLE = "input:not([disabled]), textarea:not([disabled]), select:not([disabled]), button:not([disabled]), a[href], area[href], summary, [tabindex], [contenteditable]:not([contenteditable=false]), iframe";
  let hovered = null;
  let pressed = null; // { el, button }

  function mods(m = 0) {
    return { altKey: !!(m & 1), ctrlKey: !!(m & 2), metaKey: !!(m & 4), shiftKey: !!(m & 8) };
  }

  function mouseInit(p, at, extra = {}) {
    const win = at.el.ownerDocument.defaultView || window;
    return { bubbles: true, cancelable: true, composed: true, view: win, clientX: at.x, clientY: at.y,
      screenX: at.x + (win.screenX || 0), screenY: at.y + (win.screenY || 0), button: BUTTONS[p.button] ?? 0,
      buttons: p.buttons ?? 0, detail: p.clickCount || 0, ...mods(p.modifiers), ...extra };
  }

  function fire(el, type, init, Kind = MouseEvent) {
    return el.dispatchEvent(new Kind(type, init));
  }

  function synthMouse(p) {
    const at = deepAt(p.x, p.y);
    const el = at.el;
    const ptr = { pointerId: 1, pointerType: "mouse", isPrimary: true };
    switch (p.type) {
      case "mouseMoved": {
        if (hovered !== el) {
          if (hovered?.isConnected) {
            fire(hovered, "pointerout", mouseInit(p, at, { ...ptr, relatedTarget: el }), PointerEvent);
            fire(hovered, "mouseout", mouseInit(p, at, { relatedTarget: el }));
          }
          fire(el, "pointerover", mouseInit(p, at, { ...ptr, relatedTarget: hovered }), PointerEvent);
          fire(el, "mouseover", mouseInit(p, at, { relatedTarget: hovered }));
          hovered = el;
        }
        fire(el, "pointermove", mouseInit(p, at, ptr), PointerEvent);
        fire(el, "mousemove", mouseInit(p, at));
        return { target: describeShort(el) };
      }
      case "mousePressed": {
        const bits = { buttons: BUTTON_BITS[p.button] || 1 };
        const down = fire(el, "pointerdown", mouseInit({ ...p, ...bits }, at, ptr), PointerEvent) &
          fire(el, "mousedown", mouseInit({ ...p, ...bits }, at));
        pressed = { el, button: p.button || "left" };
        // What a real press does by default: focus what it lands on.
        if (down && (p.button || "left") === "left") {
          const f = el.closest(FOCUSABLE);
          if (f) f.focus({ preventScroll: true });
          else deepActive()?.blur?.();
        }
        return { target: describeShort(el) };
      }
      case "mouseReleased": {
        fire(el, "pointerup", mouseInit(p, at, ptr), PointerEvent);
        fire(el, "mouseup", mouseInit(p, at));
        const from = pressed;
        pressed = null;
        if (!from) return { target: describeShort(el) };
        // A click goes to what both the press and the release were on.
        let target = el;
        while (target && !(target === from.el || target.contains(from.el))) target = target.parentElement || target.getRootNode()?.host;
        target ||= el;
        const button = p.button || "left";
        if (button === "left") {
          fire(target, "click", mouseInit(p, at));
          if ((p.clickCount || 1) === 2) fire(target, "dblclick", mouseInit(p, at));
        } else if (button === "right") {
          fire(target, "contextmenu", mouseInit(p, at));
        } else {
          fire(target, "auxclick", mouseInit(p, at));
        }
        return { target: describeShort(target) };
      }
      case "mouseWheel": {
        const dx = p.deltaX || 0, dy = p.deltaY || 0;
        if (fire(el, "wheel", { ...mouseInit(p, at), deltaX: dx, deltaY: dy, deltaMode: 0 }, WheelEvent)) {
          // The nearest box that can scroll that way, else the frame's window.
          let box = el;
          const can = b => {
            const st = getComputedStyle(b);
            return (dy && /(auto|scroll|overlay)/.test(st.overflowY) && b.scrollHeight > b.clientHeight) ||
              (dx && /(auto|scroll|overlay)/.test(st.overflowX) && b.scrollWidth > b.clientWidth);
          };
          while (box && box !== box.ownerDocument.documentElement && !can(box)) box = box.parentElement || box.getRootNode()?.host;
          if (box && box !== box.ownerDocument?.documentElement) box.scrollBy({ left: dx, top: dy, behavior: "instant" });
          else (el.ownerDocument.defaultView || window).scrollBy({ left: dx, top: dy, behavior: "instant" });
        }
        return { target: describeShort(el) };
      }
    }
    throw new Error(`unknown mouse event ${p.type}`);
  }

  function editable(el) {
    if (!el) return false;
    if (el.tagName === "TEXTAREA") return !el.readOnly && !el.disabled;
    if (el.tagName === "INPUT") return !el.readOnly && !el.disabled && !/^(button|submit|reset|checkbox|radio|file|image|color|range|hidden)$/.test(el.type);
    return el.isContentEditable;
  }

  // Text into the focused field, as typing would: execCommand, which the
  // page sees as input events, else the native setter with input/change.
  function insertText(text) {
    const el = deepActive();
    if (sensitive(el)) throw new Error("the focused field is a password, code or card field: ask the user to fill it (browser_handoff)");
    if (!editable(el)) throw new Error(`nothing editable is focused (${describeShort(el)}): click the field first`);
    if (el.ownerDocument.execCommand("insertText", false, text)) return;
    if (el.isContentEditable) {
      el.textContent += text;
    } else {
      const proto = el.tagName === "TEXTAREA" ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      let v = el.value;
      try {
        const s = el.selectionStart ?? v.length, e = el.selectionEnd ?? v.length;
        v = v.slice(0, s) + text + v.slice(e);
      } catch {
        v += text; // types without a selection (email, number)
      }
      Object.getOwnPropertyDescriptor(proto, "value").set.call(el, v);
    }
    el.dispatchEvent(new InputEvent("input", { bubbles: true, composed: true, inputType: "insertText", data: text }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  }

  function focusNext(from, back) {
    const all = deepElements(document).filter(e => e.matches(FOCUSABLE) && e.tabIndex >= 0 && !e.disabled && !hidden(e) && e.getClientRects().length);
    const i = all.indexOf(from);
    const next = all[(i < 0 ? (back ? all.length : -1) : i) + (back ? -1 : 1)];
    if (next) next.focus();
    else from?.blur?.();
  }

  function synthKey(p) {
    const el = deepActive();
    const m = p.modifiers || 0;
    const init = { bubbles: true, cancelable: true, composed: true, key: p.key || p.text || "", code: p.code || "",
      keyCode: p.windowsVirtualKeyCode || 0, which: p.windowsVirtualKeyCode || 0, ...mods(m) };
    if (p.type === "keyUp") {
      el.dispatchEvent(new KeyboardEvent("keyup", init));
      return { target: describeShort(el) };
    }
    if (p.text && sensitive(el)) throw new Error("the focused field is a password, code or card field: ask the user to fill it (browser_handoff)");
    if (!el.dispatchEvent(new KeyboardEvent("keydown", init))) return { target: describeShort(el), prevented: true };
    // What the key does by default, for the keys the agent uses.
    const key = init.key;
    if (p.text && p.type !== "rawKeyDown") {
      if (!el.dispatchEvent(new KeyboardEvent("keypress", { ...init, charCode: p.text.charCodeAt(0) }))) return { target: describeShort(el) };
      if (p.text === "\r") {
        if (el.tagName === "TEXTAREA" || el.isContentEditable) insertText("\n");
        else if (el.tagName === "INPUT" && el.form && editable(el)) el.form.requestSubmit();
        else if (/^(A|BUTTON|SUMMARY)$/.test(el.tagName) || (el.tagName === "INPUT" && /^(button|submit|reset|image)$/.test(el.type))) el.click();
      } else if (p.text === " " && (/^(BUTTON|SUMMARY)$/.test(el.tagName) || (el.tagName === "INPUT" && /^(button|submit|reset|checkbox|radio)$/.test(el.type)))) {
        el.click();
      } else if (editable(el)) {
        insertText(p.text);
      }
      return { target: describeShort(el) };
    }
    const doc = el.ownerDocument;
    if ((m & 6) && /^a$/i.test(key)) {
      if (el.select && editable(el)) el.select();
      else doc.execCommand("selectAll");
    } else if (key === "Backspace" && editable(el)) {
      if (sensitive(el)) throw new Error("the focused field is a password, code or card field: ask the user to fill it (browser_handoff)");
      doc.execCommand("delete");
    } else if (key === "Delete" && editable(el)) {
      if (sensitive(el)) throw new Error("the focused field is a password, code or card field: ask the user to fill it (browser_handoff)");
      doc.execCommand("forwardDelete");
    } else if (key === "Tab") {
      focusNext(el, !!(m & 8));
    } else if (el.tagName === "SELECT" && /^Arrow(Up|Down)$/.test(key)) {
      const i = Math.min(Math.max(el.selectedIndex + (key === "ArrowDown" ? 1 : -1), 0), el.options.length - 1);
      if (i !== el.selectedIndex) {
        el.selectedIndex = i;
        el.dispatchEvent(new Event("input", { bubbles: true }));
        el.dispatchEvent(new Event("change", { bubbles: true }));
      }
    } else if (/^(PageUp|PageDown|Home|End)$/.test(key) && !editable(el)) {
      const win = doc.defaultView || window;
      const h = win.innerHeight * 0.85;
      if (key === "PageUp") win.scrollBy(0, -h);
      else if (key === "PageDown") win.scrollBy(0, h);
      else win.scrollTo(0, key === "Home" ? 0 : doc.documentElement.scrollHeight);
    }
    return { target: describeShort(el) };
  }

  function synth({ method, params = {} }) {
    if (method === "Input.dispatchMouseEvent") return synthMouse(params);
    if (method === "Input.dispatchKeyEvent") return synthKey(params);
    if (method === "Input.insertText") {
      insertText(params.text || "");
      return {};
    }
    throw new Error(`${method} cannot be done without the debugger`);
  }

  // --- the overlay -------------------------------------------------------------
  let host = null;
  let ui = null;

  function ensureOverlay() {
    if (host) return;
    host = document.createElement("stitch-overlay");
    host.style.cssText = "all: initial; position: fixed; inset: 0; pointer-events: none; z-index: 2147483647;";
    const root = host.attachShadow({ mode: "closed" });
    root.innerHTML = `
      <style>
        .glow { position: fixed; inset: 0; pointer-events: none;
                box-shadow: inset 0 0 0 3px rgba(150, 90, 255, .85), inset 0 0 22px rgba(150, 90, 255, .45); }
        .held .glow { box-shadow: inset 0 0 0 3px rgba(240, 170, 40, .9); }
        .stopped .glow { display: none; }
        .cursor { position: fixed; width: 16px; height: 16px; margin: -8px 0 0 -8px; border-radius: 50%; left: -50px; top: -50px;
                  background: rgba(150, 90, 255, .9); box-shadow: 0 0 0 3px rgba(255,255,255,.8); pointer-events: none;
                  transition: left .2s ease-out, top .2s ease-out; }
        .label { position: fixed; pointer-events: none; font: 12px system-ui, sans-serif; color: #fff; left: -500px; top: -500px;
                 background: rgba(80, 40, 160, .92); padding: 2px 7px; border-radius: 4px; white-space: nowrap; }
        .pill { position: fixed; bottom: 14px; left: 50%; transform: translateX(-50%); pointer-events: auto;
                display: flex; gap: 8px; align-items: center; font: 13px system-ui, sans-serif; color: #fff;
                background: #2b1a55; padding: 6px 8px 6px 12px; border-radius: 18px; box-shadow: 0 2px 10px rgba(0,0,0,.3); }
        .held .pill { background: #6b4a10; }
        button { font: inherit; border: 0; border-radius: 12px; padding: 3px 10px; cursor: pointer; }
        .stop { background: #e5484d; color: #fff; }
        .take { background: #fff; color: #2b1a55; }
        .stopped .take, .stopped .stop { display: none; }
        .hidden { display: none !important; }
        .asks { position: fixed; top: 10px; left: 50%; transform: translateX(-50%); display: flex; flex-direction: column; gap: 6px; pointer-events: auto; }
        .ask { font: 13px system-ui, sans-serif; color: #fff; background: #2b1a55; border: 2px solid #f0aa28; border-radius: 10px;
               padding: 8px 10px; max-width: 560px; box-shadow: 0 4px 16px rgba(0,0,0,.35); }
        .ask .why { color: #cfc4ee; font-size: 12px; margin-top: 2px; }
        .ask .row { display: flex; gap: 6px; justify-content: flex-end; margin-top: 6px; }
        .ask .allow { background: #4cd17a; color: #10301a; }
        .ask .always { background: #fff; color: #2b1a55; }
        .ask .deny { background: #e5484d; color: #fff; }
        .target { position: fixed; border: 3px solid #e51e28; border-radius: 4px; pointer-events: none; }
      </style>
      <div class="wrap"><div class="glow"></div><div class="cursor"></div><div class="label"></div>
      <div class="pill"><span class="state">Stitch</span><button class="take">Take over</button><button class="stop">Stop</button></div>
      <div class="asks"></div></div>`;
    const wrap = root.querySelector(".wrap");
    ui = { wrap, asks: root.querySelector(".asks"), cursor: root.querySelector(".cursor"), label: root.querySelector(".label"),
      state: root.querySelector(".state"), take: root.querySelector(".take") };
    root.querySelector(".stop").addEventListener("click", e => {
      e.stopPropagation();
      chrome.runtime.sendMessage({ t: "stop" });
      setState({ stopped: true, state: "Stopped" });
    });
    ui.take.addEventListener("click", e => {
      e.stopPropagation();
      held = !held;
      chrome.runtime.sendMessage({ t: held ? "takeover" : "handback" });
      setState({ held, state: held ? "You have this tab" : "Handed back to Stitch" });
    });
    (document.documentElement || document).appendChild(host);
  }

  function setState(m) {
    if (m.off) {
      host?.remove();
      host = null;
      session = null;
      return;
    }
    ensureOverlay();
    if (m.hide != null) host.style.display = m.hide ? "none" : "";
    if (m.held != null) {
      held = m.held;
      ui.wrap.classList.toggle("held", held);
      ui.take.textContent = held ? "Hand back" : "Take over";
    }
    if (m.stopped != null) ui.wrap.classList.toggle("stopped", !!m.stopped);
    if (m.state != null) ui.state.textContent = m.state;
    if (m.x != null) {
      ui.cursor.style.left = `${m.x}px`;
      ui.cursor.style.top = `${m.y}px`;
      ui.label.style.left = `${m.x + 14}px`;
      ui.label.style.top = `${m.y + 10}px`;
    }
    if (m.label != null) {
      ui.label.textContent = m.label;
      ui.label.classList.toggle("hidden", !m.label);
    }
  }

  // A question asked while the user watches (§8.4): answered here or on
  // any client, the first answer wins. The target is outlined.
  const asked = new Map(); // id -> [box, outline]
  function ask(m) {
    ensureOverlay();
    if (m.done) {
      asked.get(m.id)?.forEach(e => e?.remove());
      asked.delete(m.id);
      return;
    }
    const box = document.createElement("div");
    box.className = "ask";
    const text = document.createElement("div");
    text.textContent = m.text;
    const why = document.createElement("div");
    why.className = "why";
    why.textContent = m.why || "";
    const row = document.createElement("div");
    row.className = "row";
    const answer = choice => e => {
      e.stopPropagation();
      chrome.runtime.sendMessage({ t: "answer", id: m.id, choice });
      ask({ id: m.id, done: true });
    };
    const b = (cls, label, choice) => {
      const x = document.createElement("button");
      x.className = cls;
      x.textContent = label;
      x.addEventListener("click", answer(choice));
      return x;
    };
    row.append(b("allow", "Allow once", "allow"));
    if (m.always) row.append(b("always", "Always on this site", "always"));
    row.append(b("deny", "Deny", "deny"));
    box.append(text, why, row);
    ui.asks.append(box);
    let frame = null;
    if (m.rect) {
      frame = document.createElement("div");
      frame.className = "target";
      Object.assign(frame.style, { left: `${m.rect.x - 4}px`, top: `${m.rect.y - 4}px`, width: `${m.rect.w + 2}px`, height: `${m.rect.h + 2}px` });
      ui.wrap.append(frame);
    }
    asked.set(m.id, [box, frame]);
  }

  // --- pointing ---------------------------------------------------------------------
  // The element last right-clicked ("Ask Stitch about this…"), and the
  // picker: hover to outline, click to choose, Escape to cancel.
  let lastContext = null;
  addEventListener("contextmenu", e => { lastContext = isEl(e.composedPath()[0]) ? e.composedPath()[0] : e.target; }, true);

  function pointed(el) {
    if (!isEl(el)) return null;
    // The nearest element a snapshot would list.
    let at = el;
    while (at && at !== document.body && !roleOf(at)) at = at.parentElement;
    if (!at || at === document.body) at = el;
    return { element: describeShort(at), ref: ref(at), doc: DOC };
  }

  function pick() {
    ensureOverlay();
    const box = document.createElement("div");
    box.className = "target";
    ui.wrap.append(box);
    setState({ state: "Click what to ask Stitch about (Esc cancels)" });
    const move = e => {
      const r = e.target.getBoundingClientRect?.();
      if (!r) return;
      Object.assign(box.style, { left: `${r.left - 2}px`, top: `${r.top - 2}px`, width: `${r.width}px`, height: `${r.height}px` });
    };
    let end = () => {
      removeEventListener("mousemove", move, true);
      removeEventListener("click", click, true);
      removeEventListener("keydown", key, true);
      box.remove();
      setState({ state: session ? "Stitch" : "" });
      if (!session) setState({ off: true });
    };
    const click = e => {
      e.preventDefault();
      e.stopPropagation();
      const p = pointed(e.target);
      end();
      chrome.runtime.sendMessage({ t: "picked", ...p, selection: String(getSelection() || "").slice(0, 2000) });
    };
    const key = e => { if (e.key === "Escape") end(); };
    // The page gets none of the choosing click (no focus, no action).
    const swallow = e => { e.preventDefault(); e.stopPropagation(); };
    const downs = ["pointerdown", "mousedown", "pointerup", "mouseup"];
    downs.forEach(k => addEventListener(k, swallow, true));
    const end0 = end;
    end = () => { downs.forEach(k => removeEventListener(k, swallow, true)); end0(); };
    addEventListener("mousemove", move, true);
    addEventListener("click", click, true);
    addEventListener("keydown", key, true);
  }

  // --- the user's input ----------------------------------------------------------
  function target(t) {
    if (!isEl(t)) return "";
    const role = roleOf(t);
    return role ? describeShort(t) : t.tagName.toLowerCase() + (t.id ? `#${t.id}` : "");
  }

  let lastReport = 0;
  for (const kind of ["pointerdown", "keydown", "wheel", "paste"]) {
    addEventListener(kind, e => {
      if (!session || !e.isTrusted || e.timeStamp === 0) return;
      if (host && e.composedPath().includes(host)) return; // the overlay's own buttons
      // Wheel and key repeats come fast: one report per 250 ms is enough to
      // pause and to keep the idle clock running.
      const now = Date.now();
      if (kind === "wheel" && now - lastReport < 250) return;
      lastReport = now;
      // The key, for what is not a character typed in a secret field.
      const key = kind === "keydown" && typeof e.key === "string" && !(isEl(e.target) && sensitive(e.target)) ? e.key : undefined;
      chrome.runtime.sendMessage({ t: "user_input", kind, target: target(e.target), key }).catch(() => {});
    }, { capture: true, passive: true });
  }

  // --- messages from the service worker ---------------------------------------------
  chrome.runtime.onMessage.addListener((msg, _sender, reply) => {
    try {
      switch (msg.t) {
        case "overlay": session ??= "?"; setState(msg); return reply({ ok: true });
        case "ask": ask(msg); return reply({ ok: true, result: {} });
        case "pointed": return reply({ ok: true, result: pointed(lastContext) });
        case "pick": pick(); return reply({ ok: true, result: {} });
        case "snapshot": session ??= "?"; return reply({ ok: true, result: snapshot(msg) });
        case "text": return reply({ ok: true, result: pageText(msg.max_chars, msg.whole === true) });
        case "find": return reply({ ok: true, result: find(msg) });
        case "resolve": return reply({ ok: true, result: resolve(msg) });
        case "fill": return reply({ ok: true, result: fill(msg) });
        // Without the debugger (sw.js): CDP input played as DOM events, and
        // Page.getLayoutMetrics' sizes.
        case "synth": return reply({ ok: true, result: synth(msg) });
        case "metrics": {
          const de = document.documentElement;
          const vp = { clientWidth: innerWidth, clientHeight: innerHeight, pageX: scrollX, pageY: scrollY };
          return reply({ ok: true, result: { cssContentSize: { x: 0, y: 0, width: Math.max(de.scrollWidth, innerWidth), height: Math.max(de.scrollHeight, innerHeight) },
            cssLayoutViewport: vp, cssVisualViewport: { ...vp, offsetX: 0, offsetY: 0, scale: 1, zoom: 1 } } });
        }
        case "info": return reply({ ok: true, result: { doc: DOC, url: location.href, title: document.title, ready: document.readyState,
          scroll: { x: scrollX, y: scrollY }, viewport: { w: innerWidth, h: innerHeight, dpr: devicePixelRatio },
          focused: focusedLine(),
          selection: clean(String(getSelection() || ""), 2000) } });
        // A file input marked for CDP to find (DOM.setFileInputFiles): an
        // attribute while it is set, removed after.
        case "markFile": {
          const el = element(msg.ref, msg.doc);
          if (el.tagName !== "INPUT" || el.type !== "file") throw new Error(`${msg.ref} is not a file input`);
          if (msg.off) el.removeAttribute("data-stitch-upload"); else el.setAttribute("data-stitch-upload", msg.mark);
          return reply({ ok: true, result: { multiple: el.multiple, accept: el.accept } });
        }
        case "scrollIntoView": element(msg.ref, msg.doc).scrollIntoView({ block: "center", behavior: "instant" }); return reply({ ok: true, result: {} });
      }
    } catch (e) {
      return reply({ ok: false, error: String(e?.message || e) });
    }
    reply({ ok: false, error: `unknown message ${msg.t}` });
  });

  // A session tab that (re)loads gets its overlay back.
  chrome.runtime.sendMessage({ t: "hello" }).then(r => {
    if (!r) return;
    session = r.session;
    // In a frame only the input watcher runs; the overlay is the top page's.
    if (r.frame) return;
    const start = () => setState({ held: r.held, stopped: r.stopped, state: r.stopped ? "Stopped" : r.held ? "You have this tab" : r.title || "Stitch" });
    document.readyState === "loading" ? addEventListener("DOMContentLoaded", start, { once: true }) : start();
  }).catch(() => {});
})();
