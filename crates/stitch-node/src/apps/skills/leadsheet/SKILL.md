---
name: leadsheet
description: Compose multitrack music as leadsheet (.ls) text: validate, render, and play it in an app with an embedded MIDI player.
---

## Leadsheet — compose music as text

`.ls` is a compact source language for multitrack music, designed for you:
patterns, chord symbols, drum lanes, dynamics, arrangements — not raw MIDI
events. You author the song as text; a wasm build of the `leadsheet` compiler
(same core as the CLI) validates it and renders it to MIDI, and SpessaSynth +
a bundled GM soundfont (GeneralUser GS) play it. Everything ships with
Stitch — no network needed.

The whole stack is exposed in apps through `await initLeadsheet()` (load
`/_lib/leadsheet.js` first; the player template does), which resolves to:
`{ check, render, fmt, compress_midi, compress_jsonl, WorkletSynthesizer,
Sequencer, createSynth(audioCtx) }` — the same API surface as the playground.
`check(text)` returns a JSON string; `render(text)` returns MIDI bytes and
throws the check payload on invalid input.

**Ground rules**: the `.ls` format lives ONLY in this skill — it is not on
the web, and web search will return wrong or unrelated material ("lead
sheet" the jazz notation, ABC, MML…). NEVER web-search anything about the
format, the player, or the API. The authoritative sources are, in order:
this document, `references/FORMAT.md` (full spec), and `LS.check`
diagnostics. Read this skill's files with `read_skill` and `file`
(`{"name": "leadsheet", "file": "references/FORMAT.md"}`).

### Workflow

1. **Compose the song** as `.ls` text (format below; full spec in
   `references/FORMAT.md`, worked example in `references/pop.ls` — read them
   with `read_skill` when needed).
2. **Write the sidecar**: write the song as `<name>.ls` (write_file) next to
   the app, e.g. `apps/<name>/<name>.ls`. This is the source of truth: the
   user can take the file to the CLI or the playground.
3. **Write the player**: read `assets/player.html` with `read_skill`, then
   write a copy as `apps/<name>/index.html`, changing ONLY the contents of the
   `<script type="text/plain" id="ls-src">` block — paste the exact same song
   text there. Do not modify the rest of the template.
4. **Validate before telling the user it is ready.** On a computer, show the
   app (step 5) and check it at the address `show_app` gives with
   `web_eval`:

   ```
   web_eval({ url: "<the app's address>", code: `
     const LS = await initLeadsheet();
     return LS.check(document.getElementById('ls-src').textContent.replace(/^\n/, ''));
   ` })
   ```

   If `ok: false`, fix the song using the structured diagnostics (`line`,
   `col`, `code`, `message`, `suggestion`), sync both files, and show it
   again until it passes. On the phone there is no web_eval: the player's
   own check (the safety net below) is the validation.
5. **Show it**: `show_app` with `apps/<name>` and the song's name.
6. **Iterate**: on every edit, update `<name>.ls` first, sync the identical
   text into the `ls-src` block of the player (edit_file), then `show_app`
   again — each call shows a new version; editing the files never updates
   one already shown.

**The safety net**: the player also runs `check` on load. If an invalid song
ever reaches the user, it shows the first diagnostic AND reports the full
diagnostics back to you (`stitch.report`, a message
`{"leadsheet_check_failed": …, "result": {…, "diagnostics": [...]}}`). Trust
the diagnostics over your assumptions, fix both files, show it again. A
valid song shows `✓ bars · tracks · notes · BPM` instead.

### Format quick reference

```
song: demo  tempo: 96.00  meter: 4/4  key: Am  grid: 1/16
instruments: bass:33 drums:kit piano:0 lead:81

P1 bass   | A,,4 A,,4 G,,4 E,,4 |
P2 drums
  K |x... .... x.x. ....|
  S |.... x... .... x...|
  h |x.x. x.x. x.x. x.x.|
P3 piano* | Am . F G7 |
P4 lead   | e2 c2 d2 B2 c4 A4 |

arrangement:
  intro: [P1+P2] x2
  A:     [P1+P2+P3+P4] x4
```

**Header** — `tempo` must be spelled to hundredths (`96.00`); meter
denominator 4 or 8; `key:` optional (spelling hint only); optional
`swing: 66%` (50–75, offbeat 8ths) or `swing: 16th 58%`. `instruments:` maps
name → GM program number (`kit` = drum channel); declaration order is track
order.

**Time** — everything is in grid cells = 16th notes. A 4/4 bar = 16 cells, a
beat = 4 cells. Sub-cell via fractions (`e/2` = 32nd, `C3/2` = dotted 16th)
and tuplet groups (`(3 C D E)4` = 8th triplet over a beat). A pattern may
override the meter: `P5 drums 3/4` (all patterns stacked in one arrangement
row must agree).

**Melodic patterns** — `P1 bass | A,,4 ^F2 z2 [CEG]4 |`
- ABC pitch: `C` = middle C (C4), `c` = C5, `,` down an octave, `'` up.
- Accidentals ALWAYS explicit per note (`^` sharp, `_` flat, `=` natural);
  nothing is sticky, no key-signature inference.
- Duration = cell-count suffix, `1` implied (`e2` = 8th, `c4` = beat,
  `C16` = whole bar in 4/4). `z2` = rest. `[CEG]4` = chord (exact pitches).
- `-` ties into the next bar; ` & ` separates overlapping voices in a bar
  (each voice must sum to a full bar). Bars must sum exactly.

**Chordal patterns** — `P3 piano* | Am . F G7 |` (the `*` = chord mode)
- One column per beat; `.` = hold, `z` = rest.
- Symbols: `C Am G7 Fmaj7 Dm7 Bdim Caug Dsus4 Am7b5 C6 …`, slash bass `F/A`,
  octave hint `Am(2)` (bass octave, default 3). A symbol means its canonical
  close-stacked voicing from the bass note up.

**Drums** — `P2 drums` then one lane per voice: `x` hit, `.` empty, spaces
cosmetic. Lanes: `K` kick, `S` snare, `st` sidestick, `cp` clap, `h` closed
hat, `hp` pedal hat, `O` open hat, `C` crash, `R` ride, `rb` ride bell,
`T1..T6` toms, `tm` tambourine, `cb` cowbell, `dNN` = any GM key.

**Dynamics** — `@dyn` after the instrument (`P4 lead@mf`): `pp p mp mf f ff`
(unmarked = `f`). Per note: `>` accent, `~` ghost. Drum lanes: `X` accent,
`x` normal, `o` ghost; digits `2`/`3`/`4` in a cell = 32nd/triplet/64th
strokes (rolls); lane tuplets `(3:4xxx)` = 3 strokes over 4 cells.

**Variants** — `P8 drums ~P3` + only the lanes that differ (all-dots lane
silences an inherited one). For melodic/chordal, `~P7` is informational only.

**Arrangement** — one row per bar-stack: `label: [P1+P2+P3] xN` (`x1`
implied, labels optional and ignored by the parser, `[z]` = silent bar).
Rows play consecutively. Alternative for quick sketches: direct bars —
`b3 lead | … |` places content straight into bar 3, no arrangement needed.

**Comments** — `// margin note`, attaches to the next construct; never
affects the music.

**Automation** (filter sweeps, pitch bends, CCs) — `bind` + `@name { pos:val
ease … }` keyframe lanes; see `references/FORMAT.md`
§ Automation.

### Composing guidance

- Prefer one-bar patterns reused through the arrangement; use `~P` drum
  variants for fills instead of near-duplicate patterns.
- Give every song dynamics and at least one variation — an unmarked,
  loop-only song sounds mechanical. Swing, ghosts (`~`, `o`) and accents
  (`>`, `X`) are cheap expressiveness.
- Useful GM programs: 0 piano, 4 e-piano, 24 nylon gtr, 27 clean gtr,
  30 dist gtr, 33 fingered bass, 35 fretless, 38 synth bass, 48 strings,
  56 trumpet, 65 alto sax, 73 flute, 80 square lead, 81 saw lead, 88 pad.
- Bass lives around `C,,`–`C,`; leads read best `c`–`c'`. Middle C is `C`.
- Double-check every bar sums to the meter (16 cells in 4/4) — it is the
  most common validation error; the diagnostics will tell you the bar.
