// A mannequin stand-in beside the conversation: a few shapes, turning.
// Its own module, so a WebGL failure leaves the rest of the page working.
import * as THREE from 'three';

const el = document.getElementById('stage');
const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
renderer.setPixelRatio(window.devicePixelRatio);
el.appendChild(renderer.domElement);
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera(35, 1, 0.1, 100);
camera.position.set(0, 1.1, 4.2);
scene.add(new THREE.HemisphereLight(0xffffff, 0x444466, 1.2));
const sun = new THREE.DirectionalLight(0xffffff, 1.5);
sun.position.set(2, 3, 2);
scene.add(sun);

const skin = new THREE.MeshStandardMaterial({ color: 0xd9c2a7, roughness: 0.7 });
const dress = new THREE.MeshStandardMaterial({ color: 0xb02a37, roughness: 0.5 });
const body = new THREE.Group();
const part = (geo, mat, y) => { const m = new THREE.Mesh(geo, mat); m.position.y = y; body.add(m); return m; };
part(new THREE.SphereGeometry(0.16, 24, 16), skin, 1.75);
part(new THREE.CylinderGeometry(0.06, 0.07, 0.12, 16), skin, 1.58);
part(new THREE.CylinderGeometry(0.22, 0.17, 0.5, 24), dress, 1.3);
part(new THREE.ConeGeometry(0.5, 0.95, 32, 1, true), dress, 0.62);
part(new THREE.CylinderGeometry(0.03, 0.03, 0.6, 8), skin, -0.15);
scene.add(body);

function size() {
  const w = el.clientWidth, h = el.clientHeight;
  renderer.setSize(w, h);
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
}
window.addEventListener('resize', size);
size();
renderer.setAnimationLoop((t) => {
  body.rotation.y = t / 2000;
  renderer.render(scene, camera);
});
