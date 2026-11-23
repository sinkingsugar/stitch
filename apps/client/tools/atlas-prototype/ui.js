/* Shared chrome: tabs, search, time filter, needs toggle, detail card, loading. Plain classic script; runs last. */
(function () {
  'use strict';
  const A = window.Atlas;
  const $ = (id) => document.getElementById(id);
  const card = $('card'), overlay = $('overlay');
  let current = 'islands';
  let loadTimer = null, loading = false;

  // ---------------------------------------------------------------- tabs
  function showView(name) {
    if (!A.views[name] && name === 'space') {
      const m = $('space-msg');
      m.hidden = false; m.textContent = A.spaceFailed ? '3D is unavailable here (' + A.spaceFailed + '). Islands and River still work.' : 'Loading 3D…';
    }
    for (const k of Object.keys(A.views)) if (k !== name && A.views[k].hide) A.views[k].hide();
    current = name; A.current = name;
    document.querySelectorAll('#tabs button').forEach((b) => b.classList.toggle('on', b.dataset.view === name));
    document.querySelectorAll('.view').forEach((v) => v.classList.toggle('on', v.id === 'v-' + name));
    $('topdown').hidden = name !== 'space' || !A.views.space;
    if (A.views[name] && A.views[name].show) A.views[name].show();
    $('hint').textContent = { islands: 'scroll / pinch to zoom · drag to pan · click an island to dive in · click empty space to go back', river: 'scroll to zoom time · drag to pan · bottom strip jumps', space: 'drag to orbit · scroll to zoom · height = recency' }[name] || '';
  }
  A.showView = showView;

  // ---------------------------------------------------------------- card
  function stateDot(o) { return '<i class="dot ' + (o.need ? 'need' : o.working ? 'work' : 'idle') + '" style="margin-right:6px"></i>'; }
  function stateLabel(t) { return t.need ? (t.question ? 'needs you' : 'blocked') : t.state || 'idle'; }
  // The spawned tree of an effort: every level, indented, with the count of what hangs under a row; folded to the
  // first 30 rows with "show all" (kept per effort while the page lives).
  const TREE_FOLD = 30, expanded = new Set();
  const subCount = (t) => t.kids.reduce((n, k) => n + 1 + subCount(k), 0);
  function treeHtml(root, curId, eff) {
    const all = [];
    (function walk(t, depth) { all.push([t, depth]); t.kids.forEach((k) => walk(k, depth + 1)); })(root, 0);
    const open = expanded.has(root.id), rows = open ? all : all.slice(0, TREE_FOLD);
    const place = eff ? eff.place : null;
    let h = rows.map(([t, d]) => {
      const n = t.kids.length ? subCount(t) : 0;
      const away = place && !t.synthetic && t.placeKey !== place ? `<span class="tag" title="${A.esc(t.placeKey)}">${A.esc(A.placeName(t.placeKey))}</span>` : '';
      return `<div class="row${t.id === curId ? ' cur' : ''}" data-id="${A.esc(t.id)}" style="padding-left:${6 + Math.min(d, 12) * 14}px">${stateDot(t)}<span class="t">${A.esc(t.title || t.id)}</span>${away}${n ? `<span class="n">${n}</span>` : ''}</div>`;
    }).join('');
    if (all.length > TREE_FOLD) h += `<div class="row more" data-all="${A.esc(root.id)}">${open ? 'show fewer' : `show all ${all.length}`}</div>`;
    return h;
  }
  function placeCard(p) {
    const needs = p.efforts.filter((e) => e.need).slice(0, 6);
    const recent = p.efforts.slice().sort((a, b) => b.e - a.e).slice(0, 8);
    const li = (e) => `<div class="row" data-eff="${A.esc(e.id)}">${stateDot(e)}<span class="t">${A.esc(e.root.title || e.id)}</span></div>`;
    return `<h3>${A.esc(p.name)}</h3><div class="meta">${A.esc(p.key)}</div>
      <div class="meta">${p.efforts.length} efforts<span class="sep">·</span>${p.count} threads<span class="sep">·</span>${A.money(p.cost)}${p.working ? `<span class="sep">·</span>${p.working} working` : ''}${p.need ? `<span class="sep">·</span><b style="color:var(--orange)">${p.need} need you</b>` : ''}</div>
      <div class="meta">${A.fmtTime(p.s)} → ${A.ago(p.e)}</div>
      ${needs.length ? `<div class="lbl">Needs you</div><div class="tree">${needs.map(li).join('')}</div>` : ''}
      <div class="lbl">Recent</div><div class="tree">${recent.map(li).join('')}</div>`;
  }
  const depthOf = (t) => (t.kids.length ? 1 + t.kids.reduce((m, k) => Math.max(m, depthOf(k)), 0) : 0);
  function threadCard(t, sel) {
    const e = t.effort;
    if (t.synthetic) return `<h3>${A.esc(t.title)}</h3><div class="meta">${A.esc(A.placeName(t.placeKey))}<span class="sep">·</span>${t.kids.length} threads name a parent this listing does not have</div><div class="goal">Their parent was deleted, lives on a repository this node does not hold, or the node is older than <code>conversations</code>.</div>${e && e.threads.length > 1 ? `<div class="lbl">Threads · ${e.threads.length - 1}</div><div class="tree">${treeHtml(t, null, e)}</div>` : ''}`;
    const head = (t.digest && t.digest.headline) || t.last_words || '';
    const goal = String(t.goal || '').trim().split('\n').slice(0, 4).join('\n');
    const where = [A.esc(A.placeName(t.placeKey)), A.esc(t.node || '')].filter(Boolean).join('<span class="sep">·</span>');
    const unread = t.unread ? '<span class="sep">·</span><span style="color:var(--ring)">unread</span>' : '';
    return `<h3>${A.esc(t.title || t.id)}</h3>
      <div class="meta">${where}<span class="sep">·</span>${stateDot(t)}${A.esc(stateLabel(t))}<span class="sep">·</span>${A.money(t.cost)}${unread}</div>
      <div class="meta">${A.fmtTime(t.s)} → ${A.fmtTime(t.e)} <span style="opacity:.6">(${A.ago(t.e)})</span></div>
      ${t.question ? `<div class="head" style="color:var(--orange)">Asks: ${A.esc(t.question.command || 'approval')}</div>` : ''}
      ${head ? `<div class="head">${A.esc(head)}</div>` : ''}
      ${goal && goal !== head ? `<div class="goal">${A.esc(goal)}</div>` : ''}
      ${e && e.threads.length > 1 ? `<div class="lbl">${e.root.synthetic ? 'Threads whose parent is not loaded' : 'Spawned threads'} · ${e.threads.length - 1}<span style="opacity:.6"> · depth ${depthOf(e.root)}</span></div><div class="tree">${treeHtml(e.root, t.id, e)}</div>` : ''}
      <div class="id"><code>${A.esc(t.id)}</code><button data-copy="${A.esc(t.id)}">copy</button></div>`;
  }
  let lastKey = '';
  function renderCard() {
    const s = A.focusObj();
    const o = A.resolve(s);
    if (!o) { card.hidden = true; lastKey = ''; return; }
    const key = s.kind + s.id + (s === A.hover ? 'h' : 's') + A.data.now + (A.sel ? A.sel.id : '');
    if (key === lastKey && !card.hidden) return;
    lastKey = key;
    card.hidden = false;
    const body = s.kind === 'place' ? placeCard(o) : threadCard(s.kind === 'effort' ? o.root : o, s);
    card.innerHTML = '<button class="x" aria-label="close">×</button>' + body;
  }
  card.addEventListener('click', (ev) => {
    const t = ev.target;
    if (t.closest('.x')) { A.select(null, 'ui'); return; }
    const cp = t.closest('[data-copy]');
    if (cp) { try { navigator.clipboard.writeText(cp.dataset.copy); cp.textContent = 'copied'; setTimeout(() => (cp.textContent = 'copy'), 1200); } catch (e) { cp.textContent = '—'; } return; }
    const more = t.closest('.row[data-all]');
    if (more) { const id = more.dataset.all; if (expanded.has(id)) expanded.delete(id); else expanded.add(id); lastKey = ''; renderCard(); return; }
    const r = t.closest('.row[data-id]');
    if (r) { const th = A.data.byId.get(r.dataset.id); A.select({ kind: th.rootId === th.id ? 'effort' : 'thread', id: th.id }, 'card'); return; }
    const re = t.closest('.row[data-eff]');
    if (re) A.select({ kind: 'effort', id: re.dataset.eff }, 'card');
  });

  // ---------------------------------------------------------------- loading
  function showError(e) {
    overlay.hidden = false;
    const consent = e && e.consent;
    overlay.innerHTML = `<div style="font-size:16px;font-weight:600">${consent ? 'Atlas needs your permission to read threads' : 'Could not load threads'}</div>
      <div style="color:var(--fg2);max-width:520px">${A.esc((e && e.message) || String(e))}</div>
      ${e && e.allow ? `<pre>${A.esc(e.allow)}</pre>` : ''}<button id="retry">Retry</button>`;
    $('retry').onclick = () => { overlay.hidden = true; load(); };
  }
  // One call for the whole forest (conversations too, up to 5000 rows). A node older than `conversations` ignores the
  // option and gives spawned threads only (and at most 500): no row with detached:false says so, and the old call is made.
  const OLD_CALL = { all: true, limit: 2000 };
  async function fetchRows(prog) {
    prog = prog || {};
    const live = window.stitch && window.stitch.live;
    if (live && live.threads) {
      let resp = null;
      if (prog.status) prog.status('loading threads…');
      try { resp = await live.threads({ all: true, conversations: true, limit: 5000 }); }
      catch (e) { if (e && (e.consent || /permission|allow/i.test(String(e.message)))) throw e; resp = null; }
      // (An older node's `all` listing has a few non-detached rows too, assignments' threads: the whole forest is
      // either past the old 500 cap or complete.)
      if (resp && (resp.threads || []).some((r) => r && r.detached === false) && ((resp.threads || []).length > 500 || !resp.more)) return resp;
      // A node between the two: `every` lists conversations too (500 a call) with `places`; read each place by itself.
      try {
        if (prog.status) prog.status('loading places…');
        const first = await live.threads({ all: true, every: true, limit: 500 });
        if (first && (first.threads || []).some((r) => r && r.detached === false) && Array.isArray(first.places)) {
          const seen = new Map();
          for (const r of first.threads) seen.set(r.id, r);
          const todo = first.places.map((p) => p.place);
          const total = todo.length;
          let missing = 0, done = 0;
          if (prog.partial) prog.partial({ node: first.node, threads: [...seen.values()], more: 1 });
          if (prog.status) prog.status('loading places 0/' + total + '…', 0, total);
          const worker = async () => {
            while (todo.length) {
              const place = todo.shift();
              try {
                const r = await live.threads({ all: true, every: true, place, limit: 500 });
                for (const t of r.threads || []) seen.set(t.id, t);
                missing += r.more || 0;
              } catch (e) { console.warn('place', place, e); }
              done++;
              if (prog.status) prog.status('loading places ' + done + '/' + total + '…', done, total);
            }
          };
          await Promise.all([worker(), worker(), worker(), worker()]);
          return { node: first.node, threads: [...seen.values()], more: missing };
        }
      } catch (e) { console.warn(e); }
      if (!resp) resp = await live.threads(OLD_CALL);
      resp.legacy = true;
      return resp;
    }
    const q = new URLSearchParams(location.search).get('data') || 'sample.json';
    const r = await fetch(q);
    if (!r.ok) throw new Error('sample load failed: ' + r.status);
    const resp = await r.json();
    if (!(resp.threads || []).some((x) => x && x.detached === false)) resp.legacy = true;
    return resp;
  }
  // progress: before the first data an overlay with a bar; after a partial render a pill in the top bar
  let shown = false;
  function status(text, done, total) {
    const pill = $('prog');
    if (!shown) {
      overlay.hidden = false;
      overlay.innerHTML = `<div class="spin"></div><div style="font-size:15px;font-weight:600">${A.esc(text)}</div>${total ? `<div class="pbar"><i style="width:${Math.round(100 * done / total)}%"></i></div>` : ''}`;
    }
    if (pill) { pill.hidden = !shown; pill.textContent = text; }
  }
  async function load() {
    if (loading) return;
    loading = true; shown = !!(A.data && A.data.threads && A.data.threads.length);
    const pill = $('prog');
    try {
      const resp = await fetchRows({
        status,
        partial: (r) => { overlay.hidden = true; shown = true; A.setData(r); updateStat(); testHooks(); },
      });
      overlay.hidden = true; A.setData(resp); updateStat(); testHooks();
    }
    catch (e) { console.error(e); showError(e); }
    finally { loading = false; if (pill) pill.hidden = true; }
  }
  let hooked = false;
  function testHooks() { // dev only: ?focus=<place>&eff=<n th largest effort in it>
    if (hooked) return; hooked = true;
    const tm = /[?&]time=(\w+)/.exec(location.search); if (tm) { const b = document.querySelector('#time button[data-t="' + tm[1] + '"]'); if (b) b.click(); }
    const m = /[?&]focus=([^&]+)/.exec(location.search); if (!m) return;
    const p = A.data.places.find((x) => x.name === decodeURIComponent(m[1]));
    const e = /[?&]eff=(\d+)/.exec(location.search);
    if (p && e) { const ef = p.efforts.slice().sort((a, b) => b.threads.length - a.threads.length)[+e[1]]; A.select({ kind: 'effort', id: ef.id }, 'test'); }
    else if (p) A.select({ kind: 'place', id: p.key }, 'test');
  }
  function updateStat() {
    const s = A.stats || {};
    const d = A.data;
    $('stat').textContent = d.real + ' threads · ' + (d.efforts.length - d.efforts.filter((e) => e.root.synthetic).length) + ' efforts · ' + (A.nPlacesVis != null && A.nPlacesVis !== d.places.length ? A.nPlacesVis + ' of ' + d.places.length : d.places.length) + ' places' + (d.orphans ? ' · ' + d.orphans + ' with parent not loaded' : '') + (d.legacy ? ' · spawned threads only (this node\'s build predates conversations)' : '') + (d.more ? ' (more not loaded)' : '') + (s.query ? ' · ' + s.hits + ' match' : '');
    $('needsN').textContent = s.needs || '';
    const u = $('unread'); u.classList.toggle('on', !!A.unreadOn);
    $('unreadN').textContent = d.real ? Math.round(100 * (s.unread || 0) / d.real) + '%' : '';
    u.title = A.unreadOn ? 'Showing unread marks (quiet outlines). Click to hide.' : (s.unreadFrac > 0.4 ? 'Most threads are unread, so the marks are off. Click to show.' : 'Show unread marks');
  }

  // ---------------------------------------------------------------- wiring
  function wire() {
    document.querySelectorAll('#tabs button').forEach((b) => b.addEventListener('click', () => showView(b.dataset.view)));
    let qt = null;
    $('q').addEventListener('input', (e) => { clearTimeout(qt); qt = setTimeout(() => A.setFilter({ q: e.target.value }), 110); });
    $('q').addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        const hit = A.data.efforts.filter((x) => x.fade >= 0.99).sort((a, b) => b.e - a.e)[0];
        if (hit && A.f.q) A.select({ kind: 'effort', id: hit.id }, 'search');
      }
    });
    document.querySelectorAll('#time button').forEach((b) => b.addEventListener('click', () => {
      document.querySelectorAll('#time button').forEach((x) => x.classList.toggle('on', x === b));
      A.setFilter({ time: b.dataset.t });
    }));
    $('unread').addEventListener('click', () => { A.setFilter({ unread: !A.unreadOn }); });
    $('needs').addEventListener('click', () => { const on = !A.f.needs; $('needs').classList.toggle('on', on); A.setFilter({ needs: on }); });
    window.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') { A.select(null, 'ui'); if (document.activeElement === $('q')) $('q').blur(); }
      else if (e.key === '/' && document.activeElement !== $('q')) { e.preventDefault(); $('q').focus(); }
    });
    A.on('select', renderCard); A.on('hover', renderCard); A.on('data', () => { lastKey = ''; renderCard(); updateStat(); }); A.on('filter', updateStat);
    $('topdown').addEventListener('click', () => { if (A.views.space && A.views.space.topDown) A.views.space.topDown(); });
    A.on('space-ready', () => { if (current === 'space') showView('space'); });
    if (window.stitch && window.stitch.live && window.stitch.live.on) {
      window.stitch.live.on('threads', () => { clearTimeout(loadTimer); loadTimer = setTimeout(load, 1500); });
    }
    const m = /[?&]view=(islands|river|space)/.exec(location.search);
    showView(m ? m[1] : 'islands');
    load();
  }
  wire();
})();
