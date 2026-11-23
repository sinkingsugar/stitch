/* Atlas: shared model (data → efforts → places), palette, filters, selection, events. Declarations only; ui.js runs it. */
(function () {
  'use strict';
  const A = (window.Atlas = {});
  window.__errors = [];
  window.addEventListener('error', (e) => window.__errors.push(String(e.message) + ' @' + String(e.filename || '').split('/').pop() + ':' + e.lineno));
  window.addEventListener('unhandledrejection', (e) => window.__errors.push('rejection: ' + ((e.reason && e.reason.message) || e.reason)));

  const DAY = 86400000;
  A.DAY = DAY;
  A.FONT = '-apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, system-ui, sans-serif';
  A.views = {};
  A.data = { threads: [], byId: new Map(), efforts: [], places: [], now: Date.now() };
  A.f = { q: '', time: 'all', needs: false, unread: null }; // unread: null = automatic (off when most threads are unread)
  A.unreadOn = true;
  A.sel = null;
  A.hover = null;

  // ---- events
  const subs = {};
  A.on = (n, fn) => ((subs[n] = subs[n] || []).push(fn), fn);
  A.emit = (n, a, b) => (subs[n] || []).forEach((fn) => { try { fn(a, b); } catch (e) { window.__errors.push('handler ' + n + ': ' + e.message); console.error(e); } });

  // ---- small helpers
  A.clamp = (x, a, b) => Math.max(a, Math.min(b, x));
  A.lerp = (a, b, t) => a + (b - a) * t;
  A.smooth = (a, b, x) => { const t = A.clamp((x - a) / (b - a), 0, 1); return t * t * (3 - 2 * t); };
  A.esc = (s) => String(s == null ? '' : s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  A.WD = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  A.MON = MON;
  A.fmtTime = (ms) => { const d = new Date(ms); return MON[d.getMonth()] + ' ' + d.getDate() + ' ' + String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0'); };
  A.ago = (ms) => {
    const s = Math.max(0, (Date.now() - ms) / 1000);
    if (s < 90) return 'just now';
    if (s < 5400) return Math.round(s / 60) + ' min ago';
    if (s < 129600) return Math.round(s / 3600) + ' h ago';
    return Math.round(s / 86400) + ' d ago';
  };
  A.money = (c) => (c >= 100 ? '$' + Math.round(c) : c >= 1 ? '$' + c.toFixed(2) : c > 0 ? '$' + c.toFixed(3) : '—');
  A.placeName = (k) => {
    if (!k || k === 'global') return 'global';
    const s = String(k).replace(/\/+$/, '').split('/');
    return s[s.length - 1] || k;
  };
  const ts = (v) => {
    if (v == null || v === '') return null;
    if (typeof v === 'number') return v < 1e12 ? v * 1000 : v;
    const n = Date.parse(v);
    return isNaN(n) ? null : n;
  };

  // ---- palette: working = teal, needs you = orange, unread = bright ring, idle = grey-blue that dims with age
  A.kindOf = (o) => (o.need ? 'need' : o.working ? 'work' : 'idle');
  const styleCache = new Map();
  A.style = (kind, ageDays) => {
    const a = Math.round(A.clamp(ageDays / 14, 0, 1) * 20);
    const key = kind + (kind === 'idle' ? a : '');
    let s = styleCache.get(key);
    if (s) return s;
    if (kind === 'work') s = { fill: 'rgba(22,166,150,.9)', stroke: 'rgba(110,238,218,.95)', soft: 'rgba(45,212,191,.12)', rgb: '45,212,191', text: '#f4fffd' };
    else if (kind === 'need') s = { fill: 'rgba(232,118,34,.94)', stroke: 'rgba(255,196,140,.98)', soft: 'rgba(251,146,60,.12)', rgb: '251,146,60', text: '#fff8f2' };
    else {
      const t = a / 20, L = A.lerp(50, 19, t), al = A.lerp(.92, .5, t);
      s = { fill: `hsla(216,26%,${L}%,${al})`, stroke: `hsla(216,32%,${L + 16}%,${A.lerp(.9, .4, t)})`, soft: `hsla(216,26%,${L + 8}%,.08)`, rgb: '111,134,168', text: t > .6 ? '#aab6c7' : '#eef3fa', dim: t };
    }
    styleCache.set(key, s);
    return s;
  };

  // ---- text measuring (shared by canvas views): width per px of font size, cached
  const mc = document.createElement('canvas').getContext('2d');
  const wc = new Map();
  A.tw = (s) => {
    let w = wc.get(s);
    if (w === undefined) {
      mc.font = '500 100px ' + A.FONT;
      w = mc.measureText(s).width / 100 * 1.03;
      if (wc.size > 20000) wc.clear();
      wc.set(s, w);
    }
    return w;
  };
  A.ellipsize = (s, size, maxW) => {
    if (A.tw(s) * size <= maxW) return s;
    let lo = 0, hi = s.length;
    while (lo < hi) { const m = (lo + hi + 1) >> 1; if (A.tw(s.slice(0, m).trimEnd() + '…') * size <= maxW) lo = m; else hi = m - 1; }
    return lo ? s.slice(0, lo).trimEnd() + '…' : '';
  };
  // greedy wrap; returns {lines, cut} ; cut = text did not fit in maxLines
  A.wrap = (text, size, maxW, maxLines) => {
    const words = String(text || '').split(/\s+/).filter(Boolean);
    const lines = [];
    let cur = '', i = 0;
    for (; i < words.length; i++) {
      const w = words[i], t = cur ? cur + ' ' + w : w;
      if (A.tw(t) * size <= maxW) { cur = t; continue; }
      if (cur) { lines.push(cur); cur = ''; if (lines.length >= maxLines) break; i--; continue; }
      // a single word wider than the line
      lines.push(A.ellipsize(w, size, maxW)); if (lines.length >= maxLines) { i++; break; }
    }
    let cut = i < words.length;
    if (!cut && cur) { if (lines.length < maxLines) lines.push(cur); else cut = true; }
    if (cut && lines.length) {
      const last = lines.length - 1;
      const rest = words.slice(i).join(' ');
      // put the ellipsis on the last line
      lines[last] = A.ellipsize(lines[last] + (rest ? ' ' + rest : ''), size, maxW);
      if (!lines[last].endsWith('…')) lines[last] = A.ellipsize(lines[last] + '…', size - 0.001, maxW);
    }
    return { lines, cut };
  };

  // ---- the model
  A.setData = (resp) => {
    const rows = (resp && resp.threads) || [];
    const now = Date.now();
    const byId = new Map();
    const threads = [];
    for (const r of rows) {
      if (!r || !r.id) continue;
      const t = Object.assign({}, r);
      t.s = ts(r.started) || ts(r.last_step) || now;
      t.e = Math.max(t.s, ts(r.last_step) || t.s);
      t.cost = Number(r.cost) || 0;
      t.calls = Number(r.model_calls) || 0;
      t.placeKey = r.place || 'global';
      t.need = r.state === 'blocked' || !!r.question;
      t.working = r.state === 'working';
      t.unread = !!r.unread;
      t.kids = [];
      t.hay = [r.title, r.goal, r.digest && r.digest.headline, r.last_words].filter(Boolean).join(' \n ').toLowerCase();
      byId.set(t.id, t);
      threads.push(t);
    }
    // weights: cost, else calls scaled by the median cost per call
    const cpcs = threads.filter((t) => t.cost > 0 && t.calls > 0).map((t) => t.cost / t.calls).sort((a, b) => a - b);
    const cpc = cpcs.length ? cpcs[cpcs.length >> 1] : 0.01;
    const w0 = threads.map((t) => (t.cost > 0 ? t.cost : t.calls > 0 ? t.calls * cpc : cpc * 3));
    const pos = w0.slice().sort((a, b) => a - b);
    const floor = (pos[Math.floor(pos.length * 0.2)] || 0.01) * 0.6;
    threads.forEach((t, i) => (t.w = Math.max(w0[i], floor)));
    // Parents. A parent that is not among the rows (deleted, on a repository this node does not hold, or a node that
    // predates `conversations`) makes the thread an ORPHAN: it joins a synthetic effort for its place, it is never
    // silently a root of its own. A cycle (should not exist) is cut at its earliest member, which becomes a root.
    const state = new Map(); // id → 1 (on the walk) | 2 (done)
    for (const t of threads) {
      if (state.get(t.id)) continue;
      const path = [];
      let c = t;
      while (c && !state.get(c.id)) {
        state.set(c.id, 1); path.push(c);
        const p = c.parent && byId.get(c.parent);
        if (!p || p === c) break;
        c = p;
      }
      const p = c && c.parent && byId.get(c.parent);
      if (p && p !== c && state.get(p.id) === 1) {
        const cyc = path.slice(path.indexOf(p));
        const head = cyc.reduce((m, x) => (x.s < m.s || (x.s === m.s && x.id < m.id) ? x : m));
        head.cycleCut = true;
      }
      path.forEach((x) => state.set(x.id, 2));
    }
    const orphans = new Map(); // place → [thread]
    let nOrphans = 0;
    for (const t of threads) {
      const p = t.parent && !t.cycleCut && t.parent !== t.id ? byId.get(t.parent) : null;
      if (p) p.kids.push(t);
      else if (t.parent && !t.cycleCut) {
        t.orphan = true; nOrphans++;
        let l = orphans.get(t.placeKey); if (!l) orphans.set(t.placeKey, (l = []));
        l.push(t);
      }
    }
    const synth = [];
    for (const [place, l] of orphans) {
      const id = 'orphans:' + place;
      const s = new Date(Math.min(...l.map((x) => x.s)));
      const r = { id, title: '(parent not loaded)', goal: '', placeKey: place, state: 'idle', synthetic: true, kids: l, s: +s, e: Math.max(...l.map((x) => x.e)), cost: 0, calls: 0, w: 0, need: false, working: false, unread: false, hay: '', parent: null };
      l.forEach((x) => { x.up = r; });
      byId.set(id, r); synth.push(r); threads.push(r);
    }
    if (synth.length) { const m = Math.min(...threads.filter((x) => !x.synthetic).map((x) => x.w)); synth.forEach((r) => (r.w = m || 0.01)); }
    // roots: follow the (now acyclic) parent links, however deep
    const rootOf = new Map();
    const findRoot = (t0) => {
      const chain = [];
      let r = t0;
      while (!rootOf.has(r.id)) {
        chain.push(r);
        const p = r.synthetic ? null : r.up || (r.parent && !r.cycleCut && r.parent !== r.id ? byId.get(r.parent) : null);
        if (!p) { rootOf.set(r.id, r.id); chain.pop(); break; }
        r = p;
      }
      const root = rootOf.get(r.id);
      chain.forEach((x) => rootOf.set(x.id, root));
      return root;
    };
    for (const t of threads) t.rootId = findRoot(t);
    for (const t of threads) t.kids.sort((a, b) => a.s - b.s || (a.id < b.id ? -1 : 1));
    const efforts = [];
    const eById = new Map();
    for (const t of threads) {
      if (t.rootId === t.id) {
        const e = { id: t.id, root: t, place: t.placeKey, threads: [], s: t.s, e: t.e, w: 0, cost: 0, need: false, working: false, unread: false };
        eById.set(t.id, e); efforts.push(e);
      }
    }
    for (const t of threads) {
      const e = eById.get(t.rootId);
      if (!e) continue;
      t.effort = e; e.threads.push(t);
      e.s = Math.min(e.s, t.s); e.e = Math.max(e.e, t.e); e.w += t.w; e.cost += t.cost;
      e.need = e.need || t.need; e.working = e.working || t.working; e.unread = e.unread || t.unread;
    }
    const pMap = new Map();
    for (const e of efforts) {
      let p = pMap.get(e.place);
      if (!p) { p = { key: e.place, name: A.placeName(e.place), efforts: [], need: 0, working: 0, unread: 0, count: 0, cost: 0, s: Infinity, e: 0 }; pMap.set(e.place, p); }
      p.efforts.push(e); p.count += e.threads.filter((x) => !x.synthetic).length; p.cost += e.cost; p.s = Math.min(p.s, e.s); p.e = Math.max(p.e, e.e);
      e.threads.forEach((t) => { if (t.need) p.need++; if (t.working) p.working++; if (t.unread) p.unread++; });
      e.placeObj = p;
    }
    const places = [...pMap.values()];
    places.forEach((p) => p.efforts.sort((a, b) => a.s - b.s || (a.id < b.id ? -1 : 1)));
    places.sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
    // duplicates in display names: disambiguate with the parent folder
    const seen = new Map();
    places.forEach((p) => seen.set(p.name, (seen.get(p.name) || 0) + 1));
    places.forEach((p) => { if (seen.get(p.name) > 1) { const s = p.key.split('/').filter(Boolean); p.name = s.slice(-2).join('/'); } });
    A.data = { threads, byId, efforts, eById, places, pMap, now, node: resp && resp.node, more: !!(resp && resp.more), real: threads.length - synth.length, orphans: nOrphans, legacy: !!(resp && resp.legacy) };
    if (A.sel && !A.resolve(A.sel)) A.sel = null;
    A.applyFilter(true);
  };

  A.resolve = (s) => {
    if (!s) return null;
    const d = A.data;
    if (s.kind === 'place') return d.pMap.get(s.id) || null;
    if (s.kind === 'effort') return d.eById.get(s.id) || null;
    return d.byId.get(s.id) || null;
  };

  // ---- filters → fade (0.1 … 1) per thread / effort / place
  A.cutoff = () => {
    const f = A.f.time, n = Date.now();
    if (f === 'today') { const d = new Date(); d.setHours(0, 0, 0, 0); return d.getTime(); }
    if (f === '3d') return n - 3 * DAY;
    if (f === 'week') return n - 7 * DAY;
    if (f === 'month') return n - 30 * DAY;
    return 0;
  };
  A.applyFilter = (silent) => {
    const toks = A.f.q.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const cut = A.cutoff();
    let hits = 0, needs = 0;
    for (const t of A.data.threads) {
      let f = 1;
      if (toks.length && !toks.every((k) => t.hay.includes(k))) f = Math.min(f, 0.1);
      else if (toks.length) hits++;
      if (cut && t.e < cut) f = Math.min(f, 0.2);
      if (A.f.needs && !t.need) f = Math.min(f, 0.12);
      if (t.need) needs++;
      t.fade = f;
    }
    // the time filter REMOVES: an effort stays whole if any of its threads is active in the window, a place if any effort is
    for (const e of A.data.efforts) { e.fade = e.threads.reduce((m, t) => Math.max(m, t.fade), 0); e.vis = !cut || e.threads.some((t) => t.e >= cut); }
    let nvis = 0;
    for (const p of A.data.places) { p.fade = p.efforts.reduce((m, e) => Math.max(m, e.fade), 0); p.vis = p.efforts.some((e) => e.vis); if (p.vis) nvis++; }
    A.visPlaces = () => A.data.places.filter((p) => p.vis);
    A.visEfforts = (p) => p.efforts.filter((e) => e.vis);
    const sig = A.data.places.map((p) => (p.vis ? p.efforts.filter((e) => e.vis).length : 0)).join(',') + '|' + A.data.efforts.length;
    const layoutChanged = sig !== A._visSig; A._visSig = sig;
    // selection / hover that are no longer visible are dropped
    const gone = (o) => { if (!o) return false; const x = A.resolve(o); if (!x) return true; if (o.kind === 'place') return !x.vis; const e = o.kind === 'effort' ? x : x.effort; return !!(e && !e.vis); };
    const dropSel = gone(A.sel);
    if (gone(A.hover)) A.hover = null;
    A.nPlacesVis = nvis;
    let unread = 0;
    for (const t of A.data.threads) if (!t.synthetic && t.unread) unread++;
    const unreadFrac = A.data.real ? unread / A.data.real : 0;
    A.unreadOn = A.f.unread == null ? unreadFrac <= 0.4 : !!A.f.unread;
    A.stats = { hits, needs, total: A.data.real, query: !!toks.length, unread, unreadFrac };
    if (dropSel) A.sel = null;
    A.emit(silent || layoutChanged ? 'data' : 'filter');
    if (dropSel) A.emit('select', null, 'filter');
  };
  A.setFilter = (patch) => { Object.assign(A.f, patch); A.applyFilter(false); };

  // ---- selection / hover:  {kind: 'place'|'effort'|'thread', id}
  const same = (a, b) => (!a && !b) || (a && b && a.kind === b.kind && a.id === b.id);
  A.select = (s, src) => { if (same(A.sel, s)) { if (s) A.emit('select', s, src); return; } A.sel = s; A.emit('select', s, src); };
  A.setHover = (h) => { if (same(A.hover, h)) return; A.hover = h; A.emit('hover', h); };
  A.focusObj = () => A.resolve(A.hover) ? A.hover : A.sel;
  // which thread a selection stands for
  A.threadOf = (s) => { const o = A.resolve(s); if (!o) return null; return s.kind === 'effort' ? o.root : s.kind === 'thread' ? o : null; };
  A.kindOfThread = (t) => A.kindOf(t);
  A.ageDays = (t) => (Date.now() - t.e) / DAY;
})();
