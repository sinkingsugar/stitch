/* Islands: zoomable circle packing on a 2D canvas.  root > places > efforts > spawned threads. */
(function () {
  'use strict';
  const A = window.Atlas;
  const V = (A.views.islands = { name: 'islands' });
  const host = document.getElementById('v-islands');
  const canvas = host.querySelector('canvas');
  const ctx = canvas.getContext('2d');
  const TAU = Math.PI * 2;
  const WORLD = 1000;
  const LOD_KIDS = 40; // an effort / thread shows the circles inside it from this on-screen radius
  const FONT = A.FONT;
  const INST = /instant/.test(location.search);

  let W = 0, H = 0, dpr = 1, topOff = 56, T = d3.zoomIdentity;
  let root = null, placeNodes = [], index = new Map(); // id → node (threads and efforts), 'p:'+key → node
  let focus = null, hoverNode = null, selNode = null;
  let dirty = true, active = true, lastPulse = 0, userMoved = false, needFit = true, built = false;
  const labels = [], placed = [], dots = [], rings = [];
  const zoom = d3.zoom();

  // ---------------------------------------------------------------- layout (deterministic: sorted by name / start time)
  function mkThread(t, isEffort) {
    const d = { kind: isEffort ? 'effort' : 'thread', t, id: t.id, eff: t.effort };
    const kids = t.kids.map((k) => mkThread(k, false));
    if (kids.length) d.children = [{ self: true, t, value: Math.pow(t.w, 0.3) }].concat(kids);
    else d.value = Math.pow(t.w, 0.3);
    return d;
  }
  function build() {
    const D = A.data;
    const old = new Map();
    if (root) root.each((n) => old.set(nk(n), { x: n.x, y: n.y, r: n.r }));
    const vp = A.visPlaces ? A.visPlaces() : D.places;
    const tree = { kind: 'root', children: vp.map((p) => ({ kind: 'place', p, children: (A.visEfforts ? A.visEfforts(p) : p.efforts).map((e) => mkThread(e.root, true)) })) };
    if (!vp.length) { root = null; placeNodes = []; index.clear(); A.layout = null; return; }
    const h = d3.hierarchy(tree, (d) => d.children).sum((d) => d.value || 0);
    d3.pack().size([WORLD, WORLD]).padding((n) => (n.depth === 0 ? 14 : n.depth === 1 ? 3 : n.depth === 2 ? 1.8 : 1.2))(h);
    root = h;
    placeNodes = h.children || [];
    index = new Map();
    const pos = new Map(), pl = new Map();
    h.each((n) => {
      const d = n.data;
      n.hasKids = !!n.children;
      n.selfN = n.children ? n.children.find((c) => c.data.self) || null : null;
      if (d.kind === 'place') { index.set('p:' + d.p.key, n); pl.set(d.p.key, { x: n.x, y: n.y, r: n.r }); }
      else if (!d.self && d.t) { index.set(d.id, n); pos.set(d.id, { x: n.x, y: n.y, r: n.r }); }
    });
    A.layout = { size: WORLD, pos, places: pl };
    // smooth re-layout: nodes glide from where they were (new ones grow in place)
    if (old.size) {
      const nodes = [];
      h.each((n) => { const o = old.get(nk(n)), to = { x: n.x, y: n.y, r: n.r }; nodes.push({ n, from: o || { x: n.x, y: n.y, r: n.r * 0.2 }, to }); });
      layoutAnim = { t0: performance.now(), ms: INST ? 0 : 650, nodes };
      stepLayout(performance.now());
    } else layoutAnim = null;
    refreshStyles();
    focus = focus && index.get(focus.__id) ? index.get(focus.__id) : root;
    focus.__id = focus === root ? null : focusId(focus);
    selNode = A.sel ? nodeOfSel(A.sel) : null;
  }
  let layoutAnim = null, lastTime = 'all';
  const nk = (n) => { const d = n.data; return d.kind === 'root' ? 'root' : d.kind === 'place' ? 'p:' + d.p.key : d.self ? 's:' + d.t.id : d.id; };
  function stepLayout(now) {
    const a = layoutAnim; if (!a) return;
    let t = a.ms ? Math.min(1, (now - a.t0) / a.ms) : 1; const e = t * t * (3 - 2 * t);
    for (const q of a.nodes) { q.n.x = q.from.x + (q.to.x - q.from.x) * e; q.n.y = q.from.y + (q.to.y - q.from.y) * e; q.n.r = q.from.r + (q.to.r - q.from.r) * e; }
    dirty = true;
    if (t >= 1) layoutAnim = null;
  }
  function focusId(n) { return n.data.kind === 'place' ? 'p:' + n.data.p.key : n.data.id; }
  function nodeOfSel(s) { return s ? index.get(s.kind === 'place' ? 'p:' + s.id : s.id) || null : null; }

  function refreshStyles() {
    if (!root) return;
    const now = Date.now(), DAY = A.DAY;
    root.eachAfter((n) => {
      const d = n.data;
      if (d.kind === 'root') return;
      if (d.kind === 'place') {
        const p = d.p;
        n.fade = p.fade;
        n.agg = { need: p.need > 0, working: p.working > 0, unread: p.unread > 0 };
        return;
      }
      const t = d.t;
      let need = t.need, working = t.working, unread = t.unread, e = t.e, fade = t.fade;
      if (!d.self && n.children) for (const c of n.children) {
        if (c.data.self) continue;
        need = need || c.agg.need; working = working || c.agg.working; unread = unread || c.agg.unread;
        e = Math.max(e, c.agg.e); fade = Math.max(fade, c.fade);
      }
      n.agg = { need, working, unread, e };
      n.fade = fade;
      n.ownSty = A.style(A.kindOf(t), (now - t.e) / DAY);
      n.sty = n.children && !d.self ? A.style(need ? 'need' : working ? 'work' : 'idle', (now - e) / DAY) : n.ownSty;
      n.own = { need: t.need, working: t.working, unread: t.unread };
    });
  }

  // ---------------------------------------------------------------- view transforms
  function availBox(forNode) {
    const cardW = W > 700 && forNode && forNode !== root ? 330 : 0;
    const sheet = W <= 700 && forNode && forNode !== root && A.sel ? H * 0.42 : 0;
    return { x0: 0, x1: W - cardW, y0: topOff, y1: H - (W > 700 ? 26 : 12) - sheet };
  }
  function fitT(n, pad, maxR) {
    const b = availBox(n);
    const aw = b.x1 - b.x0, ah = b.y1 - b.y0;
    let k = Math.min(aw, ah) / (2 * n.r * (pad || 1.1));
    if (maxR) k = Math.min(k, maxR / n.r);
    return d3.zoomIdentity.translate((b.x0 + b.x1) / 2 - k * n.x, (b.y0 + b.y1) / 2 - k * n.y).scale(k);
  }
  function flyTo(t, ms) {
    userMoved = true;
    const sel = d3.select(canvas);
    if (ms === 0) { sel.interrupt().call(zoom.transform, t); return; }
    sel.interrupt().transition().duration(ms || 800).ease(d3.easeCubicInOut).call(zoom.transform, t);
  }
  function focusOn(n, ms) {
    focus = n || root;
    focus.__id = focus === root ? null : focusId(focus);
    if (!root) return;
    const leafish = !focus.children;
    flyTo(fitT(focus, focus === root ? 1.04 : 1.12, leafish ? 120 : 0), ms);
  }
  function parentFocus(n) {
    // one level out: thread → its parent thread, effort → place, place → root
    return n && n.parent ? (n.parent.data.kind === 'root' ? root : n.parent) : root;
  }
  function selFor(n) {
    const d = n.data;
    if (d.kind === 'place') return { kind: 'place', id: d.p.key };
    if (d.self) return { kind: d.t.rootId === d.t.id ? 'effort' : 'thread', id: d.t.id };
    return { kind: d.kind, id: d.id };
  }
  function effortOf(n) { while (n && n.data.kind !== 'effort' && n.data.kind !== 'place') n = n.parent; return n; }

  // ---------------------------------------------------------------- hit testing (mirrors what is drawn)
  function pick(px, py) {
    if (!root) return null;
    const k = T.k, wx = (px - T.x) / k, wy = (py - T.y) / k, tol = 6 / k;
    let best = null;
    for (const p of placeNodes) {
      const dx = wx - p.x, dy = wy - p.y;
      if (dx * dx + dy * dy <= (p.r + tol * 0.3) * (p.r + tol * 0.3)) { best = p; break; }
    }
    if (!best) return null;
    let n = best;
    for (;;) {
      if (!n.children || (n.data.kind !== 'place' && n.r * k < LOD_KIDS)) break;
      let hit = null, hd = 1e9;
      for (const c of n.children) {
        const dx = wx - c.x, dy = wy - c.y, rr = Math.max(c.r, tol);
        const dd = Math.sqrt(dx * dx + dy * dy);
        if (dd <= rr && dd - c.r < hd) { hit = c; hd = dd - c.r; }
      }
      if (!hit) break;
      if (hit.data.self) { break; } // the root thread's own disc stands for the container
      n = hit;
    }
    return n;
  }

  // ---------------------------------------------------------------- drawing
  function draw(ts) {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, W, H);
    if (!root) return;
    labels.length = 0; placed.length = 0; dots.length = 0; rings.length = 0;
    const time = ts / 1000;
    for (const p of placeNodes) drawPlace(p, time);
    // labels over everything
    ctx.textAlign = 'center'; ctx.textBaseline = 'middle'; ctx.lineJoin = 'round';
    drawPlaceLabels(time);
    for (const l of labels) if (l.tag) tagLabel(l);
    for (const l of labels) if (l.leaf) leafLabel(l);
    // spawn-tree edges of the hovered / selected effort
    const fe = new Set();
    for (const s of [A.sel, A.hover]) { const id = s && effortIdOf(s); const n = id && index.get(id); if (n && !fe.has(n)) { fe.add(n); drawEdges(n); } }
    // rings (hover/selection), then needs-you dots on top
    for (const r of rings) {
      ctx.globalAlpha = 1; ctx.beginPath(); ctx.arc(r.x, r.y, r.r, 0, TAU);
      ctx.lineWidth = r.w; ctx.strokeStyle = r.c; ctx.stroke();
    }
    for (const d of dots) drawNeed(d);
    ctx.globalAlpha = 1;
  }

  function effortIdOf(s) {
    if (s.kind === 'effort') return s.id;
    if (s.kind === 'thread') { const t = A.data.byId.get(s.id); return t && t.rootId; }
    return null;
  }
  function drawEdges(eff) {
    const k = T.k;
    if (!eff.children || eff.r * k < 14) return;
    const ctr = (n) => (n.selfN || n);
    ctx.lineCap = 'round';
    (function walk(n) {
      const a = ctr(n);
      for (const c of n.children || []) {
        if (c.data.self) continue;
        const b = ctr(c);
        const x0 = a.x * k + T.x, y0 = a.y * k + T.y, x1 = b.x * k + T.x, y1 = b.y * k + T.y;
        const t = c.data.t;
        const col = t.need ? '251,146,60' : t.working ? '45,212,191' : '190,214,250';
        const strong = t.need || t.working;
        const mx = (x0 + x1) / 2, my = (y0 + y1) / 2, dx = x1 - x0, dy = y1 - y0, bend = 0.12;
        ctx.globalAlpha = Math.max(0.25, c.fade);
        ctx.strokeStyle = `rgba(${col},${strong ? 0.8 : 0.42})`;
        ctx.lineWidth = strong ? 1.8 : 1.2;
        ctx.beginPath(); ctx.moveTo(x0, y0); ctx.quadraticCurveTo(mx - dy * bend, my + dx * bend, x1, y1); ctx.stroke();
        walk(c);
      }
    })(eff);
    ctx.globalAlpha = 1;
  }
  function onscreen(x, y, r) { return x + r >= 0 && x - r <= W && y + r >= 0 && y - r <= H; }

  function drawPlace(p, time) {
    const k = T.k, sx = p.x * k + T.x, sy = p.y * k + T.y, rs = p.r * k;
    const pd = p.data.p;
    const needDot = p.agg.need;
    if (!onscreen(sx, sy, rs * 1.15) && !needDot) return;
    if (onscreen(sx, sy, rs * 1.15)) {
      const f = p.fade;
      // soft island
      ctx.globalAlpha = 0.25 + 0.75 * f;
      let g = ctx.createRadialGradient(sx, sy, rs * 0.2, sx, sy, rs * 1.02);
      g.addColorStop(0, 'rgba(120,150,196,.05)'); g.addColorStop(0.82, 'rgba(120,150,196,.085)'); g.addColorStop(1, 'rgba(120,150,196,0)');
      ctx.fillStyle = g; ctx.beginPath(); ctx.arc(sx, sy, rs * 1.02, 0, TAU); ctx.fill();
      if (rs > 4) {
        ctx.globalAlpha = (0.12 + 0.1 * Math.min(1, rs / 60)) * (0.3 + 0.7 * f);
        ctx.beginPath(); ctx.arc(sx, sy, rs, 0, TAU); ctx.lineWidth = 1; ctx.strokeStyle = '#9db4d8'; ctx.stroke();
      }
      // activity glow
      if (pd.working > 0 && rs > 8) {
        const a = Math.min(1, pd.working / 4) * 0.07 * f * (0.85 + 0.15 * Math.sin(time * 1.6 + p.x));
        ctx.globalAlpha = 1;
        g = ctx.createRadialGradient(sx, sy, rs * 0.7, sx, sy, rs * 1.18);
        g.addColorStop(0, `rgba(45,212,191,${a})`); g.addColorStop(0.85, `rgba(45,212,191,${a * 0.4})`); g.addColorStop(1, 'rgba(45,212,191,0)');
        ctx.fillStyle = g; ctx.beginPath(); ctx.arc(sx, sy, rs * 1.18, 0, TAU); ctx.fill();
      }
      if (hoverNode === p) rings.push({ x: sx, y: sy, r: rs, w: 1.5, c: 'rgba(230,240,255,.5)' });
      if (selNode === p) rings.push({ x: sx, y: sy, r: rs, w: 2, c: 'rgba(240,248,255,.85)' });
      for (const e of p.children) drawNode(e, time);
    } else {
      for (const e of p.children) drawNode(e, time);
    }
    labels.push({ place: p, rs, sx, sy });
  }

  function drawNode(n, time) {
    const k = T.k, sx = n.x * k + T.x, sy = n.y * k + T.y, rs = n.r * k;
    const a = n.agg, vis = onscreen(sx, sy, rs + 12);
    if (!vis && !(a.need)) return;
    if (!vis) return;
    const d = n.data;
    const container = n.hasKids && !d.self && rs >= LOD_KIDS;
    if (rs < 0.5 && !a.need && !a.working) return;
    const sty = container ? n.sty : n.hasKids && !d.self ? n.sty : n.ownSty || n.sty;
    const flags = container ? null : n.hasKids && !d.self ? a : n.own || a;
    const f = n.fade;
    ctx.globalAlpha = f;
    if (container) {
      ctx.beginPath(); ctx.arc(sx, sy, rs, 0, TAU);
      ctx.fillStyle = `rgba(${sty.rgb},${a.need ? 0.1 : 0.085})`; ctx.fill();
      ctx.lineWidth = a.need ? 1.5 : 1; ctx.strokeStyle = sty.stroke; ctx.globalAlpha = f * (a.need ? 0.9 : 0.55);
      if (d.t && d.t.synthetic) { ctx.lineWidth = 1.4; ctx.globalAlpha = f * 0.85; ctx.setLineDash([5, 4]); }
      ctx.stroke(); ctx.setLineDash([]);
      ctx.globalAlpha = f;
      for (const c of n.children) drawNode(c, time);
      if (rs >= (d.kind === 'effort' ? 70 : 96)) labels.push({ tag: n, sx, sy, rs, sty });
    } else if (rs >= 0.5) {
      // working halo
      if (flags.working && rs > 1) {
        const pr = Math.max(rs, 3) * (2.1 + 0.45 * Math.sin(time * 2.1 + n.x * 0.37));
        const g = ctx.createRadialGradient(sx, sy, rs * 0.5, sx, sy, pr);
        g.addColorStop(0, 'rgba(45,212,191,.38)'); g.addColorStop(1, 'rgba(45,212,191,0)');
        ctx.globalAlpha = f; ctx.fillStyle = g; ctx.beginPath(); ctx.arc(sx, sy, pr, 0, TAU); ctx.fill();
      }
      ctx.globalAlpha = f;
      ctx.beginPath(); ctx.arc(sx, sy, rs, 0, TAU);
      ctx.fillStyle = sty.fill; ctx.fill();
      if (rs > 3) { ctx.lineWidth = 1; ctx.strokeStyle = sty.stroke; if (d.t && d.t.synthetic) ctx.setLineDash([4, 3]); ctx.stroke(); ctx.setLineDash([]); }
      if (flags.unread && A.unreadOn && rs > 2.5) { // quiet cue: a hairline, low contrast
        ctx.beginPath(); ctx.arc(sx, sy, rs + 2, 0, TAU);
        ctx.lineWidth = 0.8; ctx.strokeStyle = 'rgba(170,195,230,.38)'; ctx.stroke();
      }
      if (rs >= 30) labels.push({ leaf: n, sx, sy, rs, sty });
    }
    if (flags && flags.need) dots.push({ x: sx, y: sy, r: Math.max(rs * (rs > 8 ? 0 : 1), 0) + 0, rs });
    else if (container && n.own && n.own.need) { /* root thread's own disc handles it */ }
    if (hoverNode === n) rings.push({ x: sx, y: sy, r: rs + 2, w: 2, c: 'rgba(255,255,255,.75)' });
    if (selNode === n) rings.push({ x: sx, y: sy, r: rs + 3, w: 2.5, c: '#fff' });
  }

  function drawNeed(d) {
    // small orange marker visible from any zoom
    const r = d.rs > 8 ? 0 : Math.max(d.rs, 3.2);
    if (d.rs <= 8) {
      const g = ctx.createRadialGradient(d.x, d.y, 0, d.x, d.y, r * 3.4);
      g.addColorStop(0, 'rgba(251,146,60,.5)'); g.addColorStop(1, 'rgba(251,146,60,0)');
      ctx.globalAlpha = 1; ctx.fillStyle = g; ctx.beginPath(); ctx.arc(d.x, d.y, r * 3.4, 0, TAU); ctx.fill();
      ctx.beginPath(); ctx.arc(d.x, d.y, r, 0, TAU); ctx.fillStyle = '#fb923c'; ctx.fill();
      ctx.lineWidth = 1.2; ctx.strokeStyle = 'rgba(10,14,21,.9)'; ctx.stroke();
    } else {
      // big circles: a badge on the upper-right rim
      const bx = d.x + d.rs * 0.7, by = d.y - d.rs * 0.7;
      ctx.globalAlpha = 1; ctx.beginPath(); ctx.arc(bx, by, 4.2, 0, TAU); ctx.fillStyle = '#fb923c'; ctx.fill();
      ctx.lineWidth = 1.5; ctx.strokeStyle = 'rgba(10,14,21,.95)'; ctx.stroke();
    }
  }

  // ---- labels
  function hit(rect) { for (const r of placed) if (rect.x < r.x + r.w && rect.x + rect.w > r.x && rect.y < r.y + r.h && rect.y + rect.h > r.y) return true; return false; }
  function halo(text, x, y, color, w) {
    ctx.lineWidth = w || 3.2; ctx.strokeStyle = 'rgba(8,11,17,.78)'; ctx.strokeText(text, x, y);
    ctx.fillStyle = color; ctx.fillText(text, x, y);
  }
  function setFont(size, weight) { ctx.font = `${weight || 500} ${size}px ${FONT}`; }

  function leafLabel(l) {
    const n = l.leaf, t = n.data.t, rs = l.rs;
    const title = t.title || t.id;
    const maxW = rs * 1.52, maxH = rs * 1.32;
    let size = Math.min(15, Math.max(10, rs * 0.2)), lines = null;
    for (; size >= 10; size -= 1) {
      const w = A.wrap(title, size, maxW, 3);
      if (!w.cut && w.lines.length * size * 1.22 <= maxH) { lines = w.lines; break; }
    }
    if (!lines) {
      size = 10;
      if (maxW < 40) return;
      const ml = Math.max(1, Math.min(3, Math.floor(maxH / 12.2)));
      lines = A.wrap(title, 10, maxW, ml).lines;
    }
    let sub = null, subSize = Math.max(10, size - 2);
    if (rs >= 62) {
      const s = (t.digest && t.digest.headline) || t.last_words;
      if (s) {
        const left = maxH - lines.length * size * 1.22 - 8;
        const ml = Math.min(3, Math.floor(left / (subSize * 1.22)));
        if (ml >= 1) { const w = A.wrap(s, subSize, maxW, ml); if (w.lines.length) sub = w.lines; }
      }
    }
    const th = lines.length * size * 1.22, sh = sub ? sub.length * subSize * 1.22 + 8 : 0;
    const bw = Math.min(maxW, Math.max(...lines.map((q) => A.tw(q) * size))) + 4;
    if (hit({ x: l.sx - bw / 2, y: l.sy - (th + sh) / 2, w: bw, h: th + sh })) return;
    let y = l.sy - (th + sh) / 2 + size * 0.61;
    ctx.globalAlpha = Math.max(0.25, n.fade);
    setFont(size, 600);
    for (const ln of lines) { halo(ln, l.sx, y, l.sty.text, 3); y += size * 1.22; }
    if (sub) {
      y += 8 - size * 0.61 + subSize * 0.61;
      setFont(subSize, 400);
      ctx.globalAlpha = Math.max(0.2, n.fade) * 0.72;
      for (const ln of sub) { halo(ln, l.sx, y, '#d5deea', 2.6); y += subSize * 1.22; }
    }
  }
  function tagLabel(l) {
    const n = l.tag, t = n.data.t;
    const size = l.rs > 260 ? 13 : 12;
    const text = A.ellipsize(t.title || t.id, size, Math.min(l.rs * 1.5, 300));
    if (!text) return;
    setFont(size, 600);
    const w = A.tw(text) * size + 16, h = 19;
    const rect = { x: l.sx - w / 2, y: l.sy - l.rs - h / 2, w, h };
    if (rect.y < topOff - 4 || rect.y + h > H || rect.x + w < 0 || rect.x > W) return;
    if (hit(rect)) return;
    placed.push(rect);
    ctx.globalAlpha = Math.max(0.3, n.fade);
    ctx.beginPath(); ctx.roundRect(rect.x, rect.y, w, h, 9.5);
    ctx.fillStyle = 'rgba(10,14,22,.86)'; ctx.fill();
    ctx.lineWidth = 1; ctx.strokeStyle = l.sty.stroke; ctx.stroke();
    ctx.fillStyle = '#e6ebf2'; ctx.fillText(text, l.sx, l.sy - l.rs + 0.5);
  }
  function drawPlaceLabels(time) {
    const list = labels.filter((l) => l.place).sort((a, b) => b.rs - a.rs);
    for (const l of list) {
      const p = l.place, pd = p.data.p, rs = l.rs;
      if (!onscreen(l.sx, l.sy, rs)) continue;
      const mode = A.smooth(130, 330, rs); // 0: centred over the island, 1: tag on its rim
      const isSel = selNode === p;
      let size = A.lerp(Math.min(32, Math.max(11, rs * 0.25)), 13, mode);
      const name = pd.name;
      const maxW = A.lerp(rs * 1.7, Math.min(rs * 1.4, 320), mode);
      if (A.tw(name) * size > maxW) size = maxW / A.tw(name);
      if (size < 10.5 && !isSel) continue;
      if (size < 10.5) size = 10.5;
      setFont(size, 650);
      const tw = A.tw(name) * size;
      const badge = pd.need > 0;
      const cy = l.sy - mode * rs - (mode ? 0 : 0);
      const hasCount = rs > 34 && mode < 1;
      const w = tw + (mode > 0.5 ? 20 : 4) + (badge ? 18 : 0);
      const rect = { x: l.sx - w / 2, y: cy - size * 0.75 - (hasCount && mode < 0.5 ? 0 : 0), w, h: size * 1.5 + (hasCount && mode < 0.5 ? size * 0.9 : 0) };
      if (!isSel && hit(rect)) continue;
      placed.push(rect);
      ctx.globalAlpha = (0.35 + 0.65 * p.fade) * (isSel ? 1 : 0.96);
      if (mode > 0.35) {
        const pw = w, ph = size + 9, px = l.sx - pw / 2, py = cy - ph / 2;
        ctx.globalAlpha *= A.smooth(0.35, 0.7, mode);
        ctx.beginPath(); ctx.roundRect(px, py, pw, ph, ph / 2); ctx.fillStyle = 'rgba(14,19,28,.9)'; ctx.fill();
        ctx.lineWidth = 1; ctx.strokeStyle = 'rgba(157,180,216,.35)'; ctx.stroke();
        ctx.globalAlpha = (0.35 + 0.65 * p.fade);
      }
      const tx = l.sx - (badge ? 9 : 0);
      halo(name, tx, cy, mode > 0.5 ? '#f1f5fb' : '#e9eff8', mode > 0.5 ? 0 : 4);
      if (hasCount && mode < 0.5) {
        setFont(Math.max(10, size * 0.5), 500);
        ctx.globalAlpha *= 1 - mode * 2;
        halo(pd.count + (pd.count === 1 ? ' thread' : ' threads'), tx, cy + size * 0.95, '#9fb0c6', 3);
        ctx.globalAlpha = (0.35 + 0.65 * p.fade);
      }
      if (badge) {
        const bx = tx + tw / 2 + 13, r = Math.max(7, size * 0.42);
        ctx.beginPath(); ctx.arc(bx, cy, r, 0, TAU); ctx.fillStyle = '#fb923c'; ctx.fill();
        ctx.fillStyle = '#1a0f05'; setFont(Math.max(9, r * 1.15), 700); ctx.fillText(String(Math.min(pd.need, 99)), bx, cy + 0.5);
      }
    }
  }

  // ---------------------------------------------------------------- loop
  function frame(ts) {
    requestAnimationFrame(frame);
    if (layoutAnim) stepLayout(ts);
    if (!active || !W) return;
    const pulse = A.data.threads.length && (A.stats ? true : true) && ts - lastPulse > 60;
    if (dirty || (pulse && hasWorking())) { dirty = false; lastPulse = ts; draw(ts); }
  }
  function hasWorking() { return A.data.places.some((p) => p.working > 0); }

  function resize() {
    const r = host.getBoundingClientRect();
    if (!r.width || !r.height) return;
    const bar = document.getElementById('bar');
    topOff = (bar ? bar.offsetHeight : 48) + 4;
    const nw = Math.round(r.width), nh = Math.round(r.height);
    dpr = Math.min(window.devicePixelRatio || 1, 3);
    if (nw === W && nh === H && canvas.width === Math.round(nw * dpr)) return;
    W = nw; H = nh;
    canvas.width = Math.round(W * dpr); canvas.height = Math.round(H * dpr);
    zoom.extent([[0, 0], [W, H]]);
    if (root && (needFit || !userMoved)) { flyTo(fitT(focus || root, focus && focus !== root ? 1.12 : 1.04, focus && !focus.children ? 120 : 0), 0); needFit = false; userMoved = false; }
    dirty = true;
  }

  // ---------------------------------------------------------------- interaction
  function setHoverNode(n) {
    if (n === hoverNode) return;
    hoverNode = n; dirty = true;
    canvas.style.cursor = n ? 'pointer' : 'default';
    A.setHover(n ? selFor(n) : null);
  }
  function onClick(ev) {
    const r = canvas.getBoundingClientRect();
    const n = pick(ev.clientX - r.left, ev.clientY - r.top);
    if (!n || n === focus || (n === root)) {
      // empty space (or the focus itself): out one level
      const up = !n && focus === root ? root : parentFocus(focus);
      focusOn(up, 750);
      A.select(up === root ? null : selFor(up), 'islands');
      return;
    }
    const d = n.data;
    if (d.kind === 'place' || d.kind === 'effort') {
      focusOn(n, 800);
      A.select(selFor(n), 'islands');
    } else {
      const eff = effortOf(n);
      A.select(selFor(n), 'islands');
      // first bring the whole effort into view, then threads inside it select without moving
      if (eff && focus !== eff && !isInside(n, focus)) focusOn(eff, 800);
    }
  }
  function isInside(n, anc) { for (let x = n; x; x = x.parent) if (x === anc) return true; return false; }

  function fly(sel) {
    const n = nodeOfSel(sel);
    if (!n) return;
    const d = n.data;
    if (d.kind === 'place' || d.kind === 'effort') focusOn(n, INST ? 0 : 900);
    else { const eff = effortOf(n); focus = eff; focus.__id = focusId(eff); flyTo(fitT(n, 1.5, 150), INST ? 0 : 900); }
  }

  // ---------------------------------------------------------------- public
  V.show = () => { active = true; host.classList.add('on'); resize(); if (pendingFly) { const s = pendingFly; pendingFly = null; fly(s); } dirty = true; };
  V.hide = () => { active = false; };
  let pendingFly = null;

  function init() {
    zoom.scaleExtent([0.3, 1500]).filter((e) => e.type !== 'dblclick' && (!e.ctrlKey || e.type === 'wheel') && !e.button)
      .on('zoom', (e) => { T = e.transform; dirty = true; if (e.sourceEvent) userMoved = true; });
    d3.select(canvas).call(zoom).on('dblclick.zoom', null);
    canvas.addEventListener('click', onClick);
    canvas.addEventListener('pointermove', (e) => {
      if (e.pointerType === 'touch' || e.buttons) return;
      const r = canvas.getBoundingClientRect();
      setHoverNode(pick(e.clientX - r.left, e.clientY - r.top));
    });
    canvas.addEventListener('pointerleave', () => setHoverNode(null));
    new ResizeObserver(resize).observe(host);
    A.on('data', () => {
      const first = !built, timeChanged = A.f.time !== lastTime; lastTime = A.f.time;
      if (timeChanged) focus = null;
      build(); built = true; if (first) needFit = true; resize();
      if (first && root) { flyTo(fitT(root, 1.04), 0); userMoved = false; needFit = false; }
      else if (timeChanged && root) { focus = root; focus.__id = null; flyTo(fitT(root, 1.04), INST ? 0 : 650); userMoved = false; }
      dirty = true;
    });
    A.on('filter', () => { refreshStyles(); dirty = true; });
    A.on('select', (s, src) => {
      selNode = s ? nodeOfSel(s) : null; dirty = true;
      if (s && src !== 'islands') { if (active) fly(s); else pendingFly = s; }
    });
    A.on('hover', () => { dirty = true; });
    A.on('escape', () => { dirty = true; });
    requestAnimationFrame(frame);
    // test hooks
    V.debug = { get T() { return T; }, flyTo, fitT, get root() { return root; }, index: () => index, pick, focusOn };
  }
  init();
})();
