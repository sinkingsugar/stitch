// Space: the map in 3D (design §2.25, "The map": Space). three.js, in the lazily loaded chunk `space.js` (entry.ts),
// so the main bundle stays small and a WebGL failure leaves 2D alone.
//
// The scene, one draw call per kind: the threads (Points), the glow of what works or needs you (Points, additive),
// the marks (selection, hover, the open conversation, unread: Points drawing thin rings), the spawn edges (one
// instanced curved strip per edge, widened in screen space), the ground (place rings, place columns, time levels:
// one LineSegments), the ground's disc. Labels are HTML over the canvas (crisp at any pixel ratio, faded with CSS),
// placed in screen space without overlaps. Every position has a `from` and a `to`, so a re-layout (a time filter, new
// data) glides on the GPU with one uniform. Rendered on demand (a change, an animation, the slow idle rotation);
// nothing runs while hidden or in a background tab. Picking projects the threads (once per camera change) instead
// of raycasting.
import {
  AdditiveBlending, BufferAttribute, DoubleSide, BufferGeometry, InstancedBufferAttribute, InstancedBufferGeometry, LineSegments, Mesh, NormalBlending,
  PerspectiveCamera, Points, Scene, ShaderMaterial, WebGLRenderer, type IUniform,
} from "three";
import { DAY, type MThread, type Sel } from "../model";
import { FONT, ellipsize, tw, useCanvasMeasure } from "../text";
import { HGT, NK, S, layoutSpace, placeLabels, toPack, toWorld, type LabelCand, type SItem, type SpaceLayout } from "./layout";
import type { Space, SpaceIO, SpaceStats, View2D } from "./types";

const FOV = 46;
const TAN = Math.tan((FOV / 2) * Math.PI / 180);
const NSEG = 12; // segments of an edge's curve
const MAX_LABELS = 20;
const GLIDE_MS = 650;
const ROTATE = 0.022; // rad/s, the idle rotation
const IDLE_MS = 9000; // quiet this long (and nothing selected) before it rotates
const IDLE_END_MS = 180_000; // and it stops after three quiet minutes, as the working pulse does after 30 s: no GPU work left
const PHI_MAX = 1.45;
const BG = 0x0a0e15;

// ---------------------------------------------------------------- shaders
const NODE_VS = `
attribute vec3 p0; attribute vec3 color; attribute float size; attribute vec2 pxr; attribute float alpha; attribute float a0;
uniform float uMix, uScale, uDpr, uFogN, uFogF;
varying vec3 vC; varying float vA; varying float vPx;
void main() {
  vec4 mv = modelViewMatrix * vec4(mix(p0, position, uMix), 1.0);
  float d = max(1.0, -mv.z);
  float px = clamp(size * uScale / d, pxr.x * uDpr, pxr.y * uDpr);
  gl_PointSize = px + uDpr;
  vPx = gl_PointSize;
  gl_Position = projectionMatrix * mv;
  vC = color;
  vA = alpha * mix(a0, 1.0, uMix) * (0.32 + 0.68 * (1.0 - smoothstep(uFogN, uFogF, d)));
}`;
const NODE_FS = `
uniform float uDpr;
varying vec3 vC; varying float vA; varying float vPx;
void main() {
  float r = length(gl_PointCoord * 2.0 - 1.0) * vPx * 0.5;
  float R = vPx * 0.5 - 0.5 * uDpr;
  float a = clamp(R - r + 0.5, 0.0, 1.0);
  if (a <= 0.0) discard;
  vec3 c = vC * (1.12 - 0.3 * r / max(R, 1.0));
  gl_FragColor = vec4(c, vA * a);
}`;
const GLOW_VS = `
attribute vec3 p0; attribute vec3 color; attribute float size; attribute vec2 pxr; attribute float alpha; attribute float a0; attribute float pulse;
uniform float uMix, uScale, uDpr, uFogN, uFogF, uTime;
varying vec3 vC; varying float vA;
void main() {
  vec3 p = mix(p0, position, uMix);
  vec4 mv = modelViewMatrix * vec4(p, 1.0);
  float d = max(1.0, -mv.z);
  float px = clamp(size * uScale / d, pxr.x * uDpr, pxr.y * uDpr);
  float k = 1.0 + pulse * 0.22 * sin(uTime * 2.1 + p.x * 0.05);
  gl_PointSize = px * 3.6 * k;
  gl_Position = projectionMatrix * mv;
  vC = color;
  vA = alpha * mix(a0, 1.0, uMix) * (0.5 + 0.5 * (1.0 - smoothstep(uFogN, uFogF, d)));
}`;
const GLOW_FS = `
varying vec3 vC; varying float vA;
void main() {
  float r = length(gl_PointCoord * 2.0 - 1.0);
  if (r > 1.0) discard;
  float a = pow(1.0 - r, 2.2) * 0.55;
  gl_FragColor = vec4(vC * a * vA, 1.0);
}`;
// A thin ring around a node: `ring` = (gap, thickness), css px, around the node's own drawn size.
const MARK_VS = `
attribute vec3 p0; attribute vec3 color; attribute float size; attribute vec2 pxr; attribute float alpha; attribute vec2 ring;
uniform float uMix, uScale, uDpr;
varying vec3 vC; varying float vA; varying float vOut; varying float vRc; varying float vTh;
void main() {
  vec4 mv = modelViewMatrix * vec4(mix(p0, position, uMix), 1.0);
  float d = max(1.0, -mv.z);
  float px = clamp(size * uScale / d, pxr.x * uDpr, pxr.y * uDpr);
  vTh = ring.y * uDpr;
  vRc = px * 0.5 + ring.x * uDpr + vTh * 0.5;
  vOut = vRc + vTh * 0.5 + uDpr;
  gl_PointSize = vOut * 2.0;
  gl_Position = projectionMatrix * mv;
  vC = color; vA = alpha;
}`;
const MARK_FS = `
varying vec3 vC; varying float vA; varying float vOut; varying float vRc; varying float vTh;
void main() {
  float r = length(gl_PointCoord * 2.0 - 1.0) * vOut;
  float a = clamp(vTh * 0.5 - abs(r - vRc) + 0.5, 0.0, 1.0);
  if (a <= 0.0) discard;
  gl_FragColor = vec4(vC, vA * a);
}`;
// An edge: a quadratic curve (a, control, b), from and to, drawn as a strip of NSEG segments widened in screen space.
const EDGE_VS = `
attribute vec3 a0p; attribute vec3 c0p; attribute vec3 b0p; attribute vec3 a1p; attribute vec3 c1p; attribute vec3 b1p;
attribute vec3 ecol; attribute vec3 eprop; attribute float born;
uniform float uMix, uDpr, uFogN, uFogF; uniform vec2 uRes;
varying vec3 vC; varying float vA; varying float vE; varying float vHW;
vec3 bez(float t, vec3 a, vec3 c, vec3 b) { float u = 1.0 - t; return u * u * a + 2.0 * u * t * c + t * t * b; }
vec4 clip(vec3 p) { vec4 mv = modelViewMatrix * vec4(p, 1.0); mv.z = min(mv.z, -1.0); return projectionMatrix * mv; }
void main() {
  vec3 a = mix(a0p, a1p, uMix), c = mix(c0p, c1p, uMix), b = mix(b0p, b1p, uMix);
  float t = position.x;
  vec3 p = bez(t, a, c, b);
  vec4 mv = modelViewMatrix * vec4(p, 1.0);
  float behind = step(-1.0, mv.z);
  vec4 q = clip(p);
  vec4 q1 = clip(bez(clamp(t - 0.02, 0.0, 1.0), a, c, b)), q2 = clip(bez(clamp(t + 0.02, 0.0, 1.0), a, c, b));
  vec2 s1 = q1.xy / q1.w * uRes, s2 = q2.xy / q2.w * uRes;
  vec2 dir = s2 - s1; float l = length(dir); dir = l > 1e-5 ? dir / l : vec2(1.0, 0.0);
  vec2 nrm = vec2(-dir.y, dir.x);
  vHW = eprop.y * uDpr * 0.5;
  float hw = vHW + uDpr;
  q.xy += nrm * position.y * hw / uRes * q.w;
  gl_Position = q;
  vE = position.y * hw;
  vC = ecol;
  float d = -mv.z;
  vA = (1.0 - behind) * eprop.x * mix(born, 1.0, uMix) * mix(eprop.z, 1.0, t) * (0.35 + 0.65 * (1.0 - smoothstep(uFogN, uFogF, d)));
}`;
const EDGE_FS = `
varying vec3 vC; varying float vA; varying float vE; varying float vHW;
void main() {
  float a = clamp(vHW - abs(vE) + 0.5, 0.0, 1.0);
  if (a <= 0.0) discard;
  gl_FragColor = vec4(vC, vA * a);
}`;
const LINE_VS = `
attribute vec3 p0; attribute vec3 color; attribute float alpha; attribute float a0;
uniform float uMix, uFogN, uFogF;
varying vec3 vC; varying float vA;
void main() {
  vec4 mv = modelViewMatrix * vec4(mix(p0, position, uMix), 1.0);
  gl_Position = projectionMatrix * mv;
  vC = color; vA = alpha * mix(a0, 1.0, uMix) * (0.4 + 0.6 * (1.0 - smoothstep(uFogN, uFogF, -mv.z)));
}`;
const LINE_FS = `
varying vec3 vC; varying float vA;
void main() { gl_FragColor = vec4(vC, vA); }`;
const DISC_VS = `
varying vec2 vU;
void main() { vU = position.xz; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`;
const DISC_FS = `
uniform float uR; varying vec2 vU;
void main() {
  float r = length(vU) / uR;
  if (r > 1.0) discard;
  float a = 0.07 * (1.0 - smoothstep(0.55, 1.0, r)) + 0.025;
  gl_FragColor = vec4(0.47, 0.59, 0.77, a * (1.0 - smoothstep(0.96, 1.0, r)));
}`;

// ---------------------------------------------------------------- colours (sRGB, as written)
type RGB = [number, number, number];
const NEED: RGB = [0.984, 0.573, 0.235];
const WORK: RGB = [0.176, 0.831, 0.749];
const MINT: RGB = [0.38, 0.8, 0.78];
const WHITE: RGB = [1, 1, 1];
const UNREAD: RGB = [0.66, 0.75, 0.88];
/** In the scope tray (design §2.27): a lilac ring, apart from the selection's white and the open conversation's mint. */
const PICKED: RGB = [0.77, 0.65, 1];
function hsl(h: number, s: number, l: number, out: RGB): RGB {
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => { const k = (n + h / 30) % 12; return l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1)); };
  out[0] = f(0); out[1] = f(8); out[2] = f(4);
  return out;
}
const clamp01 = (x: number) => Math.max(0, Math.min(1, x));
const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
const smooth = (t: number) => t * t * (3 - 2 * t);
const tmp: RGB = [0, 0, 0];

/** Minimum and maximum drawn size (css px) per kind: what works or needs you never shrinks to a speck. */
const PXR: Record<NK, [number, number]> = { [NK.Child]: [2.4, 16], [NK.Root]: [4.6, 30], [NK.Lone]: [2.6, 14], [NK.Work]: [6, 26], [NK.Need]: [7, 28] };

interface Cam { tx: number; ty: number; tz: number; dist: number; theta: number; phi: number }
interface Lab { el: HTMLDivElement; o: number; target: number; cand: LabelCand; text: string; cls: string; sx: number; sy: number; level: boolean }

export function createSpace(host: HTMLElement, io: SpaceIO): Space {
  useCanvasMeasure();
  const instant = io.instant;
  const renderer = new WebGLRenderer({ antialias: true, alpha: false, powerPreference: "high-performance", preserveDrawingBuffer: instant });
  renderer.setClearColor(BG, 1);
  const canvas = renderer.domElement;
  canvas.tabIndex = 0;
  canvas.setAttribute("aria-label", "The map in 3D: places on the ground, conversations by recency, their spawned threads around them");
  canvas.className = "spacecanvas";
  const labelLayer = document.createElement("div");
  labelLayer.className = "spacelabels";
  host.append(canvas, labelLayer);

  const scene = new Scene();
  const camera = new PerspectiveCamera(FOV, 1, 1, 10000);
  const U = {
    uMix: { value: 1 }, uScale: { value: 800 }, uDpr: { value: 1 }, uFogN: { value: 1000 }, uFogF: { value: 4000 }, uTime: { value: 0 },
  };
  /** Half the drawing buffer's size, px (NDC → px). */
  const uRes = { value: { x: 1, y: 1 } };
  const mat = (vs: string, fs: string, additive = false, extra: Record<string, IUniform> = {}) =>
    new ShaderMaterial({ uniforms: { ...U, uRes, ...extra } as Record<string, IUniform>, vertexShader: vs, fragmentShader: fs, transparent: true, depthWrite: false, depthTest: false, blending: additive ? AdditiveBlending : NormalBlending });

  // ---------------------------------------------------------------- state
  let L: SpaceLayout = layoutSpace(io.model(), null, Date.now());
  /** Displayed position of every item (mix of from and to), refreshed with the projection. */
  let cur = new Float32Array(0);
  let p0 = new Float32Array(0);
  /** Per item: screen x, y (css px), depth, drawn radius (css px); valid for `projV`. */
  let proj = new Float32Array(0);
  let projV = -1, camV = 0, labV = -1;
  let W = 0, H = 0, dpr = 1;
  let active = false, destroyed = false, everShown = false, dataDirty = true, styleDirty = true;
  let raf = 0, pulseTimer: ReturnType<typeof setTimeout> | undefined;
  let glide: { t0: number } | null = null;
  let lastTs = 0, lastInput = performance.now();
  let hoverIdx = -1;
  let focusEff = -1, focusFromSel = false;
  let pendingSel: Sel | null = null;
  let hasPulse = false;
  const reduced = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  const cam: Cam = { tx: 0, ty: HGT * 0.36, tz: 0, dist: 2000, theta: 0.62, phi: 0.98 };
  let ox = 0, oxTarget = 0;
  let tween: { t0: number; ms: number; from: Cam; to: Cam } | null = null;
  const vel = { theta: 0, phi: 0 };
  let frames = 0, frameMs = 0;

  // ---------------------------------------------------------------- objects (geometries replaced on each re-layout)
  const nodesMat = mat(NODE_VS, NODE_FS);
  const glowMat = mat(GLOW_VS, GLOW_FS, true);
  const markMat = mat(MARK_VS, MARK_FS);
  const edgeMat = mat(EDGE_VS, EDGE_FS);
  edgeMat.side = DoubleSide; // the strip's winding follows the edge's direction on screen
  const lineMat = mat(LINE_VS, LINE_FS);
  const discU = { uR: { value: 1 } };
  const discMat = mat(DISC_VS, DISC_FS, false, discU);
  discMat.side = DoubleSide;
  const nodes = new Points(new BufferGeometry(), nodesMat);
  const glow = new Points(new BufferGeometry(), glowMat);
  const marks = new Points(new BufferGeometry(), markMat);
  const edges = new Mesh(new InstancedBufferGeometry(), edgeMat);
  const ground = new LineSegments(new BufferGeometry(), lineMat);
  const discGeo = new BufferGeometry();
  const disc = new Mesh(discGeo, discMat);
  disc.renderOrder = 0; ground.renderOrder = 1; edges.renderOrder = 2; glow.renderOrder = 3; nodes.renderOrder = 4; marks.renderOrder = 5;
  for (const o of [disc, ground, edges, glow, nodes, marks]) { o.frustumCulled = false; scene.add(o); }

  // The strip every edge instances: NSEG+1 pairs of (t, ±1).
  const strip = new Float32Array((NSEG + 1) * 2 * 3);
  const stripIdx: number[] = [];
  for (let k = 0; k <= NSEG; k++) {
    strip.set([k / NSEG, -1, 0, k / NSEG, 1, 0], k * 6);
    if (k < NSEG) { const a = k * 2; stripIdx.push(a, a + 1, a + 3, a, a + 3, a + 2); }
  }

  // Per-edge data kept for restyling.
  let edgeChild = new Int32Array(0);
  let glowIdx = new Int32Array(0);
  let markCap = 0;

  const attr = (g: BufferGeometry, name: string, a: Float32Array, n: number) => g.setAttribute(name, new BufferAttribute(a, n));
  const iattr = (g: BufferGeometry, name: string, a: Float32Array, n: number) => g.setAttribute(name, new InstancedBufferAttribute(a, n));
  const swap = <T extends BufferGeometry>(o: { geometry: BufferGeometry }, g: T) => { o.geometry.dispose(); o.geometry = g; };

  /** The control point of an edge's curve: bulging sideways (a side per edge) and a little up. */
  function control(ax: number, ay: number, az: number, bx: number, by: number, bz: number, side: number, out: Float32Array, o: number) {
    const dx = bx - ax, dz = bz - az, h = Math.hypot(dx, dz) + 1e-3, len = h + Math.abs(by - ay) * 0.3;
    out[o] = (ax + bx) / 2 - (dz / h) * len * 0.14 * side;
    out[o + 1] = (ay + by) / 2 + len * 0.05;
    out[o + 2] = (az + bz) / 2 + (dx / h) * len * 0.14 * side;
  }
  const sideOf = (id: string) => { let h = 0; for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) | 0; return h & 1 ? 1 : -1; };

  // ---------------------------------------------------------------- build (a re-layout)
  function build(animate: boolean) {
    dataDirty = false;
    const now = Date.now();
    const old = L, oldCur = cur;
    // Where everything is shown now (mid-glide included), so the new layout starts from there.
    L = layoutSpace(io.model(), io.layout(), now);
    const n = L.items.length;
    const pos = new Float32Array(n * 3);
    p0 = new Float32Array(n * 3);
    const born = new Float32Array(n);
    for (let i = 0; i < n; i++) {
      const it = L.items[i]!;
      pos[i * 3] = it.x; pos[i * 3 + 1] = it.y; pos[i * 3 + 2] = it.z;
      const j = animate ? old.byId.get(it.t.id) : undefined;
      if (j !== undefined && j * 3 + 2 < oldCur.length) { p0[i * 3] = oldCur[j * 3]!; p0[i * 3 + 1] = oldCur[j * 3 + 1]!; p0[i * 3 + 2] = oldCur[j * 3 + 2]!; born[i] = 1; }
      else { p0[i * 3] = it.x; p0[i * 3 + 1] = it.y; p0[i * 3 + 2] = it.z; born[i] = animate ? 0 : 1; }
    }
    cur = new Float32Array(p0);
    proj = new Float32Array(n * 4);
    projV = -1;
    // Threads: drawn in order idle, working, needing you, so the loud ones are on top from any angle.
    const g = new BufferGeometry();
    attr(g, "position", pos, 3); attr(g, "p0", p0, 3); attr(g, "a0", born, 1);
    attr(g, "color", new Float32Array(n * 3), 3); attr(g, "alpha", new Float32Array(n), 1);
    const size = new Float32Array(n), pxr = new Float32Array(n * 2);
    L.items.forEach((it, i) => { size[i] = it.size; const r = PXR[it.kind]; pxr[i * 2] = r[0]; pxr[i * 2 + 1] = r[1]; });
    attr(g, "size", size, 1); attr(g, "pxr", pxr, 2);
    g.setIndex(drawOrder());
    swap(nodes, g);
    // Glow: what works or needs you.
    const gl: number[] = [];
    L.items.forEach((it, i) => { if (it.kind === NK.Work || it.kind === NK.Need) gl.push(i); });
    glowIdx = Int32Array.from(gl);
    const gg = new BufferGeometry();
    const gpos = new Float32Array(gl.length * 3), gp0 = new Float32Array(gl.length * 3), gsz = new Float32Array(gl.length), gpxr = new Float32Array(gl.length * 2);
    const gcol = new Float32Array(gl.length * 3), gpulse = new Float32Array(gl.length), ga0 = new Float32Array(gl.length);
    gl.forEach((i, k) => {
      const it = L.items[i]!;
      gpos.set(pos.subarray(i * 3, i * 3 + 3), k * 3); gp0.set(p0.subarray(i * 3, i * 3 + 3), k * 3);
      gsz[k] = it.size; gpxr[k * 2] = PXR[it.kind][0]; gpxr[k * 2 + 1] = PXR[it.kind][1];
      gcol.set(it.kind === NK.Need ? NEED : WORK, k * 3); gpulse[k] = it.kind === NK.Work ? 1 : 0; ga0[k] = born[i]!;
    });
    hasPulse = gl.some((i) => L.items[i]!.kind === NK.Work);
    attr(gg, "position", gpos, 3); attr(gg, "p0", gp0, 3); attr(gg, "size", gsz, 1); attr(gg, "pxr", gpxr, 2); attr(gg, "color", gcol, 3);
    attr(gg, "pulse", gpulse, 1); attr(gg, "a0", ga0, 1); attr(gg, "alpha", new Float32Array(gl.length), 1);
    swap(glow, gg);
    // Edges.
    const ch: number[] = [];
    L.items.forEach((it, i) => { if (it.parent >= 0) ch.push(i); });
    edgeChild = Int32Array.from(ch);
    const m = ch.length;
    const A0 = new Float32Array(m * 3), C0 = new Float32Array(m * 3), B0 = new Float32Array(m * 3), A1 = new Float32Array(m * 3), C1 = new Float32Array(m * 3), B1 = new Float32Array(m * 3);
    const eborn = new Float32Array(m);
    ch.forEach((c, k) => {
      const it = L.items[c]!, p = it.parent, side = sideOf(it.t.id);
      A1.set(pos.subarray(p * 3, p * 3 + 3), k * 3); B1.set(pos.subarray(c * 3, c * 3 + 3), k * 3);
      control(pos[p * 3]!, pos[p * 3 + 1]!, pos[p * 3 + 2]!, pos[c * 3]!, pos[c * 3 + 1]!, pos[c * 3 + 2]!, side, C1, k * 3);
      A0.set(p0.subarray(p * 3, p * 3 + 3), k * 3); B0.set(p0.subarray(c * 3, c * 3 + 3), k * 3);
      control(p0[p * 3]!, p0[p * 3 + 1]!, p0[p * 3 + 2]!, p0[c * 3]!, p0[c * 3 + 1]!, p0[c * 3 + 2]!, side, C0, k * 3);
      eborn[k] = Math.min(born[p]!, born[c]!);
    });
    const eg = new InstancedBufferGeometry();
    eg.setAttribute("position", new BufferAttribute(strip, 3));
    eg.setIndex(stripIdx);
    iattr(eg, "a0p", A0, 3); iattr(eg, "c0p", C0, 3); iattr(eg, "b0p", B0, 3); iattr(eg, "a1p", A1, 3); iattr(eg, "c1p", C1, 3); iattr(eg, "b1p", B1, 3);
    iattr(eg, "ecol", new Float32Array(m * 3), 3); iattr(eg, "eprop", new Float32Array(m * 3), 3); iattr(eg, "born", eborn, 1);
    eg.instanceCount = m;
    swap(edges, eg);
    buildGround(old, animate);
    // Marks: unread rings, then selection, hover, the open conversation.
    // Unread and picked rings can each mark every item, then the open conversation, hover and selection.
    markCap = L.items.length * 2 + 3;
    const mg = new BufferGeometry();
    attr(mg, "position", new Float32Array(markCap * 3), 3); attr(mg, "p0", new Float32Array(markCap * 3), 3); attr(mg, "color", new Float32Array(markCap * 3), 3);
    attr(mg, "size", new Float32Array(markCap), 1); attr(mg, "pxr", new Float32Array(markCap * 2), 2); attr(mg, "alpha", new Float32Array(markCap), 1);
    attr(mg, "ring", new Float32Array(markCap * 2), 2);
    mg.setDrawRange(0, 0);
    swap(marks, mg);
    U.uMix.value = animate && !instant ? 0 : 1;
    glide = U.uMix.value < 1 ? { t0: performance.now() } : null;
    if (!glide) refreshCur(1);
    hoverIdx = -1;
    styleDirty = true;
    labV = -1;
  }

  /** Index order: idle first, then working, then needing you (drawn last, on top). */
  function drawOrder(): number[] {
    const out: number[] = [];
    for (const k of [[NK.Child, NK.Lone, NK.Root], [NK.Work], [NK.Need]] as NK[][]) L.items.forEach((it, i) => { if (k.includes(it.kind)) out.push(i); });
    return out;
  }

  // Ground: place rings and columns, time levels. Old positions (by place key, by level) for the glide.
  let groundKeys = new Map<string, number[]>();
  function buildGround(_old: SpaceLayout, animate: boolean) {
    const segs: number[] = [], segs0: number[] = [], col: number[] = [], al: number[] = [], born: number[] = [];
    const keys = new Map<string, number[]>();
    const oldG = ground.geometry.getAttribute("position") as BufferAttribute | undefined;
    const oldP0 = ground.geometry.getAttribute("p0") as BufferAttribute | undefined;
    const um = U.uMix.value;
    const at = (v: number) => (oldG && oldP0 ? lerp(oldP0.array[v]!, oldG.array[v]!, um) : 0);
    const push = (key: string, pts: number[], c: RGB, a: number) => {
      const was = animate ? groundKeys.get(key) : undefined;
      const start = segs.length;
      for (let i = 0; i < pts.length; i++) { segs.push(pts[i]!); segs0.push(was && was.length === pts.length && oldG ? at(was[i]!) : pts[i]!); }
      const idx: number[] = [];
      for (let i = 0; i < pts.length; i++) idx.push(start + i);
      keys.set(key, idx);
      for (let i = 0; i < pts.length / 3; i++) { col.push(c[0], c[1], c[2]); al.push(a); born.push(was || !animate ? 1 : 0); }
    };
    const circle = (cx: number, y: number, cz: number, r: number, n: number) => {
      const p: number[] = [];
      for (let i = 0; i < n; i++) {
        const a = (i / n) * Math.PI * 2, b = ((i + 1) / n) * Math.PI * 2;
        p.push(cx + Math.cos(a) * r, y, cz + Math.sin(a) * r, cx + Math.cos(b) * r, y, cz + Math.sin(b) * r);
      }
      return p;
    };
    const R = L.radius * 1.03;
    for (const lv of L.scale.levels) {
      const today = lv.days === 0;
      push("lv:" + lv.days, circle(0, lv.y, 0, R, 128), today ? WORK : [0.36, 0.45, 0.58], today ? 0.26 : 0.13);
    }
    for (const p of L.places) {
      const need = p.p.need > 0;
      push("pl:" + p.p.key, circle(p.cx, 0.5, p.cz, p.r, Math.max(24, Math.min(96, Math.round(p.r / 3)))), need ? NEED : [0.5, 0.6, 0.75], need ? 0.42 : 0.26);
      if (p.r > 14 && p.top > 0) push("col:" + p.p.key, [p.cx, 0, p.cz, p.cx, p.top, p.cz], [0.5, 0.6, 0.75], 0.07);
    }
    groundKeys = keys;
    const g = new BufferGeometry();
    attr(g, "position", Float32Array.from(segs), 3); attr(g, "p0", Float32Array.from(segs0), 3);
    attr(g, "color", Float32Array.from(col), 3); attr(g, "alpha", Float32Array.from(al), 1); attr(g, "a0", Float32Array.from(born), 1);
    swap(ground, g);
    const D = new Float32Array([-R, 0, -R, R, 0, -R, R, 0, R, -R, 0, R]);
    discGeo.setAttribute("position", new BufferAttribute(D, 3));
    discGeo.setIndex([0, 2, 1, 0, 3, 2]);
    discU.uR.value = R;
  }

  // ---------------------------------------------------------------- style (filters, focus, selection)
  function effortIdx(s: Sel | null): number {
    if (!s || s.kind === "place") return -1;
    const id = s.kind === "effort" ? s.id : io.model().byId.get(s.id)?.rootId;
    const i = id ? L.eIdx.get(id) : undefined;
    return i ?? -1;
  }
  function restyle() {
    styleDirty = false;
    const now = Date.now();
    const sel = io.sel();
    const fs = effortIdx(sel), fh = effortIdx(io.hovered());
    focusEff = fs >= 0 ? fs : fh;
    focusFromSel = fs >= 0;
    const dimOther = focusEff < 0 ? 1 : focusFromSel ? 0.22 : 0.5;
    const g = nodes.geometry;
    const col = (g.getAttribute("color") as BufferAttribute).array as Float32Array, al = (g.getAttribute("alpha") as BufferAttribute).array as Float32Array;
    L.items.forEach((it, i) => {
      const t = it.t, age = clamp01((now - t.e) / DAY / 14);
      let c: RGB, a: number;
      if (it.kind === NK.Need) { c = NEED; a = 1; }
      else if (it.kind === NK.Work) { c = WORK; a = 1; }
      else if (it.kind === NK.Root) { c = hsl(214, 0.42, lerp(0.8, 0.5, age), tmp); a = lerp(1, 0.75, age); }
      else if (it.kind === NK.Lone) { c = hsl(216, 0.22, lerp(0.58, 0.36, age), tmp); a = lerp(0.62, 0.38, age); }
      else { c = hsl(214, 0.4, lerp(0.7, 0.45, age), tmp); a = lerp(0.95, 0.6, age); }
      if (t.synthetic) a *= 0.45;
      a *= Math.max(0.1, t.fade);
      if (focusEff >= 0 && it.eff !== focusEff) a *= it.kind >= NK.Work ? Math.max(dimOther, 0.55) : dimOther;
      col[i * 3] = c[0]; col[i * 3 + 1] = c[1]; col[i * 3 + 2] = c[2]; al[i] = a;
    });
    g.getAttribute("color").needsUpdate = true; g.getAttribute("alpha").needsUpdate = true;
    const ga = (glow.geometry.getAttribute("alpha") as BufferAttribute | undefined);
    if (ga) {
      glowIdx.forEach((i, k) => { const it = L.items[i]!; (ga.array as Float32Array)[k] = Math.max(0.15, it.t.fade) * (focusEff >= 0 && it.eff !== focusEff ? 0.55 : 1); });
      ga.needsUpdate = true;
    }
    // Edges: the structure. Brighter and wider for what moved lately, coloured when the child works or needs you,
    // the focused effort's highlighted and the rest dimmed.
    const eg = edges.geometry;
    const ec = (eg.getAttribute("ecol") as InstancedBufferAttribute | undefined), ep = (eg.getAttribute("eprop") as InstancedBufferAttribute | undefined);
    if (ec && ep) {
      const ca = ec.array as Float32Array, pa = ep.array as Float32Array;
      edgeChild.forEach((ci, k) => {
        const it = L.items[ci]!, t = it.t, rec = clamp01(1 - (now - t.e) / (DAY * 10));
        const strong = t.need || t.working;
        let a = strong ? 0.95 : 0.55 + 0.35 * rec, w = strong ? 2.4 : 1.6 + 0.7 * rec;
        if (focusEff >= 0) {
          if (it.eff === focusEff) { a = Math.min(1, a * 1.35 + 0.25); w *= 1.35; }
          else a *= focusFromSel ? (strong ? 0.4 : 0.14) : 0.4;
        }
        a *= Math.max(0.1, t.fade);
        const c = t.need ? NEED : t.working ? WORK : hsl(213, 0.7, lerp(0.66, 0.8, rec), tmp);
        ca[k * 3] = c[0]; ca[k * 3 + 1] = c[1]; ca[k * 3 + 2] = c[2];
        pa[k * 3] = a; pa[k * 3 + 1] = w; pa[k * 3 + 2] = strong ? 0.8 : 0.65;
      });
      ec.needsUpdate = true; ep.needsUpdate = true;
    }
    restyleMarks();
    labV = -1;
  }
  function restyleMarks() {
    const g = marks.geometry;
    const P = g.getAttribute("position") as BufferAttribute | undefined;
    if (!P) return;
    const pos = P.array as Float32Array, pp0 = (g.getAttribute("p0") as BufferAttribute).array as Float32Array;
    const col = (g.getAttribute("color") as BufferAttribute).array as Float32Array, sz = (g.getAttribute("size") as BufferAttribute).array as Float32Array;
    const pxr = (g.getAttribute("pxr") as BufferAttribute).array as Float32Array, al = (g.getAttribute("alpha") as BufferAttribute).array as Float32Array;
    const ring = (g.getAttribute("ring") as BufferAttribute).array as Float32Array;
    let n = 0;
    const put = (i: number, c: RGB, a: number, gap: number, th: number) => {
      if (i < 0 || n >= markCap) return;
      const it = L.items[i]!;
      pos[n * 3] = it.x; pos[n * 3 + 1] = it.y; pos[n * 3 + 2] = it.z;
      pp0[n * 3] = p0[i * 3]!; pp0[n * 3 + 1] = p0[i * 3 + 1]!; pp0[n * 3 + 2] = p0[i * 3 + 2]!;
      col[n * 3] = c[0]; col[n * 3 + 1] = c[1]; col[n * 3 + 2] = c[2];
      sz[n] = it.size; pxr[n * 2] = PXR[it.kind][0]; pxr[n * 2 + 1] = PXR[it.kind][1];
      al[n] = a; ring[n * 2] = gap; ring[n * 2 + 1] = th;
      n++;
    };
    // Unread: a hairline, quiet, only when the toggle shows them.
    if (io.stats().unreadOn) L.items.forEach((it, i) => { if (it.t.unread && it.t.fade > 0.5 && (focusEff < 0 || it.eff === focusEff)) put(i, UNREAD, 0.22, 1.5, 1); });
    const picked = io.picked?.();
    if (picked?.size) for (const id of picked) put(L.byId.get(id) ?? -1, PICKED, 0.9, 9, 1.5);
    const op = io.opened();
    put(op ? L.byId.get(op) ?? -1 : -1, MINT, 0.9, 6, 1.5);
    put(hoverIdx, WHITE, 0.55, 3, 1);
    const s = io.sel();
    const si = s && s.kind !== "place" ? (s.kind === "effort" ? L.efforts[L.eIdx.get(s.id) ?? -1]?.rootIdx ?? -1 : L.byId.get(s.id) ?? -1) : -1;
    put(si, WHITE, 0.95, 3, 1.5);
    for (const name of ["position", "p0", "color", "size", "pxr", "alpha", "ring"]) g.getAttribute(name).needsUpdate = true;
    g.setDrawRange(0, n);
  }

  // ---------------------------------------------------------------- camera
  function fitDist(radius: number) {
    const aspect = W && H ? W / H : 1.6;
    return radius / (TAN * Math.min(1, aspect)) * 1.08;
  }
  function overview(): Cam {
    return { tx: 0, ty: HGT * 0.3, tz: 0, dist: fitDist(L.radius) * 0.92, theta: 0.62, phi: 0.98 };
  }
  function applyCam() {
    const sp = Math.sin(cam.phi), cp = Math.cos(cam.phi), st = Math.sin(cam.theta), ct = Math.cos(cam.theta);
    camera.position.set(cam.tx + cam.dist * sp * st, cam.ty + cam.dist * cp, cam.tz + cam.dist * sp * ct);
    camera.up.set(-cp * st, sp, -cp * ct);
    camera.lookAt(cam.tx, cam.ty, cam.tz);
    camera.near = Math.max(1, cam.dist * 0.02);
    camera.far = cam.dist * 4 + 4000;
    if (Math.abs(ox) > 0.5 && W && H) camera.setViewOffset(W, H, ox, 0, W, H);
    else camera.clearViewOffset();
    camera.updateProjectionMatrix();
    camera.updateMatrixWorld();
    U.uFogN.value = cam.dist * 0.6;
    U.uFogF.value = cam.dist * 2.6;
    camV++;
  }
  const wrapAngle = (a: number) => Math.atan2(Math.sin(a), Math.cos(a));
  function flyTo(to: Cam, ms = 900) {
    if (instant || ms <= 0) { tween = null; Object.assign(cam, to); applyCam(); want(); return; }
    const from = { ...cam };
    to = { ...to, theta: from.theta + wrapAngle(to.theta - from.theta) };
    tween = { t0: performance.now(), ms, from, to };
    vel.theta = vel.phi = 0;
    want();
  }
  function stepTween(now: number): boolean {
    if (!tween) return false;
    const t = Math.min(1, (now - tween.t0) / tween.ms), e = smooth(t), a = tween.from, b = tween.to;
    cam.tx = lerp(a.tx, b.tx, e); cam.ty = lerp(a.ty, b.ty, e); cam.tz = lerp(a.tz, b.tz, e);
    cam.dist = Math.exp(lerp(Math.log(a.dist), Math.log(b.dist), e));
    cam.theta = lerp(a.theta, b.theta, e); cam.phi = lerp(a.phi, b.phi, e);
    if (t >= 1) tween = null;
    return true;
  }
  const keepAngles = (c: Omit<Cam, "theta" | "phi">): Cam => ({ ...c, theta: cam.theta, phi: Math.min(Math.max(cam.phi, 0.35), 1.15) });
  function flyToEffort(ei: number, ms?: number) {
    const r = L.efforts[ei];
    if (!r) return;
    const lone = r.items.length <= 1;
    const it = L.items[r.rootIdx]!;
    if (lone) flyTo(keepAngles({ tx: it.x, ty: it.y, tz: it.z, dist: 190 }), ms);
    else flyTo(keepAngles({ tx: r.cx, ty: r.cy, tz: r.cz, dist: Math.max(150, fitDist(r.ext + 12) * 1.15) }), ms);
  }
  function flyToSel(s: Sel, ms?: number) {
    if (s.kind === "place") {
      const p = L.places.find((q) => q.p.key === s.id);
      if (p) flyTo(keepAngles({ tx: p.cx, ty: p.top * 0.45, tz: p.cz, dist: Math.max(260, fitDist(p.r) * 1.25) }), ms);
      return;
    }
    flyToEffort(effortIdx(s), ms);
  }

  // ---------------------------------------------------------------- projection, picking
  function refreshCur(mix: number) {
    const pos = (nodes.geometry.getAttribute("position") as BufferAttribute | undefined)?.array as Float32Array | undefined;
    if (!pos || pos.length !== cur.length) return;
    for (let i = 0; i < cur.length; i++) cur[i] = lerp(p0[i]!, pos[i]!, mix);
  }
  function project() {
    if (projV === camV) return;
    projV = camV;
    const m = camera.matrixWorldInverse.elements, p = camera.projectionMatrix.elements;
    const n = L.items.length, scale = H / (2 * TAN);
    for (let i = 0; i < n; i++) {
      const x = cur[i * 3]!, y = cur[i * 3 + 1]!, z = cur[i * 3 + 2]!;
      const vx = m[0]! * x + m[4]! * y + m[8]! * z + m[12]!, vy = m[1]! * x + m[5]! * y + m[9]! * z + m[13]!, vz = m[2]! * x + m[6]! * y + m[10]! * z + m[14]!;
      const cx = p[0]! * vx + p[4]! * vy + p[8]! * vz + p[12]!, cy = p[1]! * vx + p[5]! * vy + p[9]! * vz + p[13]!, cw = p[3]! * vx + p[7]! * vy + p[11]! * vz + p[15]!;
      const d = -vz, o = i * 4;
      if (d < 1 || cw <= 0) { proj[o] = -1e5; proj[o + 1] = -1e5; proj[o + 2] = -1; proj[o + 3] = 0; continue; }
      proj[o] = (cx / cw * 0.5 + 0.5) * W; proj[o + 1] = (-cy / cw * 0.5 + 0.5) * H; proj[o + 2] = d;
      const it = L.items[i]!, r = PXR[it.kind];
      proj[o + 3] = Math.min(r[1], Math.max(r[0], it.size * scale / d)) / 2;
    }
  }
  function pickAt(x: number, y: number): number {
    project();
    let best = -1, bs = 1e9;
    const al = (nodes.geometry.getAttribute("alpha") as BufferAttribute | undefined)?.array;
    for (let i = 0; i < L.items.length; i++) {
      const o = i * 4;
      if (proj[o + 2]! < 0) continue;
      if (al && al[i]! < 0.08) continue;
      const dx = proj[o]! - x, dy = proj[o + 1]! - y, d = Math.hypot(dx, dy), rad = Math.max(7, proj[o + 3]! + 4);
      if (d > rad) continue;
      const loud = L.items[i]!.kind >= NK.Work ? 0.85 : 1;
      const s = (d / rad) * loud;
      if (s < bs) { bs = s; best = i; }
    }
    return best;
  }
  const selOfItem = (it: SItem): Sel => ({ kind: it.t.rootId === it.t.id ? "effort" : "thread", id: it.t.id });

  // ---------------------------------------------------------------- labels (HTML, collision-avoided, faded)
  const labs = new Map<string, Lab>();
  const cands: LabelCand[] = [];
  function lab(key: string, text: string, cls: string, size: number, level = false): Lab {
    let l = labs.get(key);
    if (!l) {
      const el = document.createElement("div");
      el.className = "slab " + cls;
      el.textContent = text;
      el.style.opacity = "0";
      labelLayer.appendChild(el);
      const w = tw(text) * size + 8;
      l = { el, o: 0, target: 0, text, cls, sx: 0, sy: 0, level, cand: { key, x: 0, y: 0, w, h: size * 1.3, prio: 0, shown: false, force: false, on: false } };
      labs.set(key, l);
    } else if (l.text !== text) {
      l.text = text; l.el.textContent = text; l.cand.w = tw(text) * size + 8;
    }
    return l;
  }
  const titleOf = (t: MThread, size: number, max: number) => ellipsize(t.title || t.id, size, max);
  function placeAll() {
    labV = camV;
    project();
    cands.length = 0;
    const box = { x: 4, y: 4, w: W - 8 - io.cardWidth() * (io.sel() ? 1 : 0), h: H - 30 };
    for (const l of labs.values()) { l.target = 0; l.cand.shown = l.o > 0.5; l.cand.force = false; }
    const sel = io.sel();
    const selI = sel && sel.kind !== "place" ? (sel.kind === "effort" ? L.efforts[L.eIdx.get(sel.id) ?? -1]?.rootIdx ?? -1 : L.byId.get(sel.id) ?? -1) : -1;
    const add = (i: number, prio: number, force: boolean) => {
      const it = L.items[i]!, o = i * 4;
      if (proj[o + 2]! < 0) return;
      const root = it.root;
      const size = root ? 12.5 : 11.5;
      const kind = it.kind === NK.Need ? " need" : it.kind === NK.Work ? " work" : "";
      const l = lab("t:" + it.t.id, titleOf(it.t, size, root ? 280 : 220), (root ? "root" : "kid") + kind, size);
      l.sx = proj[o]!; l.sy = proj[o + 1]! - proj[o + 3]! - 3;
      l.cand.x = l.sx; l.cand.y = l.sy; l.cand.prio = prio; l.cand.force = force;
      cands.push(l.cand);
    };
    if (focusEff >= 0) {
      const r = L.efforts[focusEff]!;
      for (const i of r.items) { const it = L.items[i]!; add(i, (it.root ? 10 : 6) + (it.kind >= NK.Work ? 2 : 0) + Math.min(1, it.t.w), i === selI || i === hoverIdx); }
    } else if (hoverIdx >= 0) add(hoverIdx, 20, true);
    // The overview: the efforts that matter most (size, recency, what works or needs you).
    for (let ei = 0; ei < L.efforts.length; ei++) {
      const r = L.efforts[ei]!;
      if (ei === focusEff || r.rootIdx < 0 || r.e.fade < 0.3) continue;
      if (r.score < 0.4 && !r.e.need) continue;
      add(r.rootIdx, r.score, false);
    }
    // Places: their names on the near rim of their ring, on the ground.
    const sx = Math.sin(cam.theta), cz = Math.cos(cam.theta);
    const m = camera.matrixWorldInverse.elements, p = camera.projectionMatrix.elements;
    const toScreen = (x: number, y: number, z: number): [number, number] | null => {
      const vx = m[0]! * x + m[4]! * y + m[8]! * z + m[12]!, vy = m[1]! * x + m[5]! * y + m[9]! * z + m[13]!, vz = m[2]! * x + m[6]! * y + m[10]! * z + m[14]!;
      const cx = p[0]! * vx + p[4]! * vy + p[8]! * vz + p[12]!, cy = p[1]! * vx + p[5]! * vy + p[9]! * vz + p[13]!, cw = p[3]! * vx + p[7]! * vy + p[11]! * vz + p[15]!;
      if (cw <= 1) return null;
      return [(cx / cw * 0.5 + 0.5) * W, (-cy / cw * 0.5 + 0.5) * H];
    };
    const near = cam.phi > 0.25 ? 1 : cam.phi / 0.25; // from straight above, in the middle of the ring
    for (const pl of L.places) {
      const s = toScreen(pl.cx + sx * pl.r * near, 0, pl.cz + cz * pl.r * near);
      if (!s) continue;
      const size = 13;
      const l = lab("p:" + pl.p.key, pl.p.name, "place" + (pl.p.need ? " need" : ""), size);
      l.sx = s[0]; l.sy = s[1] + size * 1.3 + 3;
      l.cand.x = l.sx; l.cand.y = l.sy; l.cand.prio = 3 + Math.min(2, Math.log10(1 + pl.p.count)) + (sel?.kind === "place" && sel.id === pl.p.key ? 20 : 0);
      l.cand.force = false;
      cands.push(l.cand);
    }
    // Time levels: an altimeter at the right of the view, at the target's depth, so its marks read top to bottom at
    // the heights the threads around the target have; never in the way of the other labels.
    const ct = Math.cos(cam.theta), st = Math.sin(cam.theta);
    // Near the right edge of the free view (the target is centred in it; a card takes the right).
    const reach = Math.max(40, (W - (io.sel() ? io.cardWidth() : 0)) / 2 - 110) * (2 * cam.dist * TAN / H);
    let lastY = -1e9;
    for (const lv of L.scale.levels) {
      const l = lab("lv:" + lv.days, lv.label, "level" + (lv.days === 0 ? " today" : ""), 11, true);
      if (cam.phi < 0.2) continue; // from above, heights do not show
      const s = toScreen(cam.tx + ct * reach, lv.y, cam.tz - st * reach);
      if (!s) continue;
      const x = Math.min(s[0], W - 75 - (io.sel() ? io.cardWidth() : 0));
      // Top first, each under the last: one too close to the one above is left out.
      if (x > 0 && s[1] > 10 && s[1] < H - 30 && s[1] - lastY > 15) {
        l.sx = x; l.sy = s[1] + 7; lastY = s[1];
        l.cand.w = tw(lv.label) * 11 + 24; // the dash before it
        // Placed first (forced), so the titles keep clear of it.
        l.cand.x = x + l.cand.w / 2; l.cand.y = l.sy - 14 + l.cand.h; l.cand.force = true; l.cand.prio = 100;
        cands.push(l.cand);
      }
    }
    placeLabels(cands, MAX_LABELS + L.places.length, box, 6);
    for (const c of cands) if (c.on) labs.get(c.key)!.target = 1;
  }

  /** Moves and fades the labels; true while any is still fading. */
  function stepLabels(dt: number): boolean {
    let moving = false;
    const k = instant ? 1 : 1 - Math.exp(-dt / 110);
    for (const [key, l] of labs) {
      const o = l.o + (l.target - l.o) * k;
      l.o = Math.abs(o - l.target) < 0.02 ? l.target : o;
      if (l.o !== l.target) moving = true;
      if (l.o <= 0) {
        if (l.el.style.display !== "none") l.el.style.display = "none";
        if (l.target === 0 && !L.byId.has(key.slice(2)) && key.startsWith("t:")) { l.el.remove(); labs.delete(key); }
        continue;
      }
      if (l.el.style.display === "none") l.el.style.display = "";
      l.el.style.opacity = l.o.toFixed(3);
      l.el.style.transform = l.level ? `translate(${l.sx.toFixed(1)}px,${(l.sy - 14).toFixed(1)}px)` : `translate(${(l.sx - l.cand.w / 2 + 4).toFixed(1)}px,${(l.sy - l.cand.h).toFixed(1)}px)`;
    }
    return moving;
  }

  // ---------------------------------------------------------------- the loop: on demand
  function want() {
    if (!active || destroyed || raf || (typeof document !== "undefined" && document.hidden)) return;
    if (pulseTimer) { clearTimeout(pulseTimer); pulseTimer = undefined; }
    raf = requestAnimationFrame(frame);
  }
  const quiet = () => performance.now() - lastInput;
  const rotating = () => !instant && !reduced && !io.sel() && !tween && quiet() > IDLE_MS && quiet() < IDLE_END_MS && pointers.size === 0;
  function frame(ts: number) {
    raf = 0;
    if (!active || destroyed || !W) return;
    const t0 = performance.now();
    const dt = lastTs ? Math.min(100, ts - lastTs) : 16;
    lastTs = ts;
    if (dataDirty) build(everShown && L.items.length > 0);
    if (styleDirty) restyle();
    let anim = false;
    if (stepTween(t0)) anim = true;
    if (Math.abs(vel.theta) + Math.abs(vel.phi) > 1e-5 && pointers.size === 0) {
      cam.theta += vel.theta * dt; cam.phi = Math.min(PHI_MAX, Math.max(0.0005, cam.phi + vel.phi * dt));
      const k = Math.exp(-dt / 260); vel.theta *= k; vel.phi *= k;
      if (Math.abs(vel.theta) + Math.abs(vel.phi) < 1e-5) vel.theta = vel.phi = 0;
      anim = true;
    }
    if (rotating()) { cam.theta -= ROTATE * dt / 1000; anim = true; }
    oxTarget = io.sel() ? io.cardWidth() / 2 : 0;
    if (Math.abs(ox - oxTarget) > 0.5) { ox = instant ? oxTarget : ox + (oxTarget - ox) * (1 - Math.exp(-dt / 140)); anim = true; } else ox = oxTarget;
    if (glide) {
      const t = Math.min(1, (t0 - glide.t0) / GLIDE_MS);
      U.uMix.value = smooth(t);
      refreshCur(U.uMix.value);
      projV = -1; labV = -1;
      if (t >= 1) glide = null;
      anim = true;
    }
    applyCam();
    if (labV !== camV) placeAll();
    if (stepLabels(dt)) anim = true;
    U.uTime.value = ts / 1000;
    renderer.render(scene, camera);
    frames++;
    frameMs = frameMs ? frameMs * 0.9 + (performance.now() - t0) * 0.1 : performance.now() - t0;
    if (anim) want();
    else if (hasPulse && !reduced && !instant && quiet() < 30_000) pulseTimer = setTimeout(() => { pulseTimer = undefined; want(); }, 70);
    else lastTs = 0;
    if (!anim && !io.sel() && !instant && !reduced && quiet() < IDLE_MS) {
      // Wake for the idle rotation once the user has been quiet long enough.
      const left = IDLE_MS - (performance.now() - lastInput);
      if (left > 0 && !pulseTimer) pulseTimer = setTimeout(() => { pulseTimer = undefined; want(); }, left + 20);
    }
  }

  function resize() {
    const r = host.getBoundingClientRect();
    if (!r.width || !r.height) return; // hidden: keep everything as it is
    const nw = Math.round(r.width), nh = Math.round(r.height);
    const nd = Math.min(window.devicePixelRatio || 1, 2);
    if (nw === W && nh === H && nd === dpr) return;
    W = nw; H = nh; dpr = nd;
    renderer.setPixelRatio(dpr);
    renderer.setSize(W, H, false);
    canvas.style.width = W + "px"; canvas.style.height = H + "px";
    camera.aspect = W / H;
    U.uScale.value = (H * dpr) / (2 * TAN);
    U.uDpr.value = dpr;
    uRes.value.x = W * dpr / 2; uRes.value.y = H * dpr / 2;
    applyCam();
    want();
  }

  // ---------------------------------------------------------------- input: orbit, pan, zoom; tap, double-tap
  const pointers = new Map<number, { x: number; y: number }>();
  let drag: { mode: "orbit" | "pan"; x: number; y: number; moved: boolean; t: number; button: number } | null = null;
  let pinch: { d: number; mx: number; my: number } | null = null;
  let lastTap = { t: 0, x: 0, y: 0 };
  const touched = () => { lastInput = performance.now(); tween = null; };
  const local = (e: { clientX: number; clientY: number }) => { const r = canvas.getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; };
  function pan(dx: number, dy: number) {
    const wpp = (2 * cam.dist * TAN) / H;
    const st = Math.sin(cam.theta), ct = Math.cos(cam.theta);
    // Along the ground: right is (cos θ, 0, -sin θ), screen-up is away from the camera (-sin θ, 0, -cos θ).
    cam.tx += -dx * wpp * ct - dy * wpp * st;
    cam.tz += dx * wpp * st - dy * wpp * ct;
  }
  function zoomAt(f: number, x: number, y: number) {
    const nd = Math.min(7000, Math.max(12, cam.dist * f));
    f = nd / cam.dist;
    // Toward the point under the cursor, on the plane through the target facing the camera.
    const hh = cam.dist * TAN, hw = hh * (W / H);
    const nx = ((x + ox) / W) * 2 - 1, ny = 1 - (y / H) * 2;
    const e = camera.matrixWorld.elements; // columns: right (0..2), up (4..6)
    const k = 1 - f;
    cam.tx += (e[0]! * nx * hw + e[4]! * ny * hh) * k;
    cam.ty += (e[1]! * nx * hw + e[5]! * ny * hh) * k;
    cam.tz += (e[2]! * nx * hw + e[6]! * ny * hh) * k;
    cam.ty = Math.max(-50, Math.min(HGT * 1.5, cam.ty));
    cam.dist = nd;
  }
  function onDown(e: PointerEvent) {
    canvas.setPointerCapture?.(e.pointerId);
    pointers.set(e.pointerId, local(e));
    touched();
    vel.theta = vel.phi = 0;
    if (pointers.size === 1) {
      const p = local(e);
      const panning = e.button === 2 || e.button === 1 || e.shiftKey || e.ctrlKey || e.metaKey;
      drag = { mode: panning ? "pan" : "orbit", x: p.x, y: p.y, moved: false, t: performance.now(), button: e.button };
    } else if (pointers.size === 2) {
      const [a, b] = [...pointers.values()];
      pinch = { d: Math.hypot(a!.x - b!.x, a!.y - b!.y), mx: (a!.x + b!.x) / 2, my: (a!.y + b!.y) / 2 };
      if (drag) drag.moved = true;
    }
    if (hoverIdx >= 0 && e.pointerType === "touch") setHover(-1);
  }
  function onMove(e: PointerEvent) {
    const p = local(e);
    if (!pointers.has(e.pointerId)) {
      if (e.pointerType !== "touch") {
        lastInput = performance.now();
        setHover(pickAt(p.x, p.y));
        if (!raf && !pulseTimer) want(); // re-arms the idle timers after a long quiet
      }
      return;
    }
    pointers.set(e.pointerId, p);
    touched();
    if (pointers.size >= 2 && pinch) {
      const [a, b] = [...pointers.values()];
      const d = Math.hypot(a!.x - b!.x, a!.y - b!.y), mx = (a!.x + b!.x) / 2, my = (a!.y + b!.y) / 2;
      if (d > 0 && pinch.d > 0) zoomAt(pinch.d / d, mx, my);
      pan(mx - pinch.mx, my - pinch.my);
      pinch = { d, mx, my };
      want();
      return;
    }
    if (!drag) return;
    const dx = p.x - drag.x, dy = p.y - drag.y;
    if (!drag.moved && Math.hypot(dx, dy) < 5) return;
    if (!drag.moved) { drag.moved = true; setHover(-1); }
    drag.x = p.x; drag.y = p.y;
    if (drag.mode === "pan") pan(dx, dy);
    else {
      const k = 2.6 / H;
      cam.theta -= dx * k; cam.phi = Math.min(PHI_MAX, Math.max(0.0005, cam.phi - dy * k));
      vel.theta = -dx * k / 16; vel.phi = -dy * k / 16;
    }
    want();
  }
  function onUp(e: PointerEvent) {
    const p = local(e);
    pointers.delete(e.pointerId);
    if (pointers.size < 2) pinch = null;
    const d = drag;
    if (pointers.size === 0) drag = null;
    if (!d || d.moved || performance.now() - d.t > 600 || d.button !== 0 && e.pointerType === "mouse") { if (d && d.mode === "pan") vel.theta = vel.phi = 0; want(); return; }
    vel.theta = vel.phi = 0;
    // Shift+click: in or out of the scope tray, the camera where it is (a shift-drag still pans).
    if (e.shiftKey && io.togglePick) {
      const k = pickAt(p.x, p.y);
      if (k >= 0 && !L.items[k]!.t.synthetic) { io.togglePick(selOfItem(L.items[k]!)); styleDirty = true; }
      want();
      return;
    }
    // A tap: the second one close in time and place opens what the first selected.
    const now = performance.now();
    const dbl = now - lastTap.t < 380 && Math.hypot(p.x - lastTap.x, p.y - lastTap.y) < 24;
    lastTap = dbl ? { t: 0, x: 0, y: 0 } : { t: now, x: p.x, y: p.y };
    canvas.focus({ preventScroll: true });
    if (dbl) { const s = io.sel(); if (s && s.kind !== "place") io.open(s); return; }
    const i = pickAt(p.x, p.y);
    if (i < 0) { io.select(null); return; }
    const it = L.items[i]!;
    const prevEff = effortIdx(io.sel());
    io.select(selOfItem(it));
    if (it.root || prevEff !== it.eff) flyToEffort(it.eff);
  }
  function onWheel(e: WheelEvent) {
    e.preventDefault();
    touched();
    const unit = e.deltaMode === 1 ? 33 : e.deltaMode === 2 ? H : 1;
    const f = Math.exp(e.deltaY * unit * (e.ctrlKey ? 0.01 : 0.0016));
    const p = local(e);
    zoomAt(f, p.x, p.y);
    want();
  }
  function onKey(e: KeyboardEvent) {
    const step = 0.08;
    if (e.key === "Enter") { const s = io.sel(); if (s && s.kind !== "place") io.open(s); e.preventDefault(); return; }
    const k = e.key;
    if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "+", "=", "-"].includes(k)) return;
    e.preventDefault();
    touched();
    if (k === "+" || k === "=") zoomAt(0.85, W / 2 - ox, H / 2);
    else if (k === "-") zoomAt(1 / 0.85, W / 2 - ox, H / 2);
    else if (e.shiftKey) pan(k === "ArrowLeft" ? 40 : k === "ArrowRight" ? -40 : 0, k === "ArrowUp" ? 40 : k === "ArrowDown" ? -40 : 0);
    else if (k === "ArrowLeft") cam.theta += step;
    else if (k === "ArrowRight") cam.theta -= step;
    else cam.phi = Math.min(PHI_MAX, Math.max(0.0005, cam.phi + (k === "ArrowUp" ? -step : step)));
    want();
  }
  function setHover(i: number) {
    if (i === hoverIdx) return;
    hoverIdx = i;
    canvas.style.cursor = i >= 0 ? "pointer" : "grab";
    io.hover(i >= 0 ? selOfItem(L.items[i]!) : null);
    styleDirty = true;
    want();
  }
  const onLeave = () => { if (!pointers.size) setHover(-1); };
  const noMenu = (e: Event) => e.preventDefault();
  const onVis = () => { if (!document.hidden) want(); };
  const onLost = (e: Event) => { e.preventDefault(); };
  const onRestored = () => { dataDirty = true; want(); };
  canvas.addEventListener("pointerdown", onDown);
  canvas.addEventListener("pointermove", onMove);
  canvas.addEventListener("pointerup", onUp);
  canvas.addEventListener("pointercancel", onUp);
  canvas.addEventListener("pointerleave", onLeave);
  canvas.addEventListener("wheel", onWheel, { passive: false });
  canvas.addEventListener("keydown", onKey);
  canvas.addEventListener("contextmenu", noMenu);
  canvas.addEventListener("webglcontextlost", onLost);
  canvas.addEventListener("webglcontextrestored", onRestored);
  document.addEventListener("visibilitychange", onVis);
  const ro = new ResizeObserver(resize);
  ro.observe(host);
  canvas.style.cursor = "grab";

  // ---------------------------------------------------------------- public
  const space: Space = {
    update(kind) {
      if (kind === "data") dataDirty = true;
      styleDirty = true;
      want();
    },
    selected(from) {
      styleDirty = true;
      const s = io.sel();
      if (s && from !== "space") { if (active && !dataDirty) flyToSel(s); else pendingSel = s; }
      want();
    },
    redraw() { styleDirty = true; want(); },
    show(from2d) {
      active = true;
      resize();
      if (dataDirty) build(false);
      const first = !everShown;
      everShown = true;
      if (from2d !== undefined) {
        // From 2D: straight above what Islands showed (the same picture), then tilting into 3D.
        const pack = io.layout();
        const whole = !from2d || !pack || from2d.w >= pack.size * 0.85;
        const end = whole ? overview() : (() => {
          const c = toWorld(pack!, from2d!.x, from2d!.y);
          const half = (from2d!.w * S) / 2;
          return { tx: c.x, ty: HGT * 0.3, tz: c.z, dist: Math.max(120, half / (TAN * (W / H)) * 1.05), theta: 0, phi: 0.9 };
        })();
        const top: Cam = { ...end, theta: whole ? 0 : end.theta, phi: 0.0005, ty: HGT * 0.3, dist: whole ? fitDist(L.radius) * 1.05 : end.dist / 1.05 };
        if (instant) { Object.assign(cam, end); applyCam(); }
        else { Object.assign(cam, top); applyCam(); flyTo(end, 1100); }
      } else if (first) { Object.assign(cam, overview()); applyCam(); }
      // A selection made elsewhere while hidden (the sidebar opened a conversation): fly there, unless coming from 2D,
      // which was already looking at it.
      if (pendingSel) {
        const s = pendingSel, now = io.sel();
        pendingSel = null;
        if (from2d === undefined && now && now.kind === s.kind && now.id === s.id) flyToSel(s);
      }
      lastInput = performance.now();
      styleDirty = true;
      labV = -1;
      want();
    },
    hide() {
      active = false;
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
      if (pulseTimer) clearTimeout(pulseTimer);
      pulseTimer = undefined;
      lastTs = 0;
      setHover(-1);
    },
    destroy() {
      space.hide();
      destroyed = true;
      ro.disconnect();
      document.removeEventListener("visibilitychange", onVis);
      for (const o of [nodes, glow, marks, edges, ground, disc]) { o.geometry.dispose(); (o.material as ShaderMaterial).dispose(); }
      renderer.dispose();
      canvas.remove(); labelLayer.remove();
    },
    topDown() {
      touched();
      flyTo({ tx: 0, ty: HGT * 0.3, tz: 0, dist: fitDist(L.radius) * 1.06, theta: 0, phi: 0.0005 }, 1000);
    },
    overview() { touched(); flyTo(overview(), 1000); },
    target() {
      const pack = io.layout();
      return toPack(pack ?? { size: 1000 }, cam.tx, cam.tz);
    },
    debug: {
      stats(): SpaceStats {
        const info = renderer.info.render;
        let shown = 0;
        for (const l of labs.values()) if (l.o > 0.5) shown++;
        return { drawCalls: info.calls, triangles: info.triangles, points: info.points, nodes: L.items.length, edges: edgeChild.length, edgeInstances: (edges.geometry as InstancedBufferGeometry).instanceCount ?? 0, labels: shown, frameMs, frames };
      },
      camera: () => ({ target: [cam.tx, cam.ty, cam.tz], dist: cam.dist, theta: cam.theta, phi: cam.phi }),
      pick: (x, y) => { const i = pickAt(x, y); return i >= 0 ? selOfItem(L.items[i]!) : null; },
      probe: () => { const g = edges.geometry; const f = (n: string) => Array.from((g.getAttribute(n)?.array ?? []) as Float32Array).slice(0, 6); return { ecol: f("ecol"), eprop: f("eprop"), a1p: f("a1p"), b1p: f("b1p"), c1p: f("c1p"), born: f("born"), uRes: uRes.value, mix: U.uMix.value }; },
      labels: () => [...labs.entries()].filter(([, l]) => l.o > 0.5).map(([k, l]) => k + "=" + l.text),
    },
  };
  return space;
}
