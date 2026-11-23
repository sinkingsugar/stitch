/* River: time on x, places as lanes, efforts as bars (spawned threads thinner, tucked under). */
(function () {
  'use strict';
  const A = window.Atlas;
  const V = (A.views.river = { name: 'river' });
  const host = document.getElementById('v-river');
  const canvas = host.querySelector('canvas');
  const ctx = canvas.getContext('2d');
  const DAY = A.DAY, HOUR = 3600000;
  const FONT = A.FONT;

  let W = 0, H = 0, dpr = 1, topOff = 56, labelW = 130, miniH = 54;
  let lanes = [], blocks = [], contentH = 0;
  let t0 = 0, t1 = 0; // data extent (ms)
  let view = { left: 0, ppm: 1 }; // left time, pixels per ms
  let panY = 0, dirty = true, active = false, hoverB = null, pendingSel = null, inited = false;
  const hits = [];
  let miniBins = [], miniMax = 1, lastTime = 'all';

  const barH = (cost) => (cost <= 0 ? 6 : cost < 0.1 ? 7 : cost < 0.5 ? 9 : cost < 2 ? 12 : cost < 8 ? 15 : 19);
  const THIN = 3, GAP = 1.5;

  // ---------------------------------------------------------------- layout
  function build() {
    const D = A.data;
    lanes = []; blocks = [];
    const vp = A.visPlaces ? A.visPlaces() : D.places;
    if (!vp.length) { contentH = 0; miniBins = []; t0 = Math.floor(Date.now() / DAY) * DAY - DAY; return; }
    t0 = Infinity; t1 = Date.now();
    for (const p of vp) for (const e of A.visEfforts(p)) t0 = Math.min(t0, e.s);
    t0 = Math.floor(t0 / DAY) * DAY;
    const places = vp.slice().sort((a, b) => b.e - a.e || (a.key < b.key ? -1 : 1));
    const MINX = 5 * HOUR, PADX = 2.5 * HOUR; // minimum footprint, in time, so packing is zoom independent
    let y = 0;
    for (const p of places) {
      const rows = []; // each: {end, h}
      const lane = { p, y, h: 0, blocks: [] };
      for (const e of A.visEfforts(p).slice().sort((a, b) => a.s - b.s)) {
        const bar = barH(e.cost);
        // children packed into thin sub-rows
        const kids = e.threads.filter((t) => t !== e.root).sort((a, b) => a.s - b.s);
        const sub = []; const placedKids = [];
        for (const t of kids) {
          let r = sub.findIndex((end) => end <= t.s);
          if (r < 0) { if (sub.length < 4) { r = sub.length; sub.push(0); } else { r = 0; for (let i = 1; i < sub.length; i++) if (sub[i] < sub[r]) r = i; } }
          sub[r] = Math.max(t.e, t.s + MINX * 0.4);
          placedKids.push({ t, row: r });
        }
        const h = bar + (sub.length ? 2 + sub.length * (THIN + GAP) : 0) + 4;
        const s = e.s, en = Math.max(e.e, e.s + MINX), ext = en + PADX;
        let ri = rows.findIndex((r) => r.end <= s);
        if (ri < 0) { ri = rows.length; rows.push({ end: 0, h: 0 }); }
        rows[ri].end = ext; rows[ri].h = Math.max(rows[ri].h, h);
        const b = { e, s, en: Math.max(e.e, e.s), bar, h, row: ri, kids: placedKids, lane, subRows: sub.length };
        lane.blocks.push(b); blocks.push(b);
      }
      let ry = 0;
      for (const r of rows) { r.y = ry; ry += r.h; }
      for (const b of lane.blocks) b.y = rows[b.row].y;
      lane.h = Math.max(ry, 12) + 14;
      lane.y = y; y += lane.h;
      lanes.push(lane);
    }
    contentH = y;
    // minimap bins: per day, activity = threads whose last step falls on it
    const nd = Math.ceil((t1 - t0) / DAY) + 1;
    miniBins = Array.from({ length: nd }, () => ({ n: 0, need: 0, work: 0 }));
    for (const t of D.threads) {
      if (t.synthetic || !t.effort || !t.effort.vis) continue;
      const i = Math.min(nd - 1, Math.max(0, Math.floor((t.e - t0) / DAY)));
      miniBins[i].n++; if (t.need) miniBins[i].need++; if (t.working) miniBins[i].work++;
    }
    miniMax = Math.max(1, ...miniBins.map((b) => b.n));
  }

  // ---------------------------------------------------------------- view math
  const xOf = (t) => labelW + (t - view.left) * view.ppm;
  const tOf = (x) => view.left + (x - labelW) / view.ppm;
  function fitAll() {
    const span = Math.max(DAY, Date.now() - t0 + DAY * 0.6);
    view.ppm = (W - labelW - 14) / span; view.left = t0 - DAY * 0.0;
  }
  function clampView() {
    const minPpm = (W - labelW - 14) / (Date.now() - t0 + DAY * 1.5);
    view.ppm = A.clamp(view.ppm, minPpm * 0.999, 1 / 8000);
    const now = Date.now();
    view.left = A.clamp(view.left, t0 - 3 * DAY, now + DAY * 0.3 - (W - labelW) / view.ppm * 0.5);
    const maxPan = Math.max(0, contentH - (H - topOff - miniH - 16));
    panY = A.clamp(panY, -maxPan, 0);
  }
  function zoomAt(f, cx) {
    const tt = tOf(cx);
    view.ppm *= f; clampView();
    view.left = tt - (cx - labelW) / view.ppm; clampView(); dirty = true;
  }
  function centerOn(b) {
    // bring a block into view, animated
    const np = Math.max(view.ppm, 400 / (6 * DAY), Math.min(1 / 8000, 0.55 * (W - labelW - 330 * (W > 700 ? 1 : 0)) / Math.max(b.en - b.s, 3 * HOUR)));
    const target = { left: (b.s + b.en) / 2 - (W - labelW - 330 * (W > 700 ? 1 : 0)) / 2 / np, panY: -(b.lane.y + b.y) + (H - topOff - miniH) / 2 - 20 };
    if (view.ppm < 1 / (3 * DAY / 120)) { /* too far out: zoom to about 3 days per 120px */ }
    animateTo(target, np);
  }
  let anim = null;
  function animateTo(target, ppm) {
    const from = { left: view.left, ppm: view.ppm, panY };
    const st = performance.now(), dur = /instant/.test(location.search) ? 1 : 700;
    const maxPan = Math.max(0, contentH - (H - topOff - miniH - 16));
    const toLeft = target.left, toPan = A.clamp(target.panY, -maxPan, 0);
    // keep the centre stable: interpolate the centre time and ppm (log)
    const cw = (W - labelW) / 2, cFrom = from.left + cw / from.ppm, cTo = toLeft + cw / ppm;
    anim = (now) => {
      let t = A.clamp((now - st) / dur, 0, 1); t = t * t * (3 - 2 * t);
      view.ppm = Math.exp(A.lerp(Math.log(from.ppm), Math.log(ppm), t));
      const c = A.lerp(cFrom, cTo, t);
      view.left = c - cw / view.ppm; panY = A.lerp(from.panY, toPan, t);
      dirty = true;
      if (t >= 1) anim = null;
    };
  }

  // ---------------------------------------------------------------- drawing
  function draw() {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, W, H);
    hits.length = 0;
    const now = Date.now();
    const top = topOff, bot = H - miniH - 14;
    ctx.save();
    ctx.beginPath(); ctx.rect(labelW, top, W - labelW, bot - top); ctx.clip();
    drawGrid(top, bot, now);
    ctx.save(); ctx.translate(0, top + panY);
    ctx.textBaseline = 'middle';
    for (const lane of lanes) {
      const ly = lane.y;
      if (ly + lane.h + panY + top < top || ly + panY + top > bot) continue;
      ctx.fillStyle = 'rgba(255,255,255,.025)';
      ctx.fillRect(labelW, ly + lane.h - 1, W, 1);
      for (const b of lane.blocks) drawBlock(b, ly + 6, now);
    }
    ctx.restore();
    // now line
    const nx = xOf(now);
    ctx.globalAlpha = 1;
    ctx.strokeStyle = 'rgba(232,243,255,.55)'; ctx.lineWidth = 1; ctx.setLineDash([3, 4]);
    ctx.beginPath(); ctx.moveTo(nx, top); ctx.lineTo(nx, bot); ctx.stroke(); ctx.setLineDash([]);
    ctx.restore();
    // lane labels column
    ctx.save();
    ctx.beginPath(); ctx.rect(0, top, labelW, bot - top); ctx.clip();
    const g = ctx.createLinearGradient(0, 0, labelW, 0);
    g.addColorStop(0, 'rgba(10,14,21,1)'); g.addColorStop(0.9, 'rgba(10,14,21,.96)'); g.addColorStop(1, 'rgba(10,14,21,.0)');
    ctx.fillStyle = g; ctx.fillRect(0, top, labelW + 6, bot - top);
    ctx.textBaseline = 'middle'; ctx.textAlign = 'left';
    for (const lane of lanes) {
      const y = top + panY + lane.y;
      if (y + lane.h < top || y > bot) continue;
      const f = lane.p.fade;
      ctx.globalAlpha = 0.3 + 0.7 * f;
      ctx.font = `600 12px ${FONT}`; ctx.fillStyle = '#d9e2ee';
      const hasN = lane.p.need > 0;
      const name = A.ellipsize(lane.p.name, 12, labelW - 24 - (hasN ? 14 : 0));
      ctx.fillText(name, 12, y + 15);
      if (hasN) { ctx.beginPath(); ctx.arc(14 + A.tw(name) * 12 + 5, y + 15, 3.6, 0, 6.3); ctx.fillStyle = '#fb923c'; ctx.fill(); }
      ctx.font = `400 10.5px ${FONT}`; ctx.fillStyle = '#6f7d91';
      ctx.fillText(lane.p.efforts.length + ' efforts · ' + lane.p.count, 12, y + 30);
    }
    ctx.restore();
    ctx.globalAlpha = 1;
    drawAxis(top, now);
    drawMini(now);
    if (hoverB) { const hb = hoverB; /* tooltip for titles that do not fit */ drawTip(hb); }
  }

  function drawGrid(top, bot, now) {
    const pxDay = DAY * view.ppm;
    const tl = tOf(labelW), tr = tOf(W);
    // week bands (Monday start)
    let d = new Date(tl); d.setHours(0, 0, 0, 0);
    d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
    ctx.globalAlpha = 1;
    for (let wk = d.getTime(), i = 0; wk < tr; i++) {
      const nx = new Date(wk); nx.setDate(nx.getDate() + 7);
      if (Math.round(wk / (7 * DAY)) % 2 === 0) { ctx.fillStyle = 'rgba(255,255,255,.022)'; ctx.fillRect(xOf(wk), top, xOf(nx.getTime()) - xOf(wk), bot - top); }
      wk = nx.getTime();
    }
    // day lines
    const dd = new Date(tl); dd.setHours(0, 0, 0, 0);
    ctx.lineWidth = 1;
    for (let t = dd.getTime(); t < tr; ) {
      const x = Math.round(xOf(t)) + 0.5;
      const day = new Date(t), wk = day.getDay() === 1;
      if (pxDay > 14 || wk) { ctx.strokeStyle = wk ? 'rgba(255,255,255,.09)' : 'rgba(255,255,255,.045)'; ctx.beginPath(); ctx.moveTo(x, top); ctx.lineTo(x, bot); ctx.stroke(); }
      if (pxDay > 480) { for (let h = 6; h < 24; h += 6) { const hx = Math.round(xOf(t + h * HOUR)) + 0.5; ctx.strokeStyle = 'rgba(255,255,255,.025)'; ctx.beginPath(); ctx.moveTo(hx, top); ctx.lineTo(hx, bot); ctx.stroke(); } }
      const nxt = new Date(day); nxt.setDate(nxt.getDate() + 1); t = nxt.getTime();
    }
  }
  function drawAxis(top, now) {
    const pxDay = DAY * view.ppm;
    const tl = tOf(labelW), tr = tOf(W);
    ctx.save();
    ctx.beginPath(); ctx.rect(labelW, 0, W - labelW, top + 2); ctx.clip();
    ctx.textAlign = 'left'; ctx.textBaseline = 'alphabetic';
    const dd = new Date(tl); dd.setHours(0, 0, 0, 0);
    let lastLabelX = -1e9;
    for (let t = dd.getTime(); t < tr; ) {
      const day = new Date(t);
      const x = xOf(t);
      const step = pxDay >= 44 ? 1 : 0;
      const isMon = day.getDay() === 1;
      if ((step || isMon) && x > labelW - 2 && x - lastLabelX > 34) {
        ctx.fillStyle = isMon ? '#9fb0c6' : '#6b788c'; ctx.font = `${isMon ? 600 : 500} 11px ${FONT}`;
        const lab = pxDay >= 70 ? A.WD[day.getDay()] + ' ' + day.getDate() : pxDay >= 44 ? A.WD[day.getDay()].slice(0, 2) + day.getDate() : A.MON[day.getMonth()] + ' ' + day.getDate();
        ctx.fillText(lab, x + 4, top - 8); lastLabelX = x;
        if (pxDay > 480) { ctx.fillStyle = '#566173'; ctx.font = `400 10px ${FONT}`; for (let h = 6; h < 24; h += 6) ctx.fillText(String(h).padStart(2, '0') + ':00', xOf(t + h * HOUR) + 3, top - 8); }
      }
      const nxt = new Date(day); nxt.setDate(nxt.getDate() + 1); t = nxt.getTime();
    }
    const nx = xOf(now);
    ctx.fillStyle = '#e8f3ff'; ctx.font = `600 10.5px ${FONT}`; ctx.textAlign = 'right';
    ctx.fillText('now', Math.min(W - 6, nx - 5), top - 22 + 12 - 10);
    ctx.restore();
  }

  function focusEffort() {
    for (const s of [hoverB && { kind: hoverB.kind, id: hoverB.id, b: hoverB.b }, A.sel]) {
      if (!s) continue;
      if (s.b) return s.b.e.id;
      if (s.kind === 'effort') return s.id;
      if (s.kind === 'thread') { const t = A.data.byId.get(s.id); if (t) return t.rootId; }
    }
    return null;
  }
  function drawBlock(b, oy, now) {
    const e = b.e;
    const x0 = xOf(b.s), x1 = Math.max(xOf(b.en), x0 + 4);
    if (x1 < labelW - 4 || x0 > W + 4) return;
    const y = oy + b.y, f = e.fade;
    const kind = A.kindOf(e), sty = A.style(kind, (now - e.e) / DAY);
    const w = x1 - x0, h = b.bar;
    const fid = focusEffort(), isFocus = fid === e.id, dimmed = fid && !isFocus;
    ctx.globalAlpha = f * (dimmed ? 0.4 : 1);
    // connectors + thin bars
    if (b.kids.length) {
      const ky = y + h + 2;
      const rowOf = new Map(); for (const k of b.kids) rowOf.set(k.t.id, k.row);
      for (const k of b.kids) {
        const t = k.t, kx0 = xOf(t.s), kx1 = Math.max(xOf(t.e), kx0 + 3);
        if (kx1 < labelW || kx0 > W) continue;
        const yy = ky + k.row * (THIN + GAP);
        const ks = A.style(A.kindOf(t), (now - t.e) / DAY);
        ctx.globalAlpha = Math.min(f, t.fade) * 0.9 * (dimmed ? 0.4 : 1);
        // connector from the parent at spawn time: faint when zoomed in; for the hovered / selected effort always, with every level
        if (isFocus) {
          const pr = rowOf.get(t.parent);
          const py = pr == null ? y + h : ky + pr * (THIN + GAP) + THIN;
          const cx = Math.round(kx0) + 0.5, strong = t.need || t.working;
          ctx.strokeStyle = t.need ? 'rgba(251,146,60,.95)' : t.working ? 'rgba(45,212,191,.95)' : 'rgba(205,224,255,.6)';
          ctx.lineWidth = strong ? 1.6 : 1.1;
          ctx.beginPath(); ctx.moveTo(cx, py); ctx.lineTo(cx, yy + THIN / 2); ctx.lineTo(cx + 3, yy + THIN / 2); ctx.stroke();
          ctx.beginPath(); ctx.arc(cx, py, 1.6, 0, 6.3); ctx.fillStyle = ctx.strokeStyle; ctx.fill();
        } else if (DAY * view.ppm > 60) { ctx.strokeStyle = 'rgba(150,170,200,.16)'; ctx.lineWidth = 1; ctx.beginPath(); ctx.moveTo(Math.round(kx0) + 0.5, y + h); ctx.lineTo(Math.round(kx0) + 0.5, yy + THIN / 2); ctx.stroke(); }
        ctx.fillStyle = ks.fill; ctx.beginPath(); ctx.roundRect(kx0, yy, kx1 - kx0, THIN, 1.5); ctx.fill();
        if (t.need) { ctx.fillStyle = '#fb923c'; ctx.fillRect(kx1 - 2, yy - 1, 3, THIN + 2); }
        if (t.unread && A.unreadOn) { ctx.strokeStyle = 'rgba(170,195,230,.3)'; ctx.lineWidth = 0.7; ctx.strokeRect(kx0 - 0.5, yy - 0.5, kx1 - kx0 + 1, THIN + 1); }
        hits.push({ x: kx0, y: oy + 0 + (yy - oy) + 0, w: kx1 - kx0, h: THIN + GAP, kind: 'thread', id: t.id, b, py: yy });
      }
    }
    ctx.globalAlpha = f * (dimmed ? 0.4 : 1);
    if (e.working) { ctx.fillStyle = 'rgba(45,212,191,.16)'; ctx.beginPath(); ctx.roundRect(x0 - 2, y - 2, w + 4, h + 4, h / 2 + 2); ctx.fill(); }
    ctx.fillStyle = sty.fill; ctx.beginPath(); ctx.roundRect(x0, y, w, h, Math.min(h / 2, 7)); ctx.fill();
    ctx.strokeStyle = sty.stroke; ctx.lineWidth = 1; if (e.root.synthetic) ctx.setLineDash([4, 3]); ctx.stroke(); ctx.setLineDash([]);
    if (e.unread && A.unreadOn) { ctx.strokeStyle = 'rgba(170,195,230,.3)'; ctx.lineWidth = 0.8; ctx.beginPath(); ctx.roundRect(x0 - 2, y - 2, w + 4, h + 4, Math.min(h / 2, 7) + 2); ctx.stroke(); }
    if (e.need) { ctx.fillStyle = '#fb923c'; ctx.beginPath(); ctx.arc(Math.min(x1, W) - 1, y + h / 2, 3.2, 0, 6.3); ctx.fill(); ctx.strokeStyle = '#0a0e15'; ctx.lineWidth = 1.2; ctx.stroke(); }
    // title when it fits
    const vx0 = Math.max(x0, labelW) + 6;
    if (h >= 11 && x1 - vx0 > 50) {
      const size = h >= 15 ? 11.5 : 10.5;
      const text = A.ellipsize(e.root.title || e.id, size, Math.min(x1, W) - vx0 - 8);
      if (text && text.length > 2) { ctx.font = `600 ${size}px ${FONT}`; ctx.textAlign = 'left'; ctx.fillStyle = sty.text; ctx.fillText(text, vx0, y + h / 2 + 0.5); }
    }
    hits.push({ x: x0, y, w, h: h + 2, kind: 'effort', id: e.id, b, py: y });
    if (hoverB && hoverB.id === e.id && hoverB.kind === 'effort' || (A.sel && A.sel.id === e.id && A.sel.kind === 'effort')) {
      ctx.globalAlpha = 1; ctx.strokeStyle = '#fff'; ctx.lineWidth = 2; ctx.beginPath(); ctx.roundRect(x0 - 1, y - 1, w + 2, h + 2, Math.min(h / 2, 7) + 1); ctx.stroke();
    }
    if (A.sel && A.sel.kind === 'thread') {
      const k = b.kids.find((q) => q.t.id === A.sel.id);
      if (k) { ctx.globalAlpha = 1; ctx.strokeStyle = '#fff'; ctx.lineWidth = 1.5; const yy = y + h + 2 + k.row * (THIN + GAP); ctx.strokeRect(xOf(k.t.s) - 1, yy - 1, Math.max(xOf(k.t.e) - xOf(k.t.s), 3) + 2, THIN + 2); }
    }
  }
  function drawTip(hb) {
    const e = A.data.eById.get(hb.b.e.id); if (!e) return;
    const t = hb.kind === 'thread' ? A.data.byId.get(hb.id) : e.root;
    if (!t) return;
    const x = A.clamp(hb.x + hb.w / 2, labelW + 10, W - 10), y = topOff + panY + hb.py - 8;
    const text = A.ellipsize(t.title || t.id, 12, 300);
    ctx.font = `600 12px ${FONT}`; ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
    const w = A.tw(text) * 12 + 16;
    const rx = A.clamp(x - w / 2, labelW + 4, W - w - 4), ry = Math.max(topOff + 2, y - 22);
    ctx.globalAlpha = 1; ctx.fillStyle = 'rgba(10,14,22,.94)'; ctx.beginPath(); ctx.roundRect(rx, ry, w, 20, 10); ctx.fill();
    ctx.strokeStyle = 'rgba(255,255,255,.2)'; ctx.lineWidth = 1; ctx.stroke();
    ctx.fillStyle = '#eef3fa'; ctx.fillText(text, rx + w / 2, ry + 10.5);
  }

  function miniGeo() { return { x0: labelW, x1: W - 14, y0: H - miniH - 4, y1: H - 6 }; }
  function drawMini(now) {
    const g = miniGeo(), w = g.x1 - g.x0, h = g.y1 - g.y0 - 12;
    const span = Math.max(DAY, now - t0 + DAY * 0.6);
    ctx.globalAlpha = 1; ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
    ctx.fillStyle = 'rgba(255,255,255,.03)'; ctx.beginPath(); ctx.roundRect(g.x0, g.y0, w, g.y1 - g.y0, 8); ctx.fill();
    ctx.font = `500 10px ${FONT}`; ctx.fillStyle = '#566173'; ctx.fillText('activity per day', 12, g.y0 + 12);
    const bw = w / span * DAY;
    for (let i = 0; i < miniBins.length; i++) {
      const b = miniBins[i], x = g.x0 + (t0 + i * DAY - t0) / span * w;
      const bh = Math.max(b.n ? 2 : 0, (b.n / miniMax) * h);
      ctx.fillStyle = 'rgba(111,134,168,.55)'; ctx.fillRect(x + 0.5, g.y1 - 4 - bh, Math.max(1, bw - 1), bh);
      if (b.need) { const nh = Math.max(2, (b.need / miniMax) * h); ctx.fillStyle = '#fb923c'; ctx.fillRect(x + 0.5, g.y1 - 4 - nh, Math.max(1, bw - 1), nh); }
      if (b.work) { ctx.fillStyle = '#2dd4bf'; ctx.fillRect(x + 0.5, g.y1 - 4 - Math.max(2, (b.work / miniMax) * h), Math.max(1, bw - 1), 2); }
    }
    // brush
    const bx0 = g.x0 + (tOf(labelW) - t0) / span * w, bx1 = g.x0 + (tOf(W) - t0) / span * w;
    const l = A.clamp(bx0, g.x0, g.x1), r = A.clamp(bx1, g.x0, g.x1);
    ctx.fillStyle = 'rgba(232,243,255,.08)'; ctx.fillRect(l, g.y0, Math.max(3, r - l), g.y1 - g.y0);
    ctx.strokeStyle = 'rgba(232,243,255,.5)'; ctx.lineWidth = 1; ctx.strokeRect(l + 0.5, g.y0 + 0.5, Math.max(3, r - l) - 1, g.y1 - g.y0 - 1);
  }

  // ---------------------------------------------------------------- interaction (own gestures: x zoom, 2-axis pan, pinch)
  const ptrs = new Map();
  let dragMode = null, down = null, lastPinch = null;
  function pos(e) { const r = canvas.getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; }
  function inMini(p) { const g = miniGeo(); return p.y >= g.y0 - 4 && p.x >= g.x0 - 4 && p.x <= g.x1 + 4; }
  function miniJump(p) {
    const g = miniGeo(), span = Math.max(DAY, Date.now() - t0 + DAY * 0.6);
    const t = t0 + (A.clamp(p.x, g.x0, g.x1) - g.x0) / (g.x1 - g.x0) * span;
    anim = null; view.left = t - (W - labelW) / 2 / view.ppm; clampView(); dirty = true;
  }
  function hitAt(p) {
    for (let i = hits.length - 1; i >= 0; i--) {
      const h = hits[i];
      const hh = h.kind === 'thread' ? Math.max(h.h, 6) : h.h;
      const yy = topOff + panY + h.py;
      if (p.x >= h.x - 1 && p.x <= h.x + h.w + 1 && p.y >= yy - (h.kind === 'thread' ? 2 : 1) && p.y <= yy + hh) return h;
    }
    return null;
  }
  function onDown(e) {
    canvas.setPointerCapture && canvas.setPointerCapture(e.pointerId);
    const p = pos(e); ptrs.set(e.pointerId, p); anim = null;
    if (ptrs.size === 1) { down = { x: p.x, y: p.y, moved: false, mini: inMini(p) }; dragMode = down.mini ? 'mini' : 'pan'; if (down.mini) miniJump(p); }
    else { dragMode = 'pinch'; const [a, b] = [...ptrs.values()]; lastPinch = { d: Math.hypot(a.x - b.x, a.y - b.y), cx: (a.x + b.x) / 2 }; }
  }
  function onMove(e) {
    const p = pos(e);
    if (!ptrs.has(e.pointerId)) { // hover
      if (e.pointerType === 'touch') return;
      const h = inMini(p) ? null : hitAt(p);
      setHover(h);
      return;
    }
    const prev = ptrs.get(e.pointerId); ptrs.set(e.pointerId, p);
    if (dragMode === 'mini') { miniJump(p); return; }
    if (dragMode === 'pinch' && ptrs.size >= 2) {
      const [a, b] = [...ptrs.values()], d = Math.hypot(a.x - b.x, a.y - b.y), cx = (a.x + b.x) / 2;
      if (lastPinch && lastPinch.d > 0) zoomAt(d / lastPinch.d, cx);
      if (lastPinch) { view.left -= (cx - lastPinch.cx) / view.ppm; clampView(); }
      lastPinch = { d, cx }; dirty = true; if (down) down.moved = true; return;
    }
    if (dragMode === 'pan') {
      const dx = p.x - prev.x, dy = p.y - prev.y;
      if (down && (Math.abs(p.x - down.x) > 4 || Math.abs(p.y - down.y) > 4)) down.moved = true;
      if (down && down.moved) { view.left -= dx / view.ppm; panY += dy; clampView(); dirty = true; }
    }
  }
  function onUp(e) {
    const p = pos(e);
    ptrs.delete(e.pointerId);
    if (ptrs.size < 2) lastPinch = null;
    if (ptrs.size === 0) {
      if (down && !down.moved && dragMode === 'pan') {
        const h = hitAt(p);
        if (h) A.select({ kind: h.kind, id: h.id }, 'river'); else if (!inMini(p)) A.select(null, 'river');
      }
      down = null; dragMode = null;
    }
  }
  function onWheel(e) {
    e.preventDefault();
    const p = pos(e);
    if (e.shiftKey && !e.ctrlKey) { panY -= e.deltaY || e.deltaX; clampView(); dirty = true; return; }
    if (Math.abs(e.deltaX) > Math.abs(e.deltaY) * 1.2 && !e.ctrlKey) { view.left += e.deltaX / view.ppm; clampView(); dirty = true; return; }
    const dy = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
    zoomAt(Math.exp(-dy * (e.ctrlKey ? 0.01 : 0.0016)), p.x); anim = null;
  }
  function setHover(h) {
    const cur = hoverB;
    if ((!h && !cur) || (h && cur && h.id === cur.id && h.kind === cur.kind)) { if (h) hoverB = h; return; }
    hoverB = h; dirty = true;
    canvas.style.cursor = h ? 'pointer' : 'default';
    A.setHover(h ? { kind: h.kind, id: h.id } : null);
  }

  // ---------------------------------------------------------------- sizing + loop
  function resize() {
    const r = host.getBoundingClientRect();
    if (!r.width || !r.height) return;
    const bar = document.getElementById('bar');
    topOff = (bar ? bar.offsetHeight : 48) + 26;
    const nw = Math.round(r.width), nh = Math.round(r.height);
    dpr = Math.min(window.devicePixelRatio || 1, 3);
    labelW = nw < 700 ? 84 : 132;
    miniH = nw < 700 ? 40 : 52;
    const first = !W;
    W = nw; H = nh;
    canvas.width = Math.round(W * dpr); canvas.height = Math.round(H * dpr);
    if (first || !inited) { fitAll(); inited = true; }
    clampView(); dirty = true;
  }
  function frame(ts) {
    requestAnimationFrame(frame);
    if (!active) return;
    if (anim) anim(ts);
    if (dirty) { dirty = false; draw(); }
  }
  function blockOf(sel) {
    if (!sel) return null;
    const id = sel.kind === 'thread' ? (A.data.byId.get(sel.id) || {}).rootId : sel.kind === 'effort' ? sel.id : null;
    if (sel.kind === 'place') { const l = lanes.find((x) => x.p.key === sel.id); return l && l.blocks[l.blocks.length - 1]; }
    return blocks.find((b) => b.e.id === id) || null;
  }
  V.show = () => { document.body.classList.add('rv'); active = true; host.classList.add('on'); resize(); if (pendingSel) { const b = blockOf(pendingSel); pendingSel = null; if (b) centerOn(b); } dirty = true; };
  V.hide = () => { active = false; document.body.classList.remove('rv'); };

  function init() {
    canvas.addEventListener('pointerdown', onDown);
    canvas.addEventListener('pointermove', onMove);
    canvas.addEventListener('pointerup', onUp);
    canvas.addEventListener('pointercancel', onUp);
    canvas.addEventListener('pointerleave', () => { if (!ptrs.size) setHover(null); });
    canvas.addEventListener('wheel', onWheel, { passive: false });
    new ResizeObserver(resize).observe(host);
    A.on('data', () => {
      const tc = A.f.time !== lastTime; lastTime = A.f.time;
      build(); if ((!inited || tc) && W) { fitAll(); inited = true; panY = 0; }
      clampView(); dirty = true;
    });
    A.on('filter', () => { dirty = true; });
    A.on('hover', () => { dirty = true; });
    A.on('select', (s, src) => {
      dirty = true;
      if (s && src !== 'river') { if (active) { const b = blockOf(s); if (b) centerOn(b); } else pendingSel = s; }
    });
    requestAnimationFrame(frame);
    V.debug = { get panY() { return panY; }, get contentH() { return contentH; }, get view() { return view; }, set ppm(v) { view.ppm = v; }, zoomAt, get lanes() { return lanes; } };
  }
  init();
})();
