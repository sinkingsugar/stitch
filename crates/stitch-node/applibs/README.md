# Bundled libraries for apps

Served at `/_lib/<file>` on every app's origin (design §2.10), offline:
edge talk's whole set (`EdgeTalk/Resources/JSLibraries`), each file gzipped
(`gzip -9 -n`). The node keeps them in its binary (`src/apps/libs_list.rs`,
made from this directory). `/_lib/leadsheet.js`, the leadsheet stack's
loader, is Stitch's own (`src/apps/leadsheet.js`, edge talk's
`leadsheetLoader`).

To add or update one: put `<file>.gz` here, regenerate `libs_list.rs` (one
`(name, include_bytes!)` entry per file), list it in the `apps` skill
(`src/apps/SKILL.md`), and add its license text to `notices/applibs.txt`.

**MeshRepair is our own** (not edge talk's GPL VCGlib build): the
`meshrepair` repository (<https://github.com/sinkingsugar/meshrepair>,
checked out at `../meshrepair`; Rust compiled to WebAssembly, MIT OR
Apache-2.0), the same browser API. Its `scripts/build.sh <this dir>`
writes the three `.gz` files here; they were built from meshrepair commit
`69b677435beee86ea900db30634783f91c5eb102`. The wasm is fetched from
`/_lib/meshrepair.wasm`.

| File | Library | License |
|---|---|---|
| `chart.min.js` | Chart.js | MIT |
| `d3.min.js` | D3 | ISC |
| `mermaid.min.js` | Mermaid | MIT |
| `leaflet.min.js`, `leaflet.min.css` | Leaflet | BSD-2-Clause |
| `katex.min.js`, `katex.min.css` | KaTeX (without its fonts) | MIT |
| `three.module.min.js` | Three.js r170 | MIT |
| `OrbitControls.min.js` | Three.js's OrbitControls | MIT |
| `rough.min.js` | Rough.js | MIT |
| `paper.min.js` | Paper.js | MIT |
| `tone.min.js` | Tone.js | MIT |
| `p5.min.js` | p5.js | LGPL-2.1 |
| `litegraph.min.js`, `litegraph.min.css` | LiteGraph.js | MIT |
| `uplot.min.js`, `uplot.min.css` | uPlot | MIT |
| `math.min.js` | Math.js | Apache-2.0 |
| `matter.min.js` | Matter.js | MIT |
| `nexusui.min.js` | NexusUI | MIT |
| `lil-gui.min.js` | lil-gui | MIT |
| `glslcanvas.min.js` | GlslCanvas | MIT |
| `jscad-modeling.min.js` | JSCAD modeling | MIT |
| `jscad-regl-renderer.min.js` | JSCAD regl renderer | MIT |
| `jscad-stl-serializer.min.js` | JSCAD STL serializer | MIT |
| `meshrepair-loader.min.js`, `meshrepair.min.js`, `meshrepair.wasm` | meshrepair (Fragcolor's, Rust to WebAssembly; github.com/sinkingsugar/meshrepair, commit `69b6774`) | MIT OR Apache-2.0 |
| `mqtt.min.js` | MQTT.js | MIT |
| `pdfmake.min.js` | pdfmake | MIT |
| `pdfmake-vfs-fonts.js` | pdfmake's Roboto | Apache-2.0 |
| `xlsx.full.min.js` | SheetJS Community Edition | Apache-2.0 |
| `html-docx.min.js` | html-docx-js | MIT |
| `leadsheet_wasm.js`, `leadsheet_wasm_bg.wasm` | leadsheet compiler (Fragcolor's, from edge talk) | Stitch's (LICENSE) |
| `spessasynth.min.js`, `spessasynth_processor.min.js` | SpessaSynth | Apache-2.0 |
| `GeneralUserGS.sf3` | GeneralUser GS 2.0.3 soundfont, by S. Christian Collins | GeneralUser GS License v2.0 (free use and distribution) |

Their license texts and attributions are in `notices/applibs.txt` (see
`notices/README.md`): a change here changes that file too.

p5.js (LGPL) is carried as a file the node serves, run by the web view,
never linked or called by Stitch: an aggregate, as edge talk's app bundle
has it.
