# The Atlas prototype (reference only)

The scratch app the map was ported from (2026-11-11, over `stitch.live.threads` with `conversations`):
plain JS, classic scripts, d3 v7 (`/_lib/d3.min.js`) and Three.js (`space.js`, an ES module). It is **not built, not tested and not
served**; it is kept so that what is not ported yet can be ported from it, and so that the look and behaviour of the
ported part can be compared (design §2.25, "As built, the map in the client").

| prototype | in the client |
|---|---|
| `common.js` (model, roll-up, palette, filters, text measuring) | `src/map/model.ts`, `src/map/text.ts`, `src/map/state.ts` (ported, tested) |
| `islands.js` | `src/map/islands.ts` (ported) |
| `ui.js` (tabs, search, filters, card, loading) | `src/map/MapView.tsx`, `src/map/Card.tsx` |
| `river.js` | not ported (deprioritised by the user, 2026-11-12) |
| `space.js` (Three.js) | `src/map/space/` (layout.ts, engine.ts; the lazily loaded chunk `dist/space.js`) |
