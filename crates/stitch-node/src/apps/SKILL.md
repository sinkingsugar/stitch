---
name: apps
description: Making apps for the user (dashboards, tools, calculators, charts, games, documents to download): HTML, CSS and JavaScript shown with show_app, on the phone and in a browser, with state that syncs across their devices.
---

# Apps

An app is a directory with `index.html` (and whatever else it loads: scripts, styles, images, data, WebAssembly), or one HTML file. Write its files with write_file, then call `show_app` with its path and a name. The user sees it in the conversation, on the phone and in a browser on their computers, and can pin it to their home screen.

## Versions

Each `show_app` is a version, frozen: editing the files changes nothing shown until you call `show_app` again (same name). The conversation keeps every version where it was shown; a pinned app moves to the new version by itself (same name), keeping its data, unless the user pinned it at a fixed version.

Keep an app in a directory of its own (`apps/<name>/` in the place; in a thread with no place, its working folder, where relative paths start; or a temporary one), with only what it loads: `show_app` takes the whole directory (hidden files, `node_modules`, `target`, `build`, `dist` left out), at most 2000 files and 64 MiB. A global thread's working folder syncs between the user's nodes, so relative app directories follow the thread when it continues elsewhere.

`stitch app export <thread | pinned name | blake3:version> [dir]` writes a shown version's files to a directory (default `./<name>`; `--force` to write over one that is not empty): the way to get an app's source back when its files are not on this node.

## Pages

- Write whole pages: `<!doctype html>`, `<head>`, `<body>`. Stitch puts its bridge first in the `<head>` and adds a viewport tag when there is none.
- Each app has an origin of its own: load its files by relative paths (`js/main.js`, `data.json`); `fetch` works on them.
- Without `networking`, pages reach only their own files and the bundled libraries: no CDN, no external images, no map tiles, no APIs. Set `networking: true` when the app needs them.
- Links to other sites open in the browser (on the phone, Safari); the app stays.
- Look: link `/_stitch/stitch.css` for Stitch's look, light and dark with the device (`--bg`, `--bg2`, `--fg`, `--fg2`, `--line`, `--accent`, `.card`), or style it yourself; respect `prefers-color-scheme`. Phones are narrow: lay out for 360 px wide first. In the conversation an app shows at most 600 points tall, scrolling inside; full screen it has the whole screen.

## Bundled libraries (offline)

Load them with a tag from `/_lib/` (`<script src="/_lib/chart.min.js"></script>`, `<link rel="stylesheet" href="/_lib/leaflet.min.css">`), in the order given where there are several:

- **Chart.js** `chart.min.js` — `new Chart(canvas, config)` — bar, line, pie, doughnut, radar, scatter, bubble charts
- **D3.js** `d3.min.js` — `d3.select(...)` — advanced data visualization, SVG manipulation
- **Mermaid** `mermaid.min.js` — `<pre class="mermaid">graph TD; A-->B</pre>`, then `mermaid.initialize({ startOnLoad: true })` — flowcharts, sequence diagrams, gantt, class diagrams
- **Leaflet** `leaflet.min.css`, `leaflet.min.js` — `L.map(...)` — interactive maps (tile layers need `networking`)
- **KaTeX** `katex.min.css`, `katex.min.js` — `katex.render(expr, element)` — LaTeX math (its fonts are not bundled: system fonts stand in)
- **Three.js** — `import * as THREE from 'three'` in a `<script type="module">` — 3D graphics, WebGL scenes, animations. OrbitControls: `import { OrbitControls } from 'three/addons/controls/OrbitControls.js'`. Stitch gives every page an import map for the bundled modules: write none of your own (a page takes one, and yours would replace it)
- **Rough.js** `rough.min.js` — `rough.canvas(canvas).rectangle(...)` — hand-drawn, sketchy graphics
- **Paper.js** `paper.min.js` — `paper.setup(canvas)` — vector graphics scripting, paths, animations
- **Tone.js** `tone.min.js` — `new Tone.Synth().toDestination()` — Web Audio: synths, effects, scheduling
- **p5.js** `p5.min.js` — `function setup() { createCanvas(...) }` — creative coding, generative art
- **LiteGraph.js** `litegraph.min.css`, `litegraph.min.js` — `new LGraph()` — node-based graph editor, visual programming
- **uPlot** `uplot.min.css`, `uplot.min.js` — `new uPlot(opts, data, el)` — fast time-series and scatter charts
- **JSCAD** `jscad-modeling.min.js`, then `jscad-regl-renderer.min.js` for its WebGL viewer — `jscadModeling.primitives.cube(...)` — parametric 3D CAD modeling
- **JSCAD STL serializer** `jscad-stl-serializer.min.js`, after `jscad-modeling.min.js` — `jscadStlSerializer.serialize({ binary: true }, model)` — STL export for JSCAD geometries (not part of `@jscad/modeling`); returns an array of `ArrayBuffer` chunks
- **MeshRepair** `meshrepair-loader.min.js`, then `meshrepair.min.js` — `await initMeshRepair()` — STL mesh repair in WebAssembly: welds vertices, fixes holes, T-junctions, non-manifold edges and vertices, normals (outward, cavities inward), duplicate and degenerate faces; the result reports `watertight` and `manifold`. Presets `'minimal'`, `'print-ready'`, `'aggressive'`. Offline
- **GlslCanvas** `glslcanvas.min.js` — `new GlslCanvas(canvas)` — GLSL shaders on a canvas
- **Matter.js** `matter.min.js` — `Matter.Engine.create()` — 2D rigid body physics
- **NexusUI** `nexusui.min.js` — `new Nexus.Dial('#el')` — audio UI components (dials, sliders, sequencers)
- **lil-gui** `lil-gui.min.js` — `new lil.GUI()` — a floating panel for tweaking variables
- **MQTT.js** `mqtt.min.js` — `mqtt.connect(url)` — IoT and microcontroller telemetry over WebSockets (needs `networking`)
- **Math.js** `math.min.js` — `math.evaluate(expr)` — mathematics, matrices, symbolic computation, units
- **pdfmake** `pdfmake.min.js`, then `pdfmake-vfs-fonts.js` — `pdfMake.createPdf(docDefinition).download('file.pdf')` — PDF documents with real pages: tables that never clip across pages, repeating headers, footers, page numbers. Its own layout engine, identical on every device: use it (not `window.print()`) for contracts, invoices, reports, anything multipage (see Documents below)
- **html-docx-js** `html-docx.min.js` — `htmlDocx.asBlob(html)` — an editable Word `.docx`. Word and Google Docs open it, Apple Pages and LibreOffice do not: offer it next to the PDF, never as the only output
- **SheetJS** `xlsx.full.min.js` — `XLSX.utils.aoa_to_sheet(rows)` and `XLSX.writeFile(wb, 'file.xlsx')` — real Excel files (sheets, formulas; also reads uploaded .xlsx and .csv): cost tables, budgets, anything for Numbers or Excel
- **leadsheet** `leadsheet.js` — `await initLeadsheet()` → `{check, render, fmt, WorkletSynthesizer, Sequencer, createSynth(audioCtx)}` — multitrack music as `.ls` text, checked, rendered to MIDI and played through a bundled GM soundfont (GeneralUser GS). Offline. For music, follow the `leadsheet` skill (read_skill) and its player template instead of making one

Other libraries: copy them into the app's directory (with `networking`, a CDN works too).

## 3D printing (JSCAD → STL)

When the user asks for JSCAD models meant for 3D printing (or anything that will be printed), always:

1. Give them a **download button** that exports STL, not just a viewer.
2. **Run MeshRepair on the STL before the download** (`'print-ready'`), so the file is watertight, manifold, with correct normals, ready for any slicer. Do not skip it even if the JSCAD output looks clean: JSCAD's serializer emits duplicate vertices and at times non-manifold edges that slicers hate.
3. Show the repair stats (faces before and after, holes filled), so the user sees what was fixed.

```html
<button id="download">Download print-ready STL</button>
<pre id="stats"></pre>
<script src="/_lib/jscad-modeling.min.js"></script>
<script src="/_lib/jscad-stl-serializer.min.js"></script>
<script src="/_lib/meshrepair-loader.min.js"></script>
<script src="/_lib/meshrepair.min.js"></script>
<script>
// An async function: a classic <script> has no top-level await.
(async () => {
// 1. The geometry, with JSCAD.
const { cube } = jscadModeling.primitives;
const model = cube({ size: 20 });

// 2. Binary STL: chunks of ArrayBuffer, made one Uint8Array.
const chunks = jscadStlSerializer.serialize({ binary: true }, model);
const total = chunks.reduce((n, c) => n + c.byteLength, 0);
const rawBytes = new Uint8Array(total);
let off = 0;
for (const c of chunks) {
  rawBytes.set(c instanceof Uint8Array ? c : new Uint8Array(c), off);
  off += c.byteLength;
}

// 3. Repaired for printing: never skipped.
const mr = await initMeshRepair();
const { result, output } = mr.repair('model.stl', rawBytes, 'print-ready');
document.getElementById('stats').textContent =
  `${result.originalFaces} → ${result.finalFaces} faces, ` +
  `${result.holesFilled} holes filled, ` +
  `${result.duplicateFacesRemoved} dup / ${result.degenerateFacesRemoved} deg removed`;

// 4. The repaired STL, to the user.
document.getElementById('download').onclick = () => {
  const url = URL.createObjectURL(new Blob([output], { type: 'application/octet-stream' }));
  const a = document.createElement('a');
  a.href = url; a.download = 'model-print-ready.stl';
  a.click();
  URL.revokeObjectURL(url);
};
})();
</script>
```

Show the model too: JSCAD's renderer, or Three.js from the same geometry. Repairing an STL the user picks (`<input type="file">`, a repaired download) runs on the device as well, without `networking`.

## State

What an app keeps belongs to its conversation and syncs to every device of the user's: every version and the pinned app share it.

- `localStorage` works as usual (`getItem`, `setItem`, `removeItem`), and reads at once: the page starts with it.
- `stitch.storage` keeps JSON values: `await stitch.storage.set('todos', list)`, `await stitch.storage.get('todos')`, `remove`, `keys`, `all`, `clear`, and `flush()` to write now.
- Writes are saved within a second or two. When another device or page changes a key, the page gets a `storage` event (`window.addEventListener('storage', e => …)`, `e.key`, `e.newValue`) and `stitch.storage.onChange((key, value) => …)`: re-render from it.
- Keep state under a few stable keys (`settings`, `items`), not one key per keystroke; a value holds at most 256 KB. `sessionStorage` is the tab's, as usual (it lasts across reloads), never synced.

## Talking to you

- `stitch.report(data)` sends you data (text, or anything JSON) from the app, at most every 2 seconds: a form's answers, a choice, a measurement. It wakes this conversation (even after you reported), on whichever device runs it. Its promise resolves once the report is on its way (`{ok, said}`) and rejects with the reason when it cannot go (too soon, the device running this conversation not reachable now): show the user either, not a label that never changes. A pinned app reports too, to the conversation it was pinned from (this one): its report says "Pinned app" and its version, and the user is likely not looking at the chat, so do the work (refresh data, show a new version) rather than only answer.
- When a page fails as the user opens it (an uncaught error, a script that does not load), you get its first error once: fix the files and show the app again. The page shows the error on a red bar meanwhile.

## The node's live state

For dashboards and monitors: never collect facts with your tools into a data file (stale at once, and a model call per refresh). The page reads them live, read-only, once the user allows it (the phone asks them natively on the first call; in a browser the call fails with `e.consent` `'ask'` and `e.allow`, the command that allows it: show `e.message`):
- `await stitch.live.threads({all: true, status, since, limit})`: `{node, threads: [{id, title, goal, node, place, parent, read_only, state: working|blocked|idle|ended|cancelled, outcome, question: {id, command}, started, last_step, model_calls, cost, last_words}], more}`. Without `all`, the threads this conversation started. With `all: true, conversations: true` the whole forest, as the user's own list has it: every conversation (chat) and thread, a compaction chain as its newest, deleted ones left out, the latest activity first, `limit` up to 5000 (500 otherwise; 100 by default). Each row also has `detached` (false for a conversation), `pinned`, `unread`, `digest: {headline, asks, questions, at, fresh}` when it has one, and `parent`: the thread that spawned it (or that it reports to), followed to the newest of its compaction chain, so it names a listed row, except when that parent was deleted (`parent_title` says `A deleted thread`) or lives in a repository this node does not hold: then the id is not among the rows, so do not draw it as a root without saying so. In this listing `goal` is cut at 300 characters and rows carry no `in_flight` or `app`. A node older than this ignores `conversations` and gives the detached threads alone: no row with `detached: false` tells you so. About 0.5 s for 1,500 rows, 1.2 s the first time after the node starts.
- `await stitch.live.attention()`: `{questions: [{id, thread, thread_title, node, place, session, command, approval, asked}]}`.
- `await stitch.live.jobs()`: `{node, jobs: [{job, kind, thread, thread_title, session, command, condition, running, how, started, ended, last_lines}]}` (a phone has none).
- `await stitch.live.schedules({all})`: `{node, schedules: [{id, name, goal, when, at, every, tz, until, node, place, state: active|paused|ended, self_wake, thread, thread_title, threads, latest_thread, latest_title, check, read_only, model, budget, daily_cap, today, next: [times], firing, created, here: {last, fired, quiet, skipped, failures, last_said} on its own node}]}`, the soonest due first; self-wakes (`wake_at`) included; `all` adds the ended ones.
- `await stitch.live.looseEnds()`: `{node, checking, unjudged, last_pass: {at, node, due, judged, usd, model}, items: [{thread, title, kind: conversation|thread, verdict: waiting-on-you|continue|done|stale|null (a spawned thread: its outcome), line, since, unread, node, place}]}`, the latest first: the conversations that wait on the user (the quick model's judgement of their latest step, and those moved since the user last opened them). Changes come with `threads`. Read-only for your app: checking and dismissing are the built-in dashboard's.
- `await stitch.live.events({topic, limit})`: `{node, generation, events: [{id, topic, payload, at, ask: {deadline, state: pending|answered|expired|dropped|unheard, answer, by, note, schema, matched} or null}], subscriptions: [{id, name, on, node, state, thread, subscription, filter, debounce, max_per_hour, daily_cap, until, here: {events, dropped, limited, woke, last, last_said}}]}`: what outside programs published to this node (`stitch event`, `stitch-client`), the newest first (`topic` a pattern such as `ci/**`; `limit` 50 by default), kept a short while (32 per topic; an ask an hour after it is resolved), and the subscriptions (schedules with an event trigger). To answer an ask from a page, `stitch.report` to the thread that handles them.
- `stitch.live.on('threads' | 'jobs' | 'attention' | 'schedules' | 'events' | '*', () => reload())`: when that changed (also once allowed). Times are RFC 3339.
`stitch dashboard` is Stitch's own, built on these; copy its layout rather than reinvent it.

## Threads of its own

An app can run agent threads of its own (a studio where each project is a conversation with an agent, a tutor, a helpdesk): it creates them, talks to them and shows their replies, within what it declares and the user grants once. It sees only the threads it created, never the user's others.

Declare what its threads need in `stitch-app.json`, next to `index.html` (`show_app` checks it, and refuses what cannot be declared):

```json
{
  "threads": {
    "places": ["~/devel/atelier-projects", "global"],
    "tools": ["files", "viewer", "images", "web"],
    "skills": ["atelier"],
    "otter": ["render_garment"],
    "mcp_allow": [],
    "models": ["default", "quick"],
    "budget": {"usd": 2, "minutes": 60, "steps": 60, "per": "thread"},
    "daily_usd": 10,
    "concurrent": 2
  }
}
```

- `places`: where they work (`global`: a folder of each thread's own); the first is the default. `tools`: groups `files`, `viewer`, `images`, `web`, `decide` (closed questions answered by a decision model, priced to the thread), MCP servers or `server__tool` patterns; everything else is off. Not declarable: `shell` (declare container commands instead, below), `weave`, `memory`, `browser`, `apps` (do machine work through Otter or MCP tools the user installed). `skills`: the skills they get (none otherwise). `otter`: Otter tools of their skills that run (never `otter_run`: the language tools are off for app threads; their scripts have no `Http.*`). `mcp_allow`: MCP tools of this node's servers that run; no other MCP call does. `models`: the first is the default, and must have a known price. `budget` per thread (`per: "turn"` for a long-lived conversation: each turn gets it afresh), `daily_usd` for all, `concurrent` at work at once (defaults $2, $10, 2). Not declarable either: the `edge-talk` skill.
- Their threads read and write files only in their place; writing instructions or settings there (`.stitch/`, `AGENTS.md`, `CLAUDE.md`, `.github/`, …) or anything outside it asks the user, and the app cannot answer that. They reach only public web addresses, read pages without running their JavaScript (`web_eval` is off), and never run as yolo. Undeclared calls are refused, so declare what the work needs. Prefer a place of the app's own to a checkout the user builds.
- The user grants it once: `stitch app allow-threads "<name>" --thread <this thread>` shows them what it asks and the command, with `--needs <hash>`, that grants exactly that. A changed `threads` needs a new grant: say so when you change it. Until then calls fail with `e.consent` (`'ask'`, `'changed'`, `'denied'`), `e.allow` (the command) and `e.needs` (what it asks): show them in the page.

```js
const g = await stitch.threads.grant();            // {consent, needs, allow}
const t = await stitch.threads.create({goal: 'Design a red evening dress', title: 'Red dress', images: [file]});
await stitch.threads.send(t.id, 'Make the hem shorter', [photo]);  // File, Blob or data: URLs
const {messages, state} = await stitch.threads.messages(t.id, {after: lastN});
// messages: [{n, role: 'user'|'assistant'|'tool', text, images: [{url}], name, at}]; tool lines name the tool only
const {threads, spent_today, daily_usd} = await stitch.threads.list(); // state: working|approval|idle|done|cancelled
await stitch.threads.cancel(t.id);
stitch.threads.on(() => refresh());                // something moved: read again
```

- Write each thread's goal so it stands alone: it is the thread's first message, from `app "<name>"`. Keep the app's own data (project names, which thread is which project) in `stitch.storage`.
- Images: PNG, JPEG, WebP or GIF, at most 4 a message, 5 MiB each, 10 MiB together. Text at most 32 KB. Show `idle` as "your turn", `approval` as "waiting for <user> to approve on their devices".
- Limits answer 429 (too soon, too many at work, the day's money spent, the day's images) and 403 (a model, place or budget not granted): show `e.message`. A turn that runs out of money or time mid-way ends with `outcome: 'over-budget'`.
- A thread keeps its ID when it is compacted: its messages are its whole history.

## Its data folder

An app with threads can have a folder of its own for the user's files (projects, pictures, what its threads and commands make), apart from its code. Declare it under `threads` in `stitch-app.json`; `"places": ["data"]` makes it the threads' place:

```json
"threads": {"places": ["data"], "tools": ["files", "viewer"],
            "data": {"folder": "~/Apps/Atelier", "quota_mb": 4096, "local": ["out", "cache"]}}
```

- Default `~/Apps/<name>`; never inside the node's data or a git checkout. It syncs between the user's nodes like any folder, except its `local` subfolders (`out`, `cache` by default). The threads grant covers it.
- The page reads and writes it with `stitch.files` (paths relative, no `..`, no hidden parts, no links; at most 15 MiB a write; within the quota):

```js
const {files, used, quota} = await stitch.files.list('projects');   // [{path, size, modified}]
const p = await stitch.files.json('projects/a/project.json');       // also .text(), .read() (ArrayBuffer)
await stitch.files.write('projects/a/project.json', p);             // objects as JSON; strings, Blobs, ArrayBuffers as they are
await stitch.files.remove('projects/a/old.jpg');
loader.load(stitch.files.url('out/run-1/result/garment.glb'));     // a URL for Three.js, <img>
const mark = await stitch.files.mark();                             // changes when the files do: poll it
```

HTML and SVG are served as downloads, never as pages.

## Commands in a container (Linux, rootless podman)

When its threads must run programs (Blender, a converter), the app ships a `Containerfile` and declares commands; each runs in a container of the app's image with no network, the image read-only, as a user that owns nothing of the user's. Put the `Containerfile` and the tools it copies in `container/` of the app, and:

```json
"container": {
  "build": {"memory_mb": 4096, "minutes": 30},
  "commands": {
    "rebuild": {
      "description": "what it makes",
      "run": ["/opt/blender/blender", "--background", "--python", "/opt/app/tools/build.py", "--", "{recipe}", "{out}"],
      "args": {"recipe": {"type": "file", "dir": "projects", "ext": ["json"]}, "out": {"type": "out", "fresh": true}},
      "inputs": {"body": {"from": "~/devel/x/base", "files": {"rig.json": "<sha256>"}}},
      "timeout": 900, "memory_mb": 6144, "cpus": 8, "max_files": 16, "max_mb": 256
    }
  }
}
```

- `FROM` pins base images by digest (`name@sha256:…`); a download is checked by its hash (`RUN curl … && echo "<sha256>  f" | sha256sum -c -`, or `ADD --checksum=sha256:…`); no `--mount=type=secret|ssh`. The program is an absolute path in the image. Each `{arg}` stands alone as one argument: `file` (a file of the data folder, copied in read-only at `/in/args/<name>/`), `out` (`/out`, or `/out/result` with `fresh`), `enum`, `int` (`min`, `max`), `slug`. `inputs`: files from outside the app, each pinned by SHA-256, copied in read-only at `/in/<name>/`.
- The user grants the build (it runs the Containerfile with internet access) and the commands: `stitch app allow-container "<name>" --thread <this thread>` shows them, with the Containerfile, and the hashes to grant. A changed Containerfile or tool asks again (with the diff).
- Its threads get `app_command` (`{command, args}`). What the command writes to `/out` comes back into the data folder under `out/<command>-<time>-<id>/`; images are also attached to the thread's reply, so `stitch.threads.messages` shows them. Links, FIFOs and anything past the caps are left out.
- `stitch app storage` shows the images on the node; `--rm <name>` removes an app's.

## Files out and in

- Downloads work as on the web (`<a download>`, a Blob through `URL.createObjectURL`, SheetJS's `writeFile`, pdfmake's `.download()`): on the phone they open the share sheet, to save or send.
- `<input type="file">` picks files on the phone and on the computer; read them with `FileReader` or `file.text()`.

## Documents (PDF, Word, Excel)

When the user needs a document (a contract, an invoice, a report, a letter: anything to print, sign or archive), build it with pdfmake, not styled HTML and `window.print()`: printing paginates unpredictably (rows clipped, headings orphaned); pdfmake lays out its pages itself.

```html
<button id="pdf">Download PDF</button>
<script src="/_lib/pdfmake.min.js"></script>
<script src="/_lib/pdfmake-vfs-fonts.js"></script>
<script>
document.getElementById('pdf').onclick = () => {
  pdfMake.createPdf({
    pageSize: 'A4',
    pageMargins: [40, 60, 40, 60],
    footer: (page, total) => ({ text: `${page} / ${total}`, alignment: 'center', fontSize: 9 }),
    content: [
      { text: 'Bridge Agreement · Accordo Ponte', style: 'h1' },
      {
        table: {
          widths: ['*', '*'],
          headerRows: 1,            // repeats on every page
          dontBreakRows: true,      // a row never splits across pages
          body: [
            [{ text: 'English', bold: true }, { text: 'Italiano', bold: true }],
            ['The parties agree…', 'Le parti concordano…'],
          ],
        },
        layout: 'lightHorizontalLines',
      },
    ],
    styles: { h1: { fontSize: 18, bold: true, margin: [0, 0, 0, 12] } },
  }).download('bridge-agreement.pdf');
};
</script>
```

- `.download(name)` hands the PDF to the share sheet on the phone, to the downloads on a computer. On the phone `.open()` and `.print()` do the same as `.download()`: the user prints it from the system's viewer.
- **Never preview the generated PDF in the page**: no `<iframe>`, `<embed>` or `<object>` on a `blob:` or `data:` PDF, no `getDataUrl` into a frame. It shows as a blank box in the phone's web view while the download works, which makes it look like your bug.
- pdfmake's font is Roboto (Latin, Cyrillic, Greek). Chinese, Japanese and Korean text need a font of their own: say so rather than produce boxes.
- The preview is styled HTML made from the same data (the PDF is made from the data too, never a screenshot of the page). Its page breaks are not pdfmake's, and that is fine: the PDF is what the user gets.
- An editable Word version: `htmlDocx.asBlob('<h1>…</h1>…')`, then an `<a download="file.docx">` click, like the STL above (the Pages caveat in the list).
- Spreadsheets: a workbook and `XLSX.writeFile(wb, 'costs.xlsx')`; it downloads like any other file.

## Making it work

A single uncaught error at the top of a script stops everything after it; in a module, the whole module.

1. Declare first, run last: every `const`, `let` and function, then one init block at the end of the script (a function called early that reads a `const` declared later throws).
2. Keep plain UI (tabs, buttons, inputs) in its own classic `<script>`, apart from heavy code (Three.js, WebGL, audio), so a 3D failure leaves the rest working.
3. Use `type="module"` only to `import`.
4. Check elements exist before using them.
5. On a computer, open the address `show_app` gives with browse_web (a screenshot shows it; web_eval runs code in it) before telling the user it is done.
