/* Space: the spawn tree is the structure.  x/z: each effort sits where Islands packs it, its conversation root is the big node and
   the spawned threads hang around it on a radial layout (children around their parent, radius by depth); height = recency on a
   log scale.  Three.js, in its own module so a WebGL failure leaves 2D alone.  Nodes are instanced points, edges are instanced
   screen-space quads (fat lines without any addon), labels are sprites placed greedily in screen space. */
import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';

const A = window.Atlas;
const host = document.getElementById('v-space').querySelector('#space-host');
const view = document.getElementById('v-space');
const HGT = 330, S = 1.3, NSEG = 8, TAU0 = 0.25; // height, xz scale of the pack, curve segments, log-time knee (days)
let SPAN = 35;
const fT = (age) => Math.log1p(Math.max(0, age) / TAU0) / Math.log1p(SPAN / TAU0);
const yOf = (ageDays) => HGT * (1 - Math.min(1, fT(ageDays)));

let renderer, scene, camera, controls, pts, ring, selRing, edgeMesh, rootGroup, labelGroup, tip;
let active = false, built = false, dirtyData = true, userTouched = false, tween = null, hovered = -1, running = false, labelDirty = true, camKey = '';
let items = [], effs = [], posArr = null, edgeN = 0, edgeList = [];
const uniforms = { uScale: { value: 800 }, uTime: { value: 0 }, uFogN: { value: 700 }, uFogF: { value: 3200 }, uRing: { value: 0 }, uRes: { value: new THREE.Vector2(1, 1) } };
const V = (A.views.space = { name: 'space' });
const labelCache = new Map();

const VERT = `attribute float size; attribute vec3 color; attribute float alpha; attribute float flag;
uniform float uScale, uTime, uFogN, uFogF; varying vec3 vC; varying float vA;
void main(){ vec4 mv = modelViewMatrix * vec4(position,1.0); float d = -mv.z;
 float s = size * (1.0 + flag * 0.2 * sin(uTime*2.2 + position.x*0.1));
 gl_PointSize = clamp(s * uScale / d, 2.0, 120.0); gl_Position = projectionMatrix * mv; vC = color;
 float fog = 1.0 - smoothstep(uFogN, uFogF, d); vA = alpha * (0.35 + 0.65*fog); }`;
// uRing: 0 disc, 1 quiet hairline ring (unread), 2 strong ring (selection)
const FRAG = `varying vec3 vC; varying float vA; uniform float uRing;
void main(){ vec2 p = gl_PointCoord*2.0-1.0; float r = length(p); if (r>1.0) discard; float a;
 if (uRing > 1.5) { a = smoothstep(0.86,0.9,r)*(1.0-smoothstep(0.95,1.0,r))*0.9; }
 else if (uRing > 0.5) { a = smoothstep(0.90,0.94,r)*(1.0-smoothstep(0.96,1.0,r))*0.32; }
 else { a = pow(1.0-r,1.8)*0.5 + (1.0-smoothstep(0.38,0.62,r))*0.55; }
 gl_FragColor = vec4(vC, a*vA); }`;
const EVERT = `attribute vec3 pa; attribute vec3 pb; attribute vec3 ecol; attribute vec4 eprop;
uniform vec2 uRes; uniform float uFogN, uFogF; varying vec3 vC; varying float vA; varying float vE;
void main(){
 vec4 ca = modelViewMatrix*vec4(pa,1.0), cb = modelViewMatrix*vec4(pb,1.0);
 float nr = 2.0;
 if (ca.z > -nr && cb.z > -nr) { gl_Position = vec4(2.0,2.0,2.0,1.0); return; }
 if (ca.z > -nr) ca = ca + (cb-ca)*((-nr-ca.z)/(cb.z-ca.z));
 if (cb.z > -nr) cb = cb + (ca-cb)*((-nr-cb.z)/(ca.z-cb.z));
 vec4 qa = projectionMatrix*ca, qb = projectionMatrix*cb;
 vec2 sa = qa.xy/qa.w*uRes*0.5, sb = qb.xy/qb.w*uRes*0.5;
 vec2 d = sb-sa; float l = length(d); d = l > 1e-4 ? d/l : vec2(1.0,0.0);
 vec2 nrm = vec2(-d.y, d.x);
 vec4 c = mix(qa, qb, position.x);
 c.xy += nrm * position.y * (eprop.y / uRes) * c.w;
 gl_Position = c;
 float dist = -mix(ca.z, cb.z, position.x);
 float along = mix(eprop.z, eprop.w, position.x);
 vC = ecol; vE = position.y;
 vA = eprop.x * (0.6 + 0.4*along) * (0.4 + 0.6*(1.0 - smoothstep(uFogN, uFogF, dist))); }`;
const EFRAG = `varying vec3 vC; varying float vA; varying float vE;
void main(){ float e = 1.0 - smoothstep(0.5, 1.0, abs(vE)); gl_FragColor = vec4(vC, vA*e); }`;

function mat(ring) {
  const u = Object.assign({}, uniforms, { uRing: { value: ring } });
  return new THREE.ShaderMaterial({ uniforms: u, vertexShader: VERT, fragmentShader: FRAG, transparent: true, depthWrite: false, depthTest: false, blending: THREE.AdditiveBlending });
}

function labelSprite(text, px, opts = {}) {
  const c = document.createElement('canvas'), dpr = 2;
  const ctx = c.getContext('2d');
  const font = `${opts.weight || 600} ${px}px ${A.FONT}`;
  ctx.font = font;
  const w = Math.ceil(ctx.measureText(text).width) + 24, h = Math.ceil(px * 1.6);
  c.width = w * dpr; c.height = h * dpr;
  ctx.scale(dpr, dpr); ctx.font = font; ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
  ctx.lineJoin = 'round'; ctx.lineWidth = 5; ctx.strokeStyle = 'rgba(8,11,17,.9)'; ctx.strokeText(text, w / 2, h / 2 + 1);
  ctx.fillStyle = opts.color || '#e6ebf2'; ctx.fillText(text, w / 2, h / 2 + 1);
  const tex = new THREE.CanvasTexture(c); tex.anisotropy = 4; tex.colorSpace = THREE.SRGBColorSpace;
  const sp = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true, depthWrite: false, depthTest: !!opts.depth, opacity: opts.opacity || 1 }));
  const k = (opts.height || 14) / h;
  sp.scale.set(w * k, h * k, 1);
  sp.userData.aspect = w / h;
  return sp;
}
function glowTexture() {
  const c = document.createElement('canvas'); c.width = c.height = 256;
  const g = c.getContext('2d'), gr = g.createRadialGradient(128, 128, 0, 128, 128, 128);
  gr.addColorStop(0, 'rgba(45,212,191,.05)'); gr.addColorStop(0.75, 'rgba(45,212,191,.025)'); gr.addColorStop(1, 'rgba(45,212,191,0)');
  g.fillStyle = gr; g.fillRect(0, 0, 256, 256);
  return new THREE.CanvasTexture(c);
}
function disposeGroup(g) {
  g.traverse((o) => { if (o.geometry) o.geometry.dispose(); if (o.material) { if (o.material.map) o.material.map.dispose(); o.material.dispose(); } });
}

function initRenderer() {
  if (renderer) return true;
  try {
    renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance', preserveDrawingBuffer: /shot/.test(location.search) });
  } catch (e) { fail(e.message || 'WebGL unavailable'); return false; }
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.setClearColor(0x080b11, 1);
  host.appendChild(renderer.domElement);
  scene = new THREE.Scene();
  camera = new THREE.PerspectiveCamera(48, 1, 1, 9000);
  camera.position.set(980, 620, 1020);
  controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true; controls.dampingFactor = 0.08;
  controls.target.set(0, HGT * 0.42, 0);
  controls.autoRotate = true; controls.autoRotateSpeed = 0.35;
  controls.maxPolarAngle = Math.PI * 0.62; controls.minDistance = 20; controls.maxDistance = 4500;
  controls.addEventListener('start', () => { userTouched = true; controls.autoRotate = false; tween = null; });
  rootGroup = new THREE.Group(); scene.add(rootGroup);
  labelGroup = new THREE.Group(); scene.add(labelGroup);
  tip = document.createElement('div');
  tip.style.cssText = 'position:absolute;pointer-events:none;padding:3px 9px;border-radius:9px;background:rgba(10,14,22,.92);border:1px solid rgba(255,255,255,.18);font:600 12px ' + A.FONT + ';color:#eef3fa;white-space:nowrap;display:none;max-width:320px;overflow:hidden;text-overflow:ellipsis;z-index:4';
  view.appendChild(tip);
  const el = renderer.domElement;
  let dn = null;
  el.addEventListener('pointerdown', (e) => { dn = { x: e.clientX, y: e.clientY, t: performance.now() }; });
  el.addEventListener('pointerup', (e) => { if (dn && Math.hypot(e.clientX - dn.x, e.clientY - dn.y) < 5) click(e); dn = null; });
  el.addEventListener('pointermove', (e) => { if (e.buttons || e.pointerType === 'touch') return; hover(e); });
  el.addEventListener('pointerleave', () => { setHover(-1); });
  new ResizeObserver(resize).observe(host);
  return true;
}
function fail(msg) { A.spaceFailed = msg; const m = document.getElementById('space-msg'); m.hidden = false; m.textContent = '3D is unavailable here (' + msg + '). Islands and River still work.'; }

function resize() {
  if (!renderer) return;
  const w = host.clientWidth, h = host.clientHeight;
  if (!w || !h) return;
  renderer.setSize(w, h);
  camera.aspect = w / h; camera.updateProjectionMatrix();
  uniforms.uScale.value = renderer.domElement.height / (2 * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2)));
  uniforms.uRes.value.set(renderer.domElement.width, renderer.domElement.height);
  for (const o of [pts, ring, selRing]) if (o) o.material.uniforms.uScale.value = uniforms.uScale.value;
  if (edgeMesh) edgeMesh.material.uniforms.uRes.value.copy(uniforms.uRes.value);
  labelDirty = true;
}

// ---------------------------------------------------------------- the radial tree layout
// Children hang around their parent in the horizontal plane: the root's children on a full circle, deeper ones in a wedge
// pointing away from their own parent, radius shrinking with depth.  Deterministic: kids are already ordered by start time.
const subSize = new Map();
function sizeOf(t) { let n = 1; for (const k of t.kids) n += sizeOf(k); subSize.set(t.id, n); return n; }
function layoutEffort(e, cx, cz, rPack) {
  const out = new Map();
  const root = e.root;
  sizeOf(root);
  const n = e.threads.filter((t) => !t.synthetic).length;
  const base = Math.max(rPack * S * 0.5, 9 + 5.6 * Math.sqrt(Math.max(0, n - 1)));
  const hash = (id) => { let h = 2166136261; for (let i = 0; i < id.length; i++) h = Math.imul(h ^ id.charCodeAt(i), 16777619); return (h >>> 0) / 4294967296; };
  (function walk(t, x, z, depth, theta) {
    out.set(t.id, { x, z, depth });
    const kids = t.kids; const k = kids.length; if (!k) return;
    const rho = base * Math.pow(0.6, depth) * (0.72 + 0.28 * Math.min(1, k / 6));
    const full = depth === 0;
    const wedge = full ? Math.PI * 2 : Math.min(Math.PI * 1.7, Math.PI * 0.5 + 0.2 * Math.PI * k);
    const off = full ? hash(root.id) * 6.283 : 0;
    // biggest subtrees in the middle of the wedge so they have the room
    const order = kids.map((c, i) => i).sort((a, b) => subSize.get(kids[b].id) - subSize.get(kids[a].id));
    const slots = new Array(k); const mid = (k - 1) / 2;
    order.forEach((ci, r) => { const slot = Math.round(mid + (r % 2 ? 1 : -1) * Math.ceil(r / 2)); slots[ci] = Math.min(k - 1, Math.max(0, slot)); });
    const used = new Set(); // resolve collisions of the rounding
    for (let i = 0; i < k; i++) { let s = slots[i]; while (used.has(s)) s = (s + 1) % k; used.add(s); slots[i] = s; }
    for (let i = 0; i < k; i++) {
      const frac = full ? slots[i] / k : (k === 1 ? 0.5 : slots[i] / (k - 1));
      const th = full ? off + frac * wedge : theta + (frac - 0.5) * wedge;
      const sz = subSize.get(kids[i].id);
      const r = rho * (0.82 + 0.18 * Math.min(1, Math.sqrt(sz) / 4)) * (1 + (slots[i] % 2 ? 0.1 : -0.06));
      walk(kids[i], x + Math.cos(th) * r, z + Math.sin(th) * r, depth + 1, th);
    }
  })(root, cx, cz, 0, 0);
  return out;
}

// ---------------------------------------------------------------- scene from data
function colorFor(t, now) {
  const age = (now - t.e) / A.DAY, dim = Math.min(1, age / 14);
  if (t.need) return [1.0, 0.6, 0.26, 1];
  if (t.working) return [0.2, 0.9, 0.8, 1];
  const b = 1 - 0.7 * dim;
  return [0.46 * b + 0.04, 0.57 * b + 0.05, 0.76 * b + 0.06, 1 - 0.35 * dim];
}
const ECOL = (t, rec) => (t.need ? [1, 0.58, 0.2] : t.working ? [0.18, 0.92, 0.8] : [0.5 + 0.2 * rec, 0.64 + 0.18 * rec, 0.88]);

function build() {
  if (!renderer || !A.layout) return;
  dirtyData = false;
  while (rootGroup.children.length) { const c = rootGroup.children.pop(); disposeGroup(c); }
  while (labelGroup.children.length) labelGroup.children.pop();
  for (const v of labelCache.values()) disposeGroup(v.sp);
  labelCache.clear();
  const now = Date.now(), L = A.layout, D = A.data;
  items = []; effs = []; edgeList = [];
  const vp = A.visPlaces ? A.visPlaces() : D.places;
  // time scale: the column covers the age of the oldest visible thread
  let maxAge = 7;
  for (const p of vp) for (const e of A.visEfforts(p)) for (const t of e.threads) if (!t.synthetic) maxAge = Math.max(maxAge, (now - t.e) / A.DAY);
  SPAN = A.clamp(maxAge * 1.04, 8, 120);
  let wmax = 0;
  for (const t of D.threads) if (!t.synthetic) wmax = Math.max(wmax, t.w);
  const idx = new Map();
  for (const p of vp) for (const e of A.visEfforts(p)) {
    const ep = L.pos.get(e.id); if (!ep) continue;
    const cx = (ep.x - 500) * S, cz = (ep.y - 500) * S;
    const lay = layoutEffort(e, cx, cz, ep.r);
    const real = e.threads.filter((t) => !t.synthetic);
    const n = real.length;
    const rec = { e, n, items: [], edges: [], score: 0, ext: 0, cx, cz };
    for (const t of e.threads) {
      if (t.synthetic) continue;
      const q = lay.get(t.id); if (!q) continue;
      const isRoot = t.id === e.root.id;
      const wn = Math.pow(t.w / wmax, 0.3);
      const size = isRoot ? 10 + 3.6 * Math.log2(n + 1) + 7 * wn : 4.5 + 9 * wn;
      idx.set(t.id, items.length); rec.items.push(items.length);
      items.push({ t, x: q.x, y: yOf((now - t.e) / A.DAY), z: q.z, depth: q.depth, eff: rec, isRoot, size, parent: -1 });
    }
    rec.rootIdx = idx.get(e.root.id);
    effs.push(rec);
  }
  for (const it of items) { const p = it.t.parent && !it.t.cycleCut ? idx.get(it.t.parent) : null; if (p != null && items[p].eff === it.eff) it.parent = p; }
  for (const r of effs) {
    let ext = 0; const c = r.items.length;
    for (const i of r.items) ext = Math.max(ext, Math.hypot(items[i].x - r.cx, items[i].z - r.cz));
    r.ext = ext;
    const rt = r.e.root, age = (now - r.e.e) / A.DAY;
    r.score = Math.log2(r.n + 1) / 7 + 0.7 * (1 - Math.min(1, fT(age))) + (r.e.need ? 0.6 : 0) + (r.e.working ? 0.4 : 0);
    r.cy = r.items.reduce((m, i) => m + items[i].y, 0) / Math.max(1, c);
  }
  const n = items.length;
  posArr = new Float32Array(n * 3);
  const col = new Float32Array(n * 3), size = new Float32Array(n), alpha = new Float32Array(n), flag = new Float32Array(n);
  const rpos = [], rsize = [];
  items.forEach((it, i) => {
    posArr.set([it.x, it.y, it.z], i * 3);
    const c = colorFor(it.t, now);
    col.set(c.slice(0, 3), i * 3); size[i] = it.size * (it.t.need ? 1.25 : 1); flag[i] = it.t.working ? 1 : 0;
    it.baseAlpha = c[3];
    if (it.t.unread) { rpos.push(it.x, it.y, it.z); rsize.push(it.size * 1.5 + 3); }
  });
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.BufferAttribute(posArr, 3));
  g.setAttribute('color', new THREE.BufferAttribute(col, 3));
  g.setAttribute('size', new THREE.BufferAttribute(size, 1));
  g.setAttribute('alpha', new THREE.BufferAttribute(alpha, 1));
  g.setAttribute('flag', new THREE.BufferAttribute(flag, 1));
  pts = new THREE.Points(g, mat(0)); pts.frustumCulled = false; pts.renderOrder = 2; rootGroup.add(pts);
  const rg = new THREE.BufferGeometry();
  const rn = rsize.length;
  rg.setAttribute('position', new THREE.Float32BufferAttribute(rpos, 3));
  rg.setAttribute('color', new THREE.Float32BufferAttribute(new Array(rn * 3).fill(0.62).map((v, i) => (i % 3 === 2 ? 0.85 : v)), 3));
  rg.setAttribute('size', new THREE.Float32BufferAttribute(rsize, 1));
  rg.setAttribute('alpha', new THREE.Float32BufferAttribute(new Array(rn).fill(1), 1));
  rg.setAttribute('flag', new THREE.Float32BufferAttribute(new Array(rn).fill(0), 1));
  ring = new THREE.Points(rg, mat(1)); ring.frustumCulled = false; ring.renderOrder = 3; rootGroup.add(ring);
  const sg = new THREE.BufferGeometry();
  sg.setAttribute('position', new THREE.Float32BufferAttribute([0, -9999, 0], 3));
  sg.setAttribute('color', new THREE.Float32BufferAttribute([1, 1, 1], 3));
  sg.setAttribute('size', new THREE.Float32BufferAttribute([30], 1));
  sg.setAttribute('alpha', new THREE.Float32BufferAttribute([1], 1));
  sg.setAttribute('flag', new THREE.Float32BufferAttribute([0], 1));
  selRing = new THREE.Points(sg, mat(2)); selRing.frustumCulled = false; selRing.renderOrder = 4; rootGroup.add(selRing);
  buildEdges();
  // ground, time levels, today layer
  const R = 560 * S;
  const grid = new THREE.GridHelper(R * 2.1, 21, 0x24303f, 0x1a2330); grid.material.transparent = true; grid.material.opacity = 0.5; rootGroup.add(grid);
  const levels = [[0, 'today', 0x2dd4bf], [1, 'yesterday'], [3, '3 days'], [7, '1 week'], [14, '2 weeks'], [28, '4 weeks'], [60, '2 months']];
  for (const [d, name, c] of levels) {
    if (d > SPAN) continue;
    const y = yOf(d), pts2 = [];
    for (let i = 0; i <= 128; i++) { const a = i / 128 * Math.PI * 2; pts2.push(new THREE.Vector3(Math.cos(a) * R, y, Math.sin(a) * R)); }
    const lg2 = new THREE.BufferGeometry().setFromPoints(pts2);
    rootGroup.add(new THREE.Line(lg2, new THREE.LineBasicMaterial({ color: c || 0x3a4960, transparent: true, opacity: d === 0 ? 0.35 : 0.24 })));
    const sp = labelSprite(name, 22, { weight: 500, color: d === 0 ? '#7ee8d8' : '#7f8da3', height: 14, opacity: 0.9 });
    sp.position.set(R + 44, y, 0); rootGroup.add(sp);
  }
  const glow = new THREE.Mesh(new THREE.CircleGeometry(R * 1.02, 64), new THREE.MeshBasicMaterial({ map: glowTexture(), transparent: true, depthWrite: false, blending: THREE.AdditiveBlending, side: THREE.DoubleSide }));
  glow.rotation.x = -Math.PI / 2; glow.position.y = yOf(0) - 1; rootGroup.add(glow);
  // places: base ring, faint column, labels at base and top
  for (const p of vp) {
    const pl = L.places.get(p.key); if (!pl) continue;
    const cx = (pl.x - 500) * S, cz = (pl.y - 500) * S, pr = pl.r * S;
    let top = 0;
    for (const e of A.visEfforts(p)) top = Math.max(top, yOf((now - e.e) / A.DAY));
    const circ = [];
    for (let i = 0; i <= 64; i++) { const a = i / 64 * Math.PI * 2; circ.push(new THREE.Vector3(cx + Math.cos(a) * pr, 0.5, cz + Math.sin(a) * pr)); }
    rootGroup.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints(circ), new THREE.LineBasicMaterial({ color: p.need ? 0xfb923c : 0x6f86a8, transparent: true, opacity: p.need ? 0.38 : 0.22 })));
    if (pr > 18) rootGroup.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(cx, 0, cz), new THREE.Vector3(cx, top, cz)]), new THREE.LineBasicMaterial({ color: 0x6f86a8, transparent: true, opacity: 0.1 })));
    const h = A.clamp(pr * 0.2, 8, 30);
    const base = labelSprite(p.name, 28, { height: h, color: '#dbe4f0', opacity: 0.92 });
    base.position.set(cx, 3, cz); rootGroup.add(base);
  }
  built = true; applyFade();
  syncSel();
}

// fat curved edges: NSEG instanced quads each; per-effort ranges so a focus can restyle just those
function buildEdges() {
  edgeList = [];
  edgeList = [];
  items.forEach((it, i) => { if (it.parent >= 0) edgeList.push({ c: i, p: it.parent, eff: it.eff }); });
  edgeN = edgeList.length;
  const inst = edgeN * NSEG;
  const pa = new Float32Array(inst * 3), pb = new Float32Array(inst * 3), ec = new Float32Array(inst * 3), ep = new Float32Array(inst * 4);
  const hashS = (i) => (Math.imul(i + 1, 2654435761) >>> 0) / 4294967296;
  const q = new THREE.Vector3();
  edgeList.forEach((ed, ei) => {
    const a = items[ed.p], b = items[ed.c];
    ed.start = ei * NSEG;
    const mx = (a.x + b.x) / 2, my = (a.y + b.y) / 2, mz = (a.z + b.z) / 2;
    const dx = b.x - a.x, dz = b.z - a.z, len = Math.hypot(dx, dz) + Math.abs(b.y - a.y) * 0.3 + 1e-3;
    const side = hashS(ei) < 0.5 ? -1 : 1;
    // control point: bulge sideways in the horizontal plane and a little up so edges arc over their siblings
    const cxp = mx + (-dz / (Math.hypot(dx, dz) + 1e-3)) * len * 0.16 * side, czp = mz + (dx / (Math.hypot(dx, dz) + 1e-3)) * len * 0.16 * side, cyp = my + len * 0.06;
    const pt = (t, out) => { const u = 1 - t; out[0] = u * u * a.x + 2 * u * t * cxp + t * t * b.x; out[1] = u * u * a.y + 2 * u * t * cyp + t * t * b.y; out[2] = u * u * a.z + 2 * u * t * czp + t * t * b.z; };
    const o0 = [0, 0, 0], o1 = [0, 0, 0];
    for (let s = 0; s < NSEG; s++) {
      const k = ed.start + s; pt(s / NSEG, o0); pt((s + 1) / NSEG, o1);
      pa.set(o0, k * 3); pb.set(o1, k * 3);
      ep[k * 4 + 2] = s / NSEG; ep[k * 4 + 3] = (s + 1) / NSEG;
    }
    ed.rec = items[ed.c].eff;
  });
  const geo = new THREE.InstancedBufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array([0, -1, 0, 1, -1, 0, 1, 1, 0, 0, 1, 0]), 3));
  geo.setIndex([0, 1, 2, 0, 2, 3]);
  geo.setAttribute('pa', new THREE.InstancedBufferAttribute(pa, 3));
  geo.setAttribute('pb', new THREE.InstancedBufferAttribute(pb, 3));
  geo.setAttribute('ecol', new THREE.InstancedBufferAttribute(ec, 3));
  geo.setAttribute('eprop', new THREE.InstancedBufferAttribute(ep, 4));
  geo.instanceCount = inst;
  const m = new THREE.ShaderMaterial({ uniforms: { uRes: { value: uniforms.uRes.value.clone() }, uFogN: uniforms.uFogN, uFogF: uniforms.uFogF }, vertexShader: EVERT, fragmentShader: EFRAG, transparent: true, depthWrite: false, depthTest: false });
  edgeMesh = new THREE.Mesh(geo, m); edgeMesh.frustumCulled = false; edgeMesh.renderOrder = 1; rootGroup.add(edgeMesh);
}
function focusEffort() {
  const fo = A.focusObj(); if (!fo) return null;
  if (fo.kind === 'effort') return fo.id;
  if (fo.kind === 'thread') { const t = A.data.byId.get(fo.id); return t && t.rootId; }
  return null;
}
function styleEdges(fid) {
  if (!edgeMesh) return;
  const now = Date.now(), ge = edgeMesh.geometry.attributes;
  const ec = ge.ecol.array, ep = ge.eprop.array;
  const dpr = renderer.getPixelRatio();
  for (const ed of edgeList) {
    const t = items[ed.c].t, rec = A.clamp(1 - (now - t.e) / (A.DAY * 10), 0, 1);
    const strong = t.need || t.working;
    let al = strong ? 0.95 : 0.42 + 0.45 * rec, w = strong ? 2.8 : 1.9 + 0.6 * rec;
    const foc = fid && ed.rec.e.id === fid;
    if (foc) { al = Math.min(1, al * 1.5 + 0.2); w *= 1.55; } else if (fid) al *= 0.16;
    al *= Math.max(0.12, t.fade);
    const c = ECOL(t, rec);
    for (let s = 0; s < NSEG; s++) {
      const k = ed.start + s;
      ec[k * 3] = c[0]; ec[k * 3 + 1] = c[1]; ec[k * 3 + 2] = c[2];
      ep[k * 4] = al; ep[k * 4 + 1] = w * dpr;
    }
  }
  ge.ecol.needsUpdate = true; ge.eprop.needsUpdate = true;
}
let lastFid = undefined;
function applyFade() {
  if (!pts) return;
  const fid = focusEffort();
  const a = pts.geometry.attributes.alpha;
  items.forEach((it, i) => (a.array[i] = it.baseAlpha * Math.max(0.12, it.t.fade) * (fid && it.eff.e.id !== fid ? 0.3 : 1)));
  a.needsUpdate = true;
  if (ring) ring.visible = !!A.unreadOn;
  styleEdges(fid); lastFid = fid; labelDirty = true;
}
function syncSel() {
  if (!selRing) return;
  const s = A.sel, a = selRing.geometry.attributes.position;
  let it = null;
  if (s && s.kind !== 'place') { const id = s.id; it = items.find((x) => x.t.id === id); }
  if (it) { a.setXYZ(0, it.x, it.y, it.z); selRing.geometry.attributes.size.array[0] = it.size * 1.25 + 5; } else a.setXYZ(0, 0, -9999, 0);
  a.needsUpdate = true; selRing.geometry.attributes.size.needsUpdate = true;
}

// ---------------------------------------------------------------- labels (sprites, collision-avoided in screen space)
const NLAB = 25;
function labelOf(it, big) {
  let v = labelCache.get(it.t.id);
  if (!v) {
    const full = it.t.title || it.t.id, txt = full.length > 38 ? full.slice(0, 37).trimEnd() + '…' : full;
    const sp = labelSprite(txt, it.isRoot ? 26 : 24, { weight: it.isRoot ? 650 : 500, color: it.isRoot ? '#f2f6fc' : '#c8d3e3', opacity: 1 });
    sp.renderOrder = 10; sp.visible = false; labelGroup.add(sp);
    v = { sp, shown: false }; labelCache.set(it.t.id, v);
  }
  return v;
}
const _lp = new THREE.Vector3();
function placeLabels() {
  if (!renderer || !items.length) return;
  const cw = host.clientWidth, ch = host.clientHeight, pr = renderer.getPixelRatio();
  const f = uniforms.uScale.value / pr;
  const fid = focusEffort();
  camera.updateMatrixWorld();
  const cands = [];
  if (fid) {
    const rec = effs.find((r) => r.e.id === fid);
    if (rec) {
      const ids = rec.items.slice().sort((a, b) => (items[b].isRoot - items[a].isRoot) || (items[b].t.w - items[a].t.w));
      ids.forEach((i, k) => { if (k < 70) cands.push(items[i]); });
    }
  } else {
    const top = effs.filter((r) => r.rootIdx != null && r.e.fade > 0.3).sort((a, b) => b.score - a.score).slice(0, 90);
    for (const r of top) cands.push(items[r.rootIdx]);
  }
  const placed = [];
  let shown = 0;
  const seen = new Set();
  const focusMode = !!fid;
  for (const it of cands) {
    if (!focusMode && shown >= NLAB) break;
    _lp.set(it.x, it.y, it.z).applyMatrix4(camera.matrixWorldInverse);
    const depth = -_lp.z; if (depth < 5) continue;
    const px = _lp.x / depth * f + cw / 2, py = -_lp.y / depth * f + ch / 2;
    if (px < 10 || px > cw - 10 || py < 70 || py > ch - 24) continue;
    const v = labelOf(it), hpx = it.isRoot ? 14 : 12, w = v.sp.userData.aspect * hpx - 4;
    const rad = it.size * uniforms.uScale.value / pr / depth / 2;
    const off = Math.min(26, rad * 0.75 + 4);
    const rect = { x: px - w / 2, y: py - off - hpx, w, h: hpx };
    if (placed.some((q) => rect.x < q.x + q.w && rect.x + rect.w > q.x && rect.y < q.y + q.h && rect.y + rect.h > q.y)) continue;
    placed.push(rect); shown++;
    v.sp.position.set(it.x, it.y, it.z);
    v.sp.scale.set(v.sp.userData.aspect * hpx * depth / f, hpx * depth / f, 1);
    v.sp.center.set(0.5, -off / hpx);
    v.sp.material.opacity = focusMode && !it.isRoot ? 0.92 : 1;
    v.sp.visible = true; seen.add(v);
  }
  for (const v of labelCache.values()) if (!seen.has(v)) v.sp.visible = false;
}

// ---------------------------------------------------------------- picking (projected, no raycast)
const _v = new THREE.Vector3();
function pickAt(cx, cy) {
  if (!pts) return -1;
  const r = renderer.domElement.getBoundingClientRect();
  let best = -1, bd = 1e9;
  camera.updateMatrixWorld();
  const pr = renderer.getPixelRatio();
  for (let i = 0; i < items.length; i++) {
    const it = items[i]; if (it.t.fade < 0.15) continue;
    _v.set(it.x, it.y, it.z).project(camera);
    if (_v.z > 1 || _v.z < -1) continue;
    const sx = (_v.x * 0.5 + 0.5) * r.width + r.left, sy = (-_v.y * 0.5 + 0.5) * r.height + r.top;
    const dist = Math.hypot(sx - cx, sy - cy);
    const pxr = Math.max(8, it.size * uniforms.uScale.value / pr / Math.max(1, camera.position.distanceTo(_v.set(it.x, it.y, it.z))) / 2);
    const score = dist / pxr;
    if (dist <= pxr + 3 && score < bd) { bd = score; best = i; }
  }
  return best;
}
const selOf = (it) => ({ kind: it.t.rootId === it.t.id ? 'effort' : 'thread', id: it.t.id });
function setHover(i, e) {
  if (i === hovered && !e) return;
  hovered = i;
  renderer.domElement.style.cursor = i >= 0 ? 'pointer' : 'grab';
  if (i < 0) { tip.style.display = 'none'; A.setHover(null); return; }
  const it = items[i];
  tip.textContent = it.t.title || it.t.id; tip.style.display = 'block';
  A.setHover(selOf(it));
}
function hover(e) {
  const i = pickAt(e.clientX, e.clientY);
  setHover(i, e);
  if (i >= 0) { const r = view.getBoundingClientRect(); tip.style.left = Math.min(r.width - 330, e.clientX - r.left + 14) + 'px'; tip.style.top = e.clientY - r.top + 14 + 'px'; }
}
function click(e) {
  const i = pickAt(e.clientX, e.clientY);
  if (i >= 0) { const it = items[i]; A.select(selOf(it), 'space'); flyToItem(it); }
  else A.select(null, 'space');
}

// ---------------------------------------------------------------- camera tweens
function easeTo(target, dist) {
  const dir = camera.position.clone().sub(controls.target).normalize();
  tweenTo(target.clone(), target.clone().add(dir.multiplyScalar(dist)), 900);
}
function tweenTo(target, pos, ms) {
  tween = { t0: performance.now(), ms, fp: camera.position.clone(), ft: controls.target.clone(), tp: pos, tt: target };
  controls.autoRotate = false;
}
function stepTween(now) {
  if (!tween) return;
  let t = Math.min(1, (now - tween.t0) / tween.ms); const e = t * t * (3 - 2 * t);
  camera.position.lerpVectors(tween.fp, tween.tp, e); controls.target.lerpVectors(tween.ft, tween.tt, e);
  if (t >= 1) tween = null;
}
V.topDown = () => { controls.autoRotate = false; tweenTo(new THREE.Vector3(0, 0, 0), new THREE.Vector3(0, 1450, 1), 1100); };
function flyToItem(it) {
  if (it.isRoot && it.eff.items.length > 1) {
    const r = it.eff; let sy = 0; for (const i of r.items) sy += items[i].y;
    easeTo(new THREE.Vector3(r.cx, sy / r.items.length, r.cz), Math.max(120, r.ext * 2.5 + 55));
  } else easeTo(new THREE.Vector3(it.x, it.y, it.z), 150);
}
function flyToSel(s) {
  if (!renderer || !s) return;
  if (s.kind === 'place') {
    const pl = A.layout && A.layout.places.get(s.id); const p = A.data.pMap.get(s.id); if (!pl || !p) return;
    let top = 0; for (const e of A.visEfforts(p)) top = Math.max(top, yOf((Date.now() - e.e) / A.DAY));
    easeTo(new THREE.Vector3((pl.x - 500) * S, top * 0.5, (pl.y - 500) * S), Math.max(300, pl.r * S * 3.2));
  } else if (s.kind === 'effort') { const r = effs.find((x) => x.e.id === s.id); if (r && r.rootIdx != null) flyToItem(items[r.rootIdx]); }
  else { const it = items.find((x) => x.t.id === s.id); if (it) flyToItem(it); }
}

// ---------------------------------------------------------------- loop
function frame(ts) {
  if (!running) return;
  requestAnimationFrame(frame);
  if (!active || !renderer) return;
  if (dirtyData) build();
  uniforms.uTime.value = ts / 1000;
  for (const o of [pts, ring, selRing]) if (o) o.material.uniforms.uTime.value = ts / 1000;
  stepTween(ts);
  controls.update();
  const k = camera.position.x.toFixed(0) + camera.position.y.toFixed(0) + camera.position.z.toFixed(0) + controls.target.x.toFixed(0) + controls.target.y.toFixed(0);
  if (labelDirty || k !== camKey) { camKey = k; labelDirty = false; placeLabels(); }
  renderer.render(scene, camera);
}
V.show = () => {
  active = true; view.classList.add('on');
  const m = document.getElementById('space-msg');
  if (!initRenderer()) return;
  m.hidden = true; document.getElementById('topdown').hidden = false;
  resize();
  if (!built && A.layout) build();
  if (!running) { running = true; requestAnimationFrame(frame); }
  if (A.sel && !userTouched) flyToSel(A.sel);
};
V.hide = () => { active = false; };

A.on('data', () => { dirtyData = true; });
A.on('filter', applyFade);
A.on('hover', () => { if (focusEffort() !== lastFid) applyFade(); });
A.on('select', (s, src) => { syncSel(); if (focusEffort() !== lastFid) applyFade(); if (s && src !== 'space' && active) flyToSel(s); });
V.debug = { get camera() { return camera; }, get controls() { return controls; }, get n() { return items.length; }, get edges() { return edgeN; }, get labels() { let n = 0; for (const v of labelCache.values()) if (v.sp.visible) n++; return n; }, get items() { return items; }, get effs() { return effs; } };
A.emit('space-ready');
