# Third-party notices

Stitch is under the Functional Source License, Version 1.1, Apache 2.0
Future License ([LICENSE](../LICENSE)). It includes, bundles or downloads the
third-party components below, under their own licenses. These files go
with every release of Stitch, and with every client that embeds its node.

| File | What it covers | How it is kept |
|---|---|---|
| [rust.txt](rust.txt) | The Rust crates built into Stitch's binaries and libraries (Linux, macOS, iOS, visionOS), Otter and engram included | Generated: `just notices` (cargo-about: [about.toml](../about.toml), [rust.hbs](rust.hbs)) |
| [client.txt](client.txt) | The npm packages bundled into the web client (`apps/client/dist`, embedded in the node) | Generated: `just notices` ([tools/notices-client.ts](../tools/notices-client.ts) over `apps/client/bun.lock`) |
| [llama.txt](llama.txt) | llama.cpp and ggml, and the libraries llama.cpp vendors (stb_image, miniaudio, subprocess.h, the hash libraries, nlohmann/json, cpp-httplib) | Generated: `just notices` (from the `crates/stitch-llama/llama.cpp` submodule) |
| [applibs.txt](applibs.txt) | The libraries served to apps at `/_lib/` (`crates/stitch-node/applibs/`), the libraries Mermaid and D3 bundle, meshrepair, the leadsheet compiler, the GeneralUser GS soundfont | By hand (below) |

`just notices-check` (in `just check` and CI) fails when a generated file is
not what a fresh run gives: after changing `Cargo.lock`, the client's
`bun.lock` or the llama.cpp submodule, run `just notices` and commit the
result. Which licenses the Rust dependencies may have is checked by
cargo-deny ([deny.toml](../deny.toml), `just deny`).

## What Cargo does not see

### Libraries for apps (`crates/stitch-node/applibs/`)

Carried in the node's binary, gzipped, and served unmodified to the apps
agents make (design §2.10); the web view runs them, Stitch never links or
calls them. Their table, with versions and licenses, is
[applibs/README.md](../crates/stitch-node/applibs/README.md); their license
texts are in [applibs.txt](applibs.txt). Of note:

- **p5.js** (LGPL-2.1): the unmodified minified release (v1.9.4), a separate
  file the node serves, replaceable by another build of p5.js under the same
  name; its source is the project's (github.com/processing/p5.js). The full
  LGPL-2.1 text is in applibs.txt.
- **Mermaid** (MIT) bundles its own dependencies (DOMPurify, MPL-2.0 OR
  Apache-2.0; Chevrotain, Apache-2.0; d3-sankey, BSD-3-Clause; Cytoscape,
  dagre-d3-es, lodash-es, marked, Langium and others, MIT): listed with
  their texts in applibs.txt.
- **Apache-2.0**: Math.js (with its NOTICE), SheetJS Community Edition,
  SpessaSynth and pdfmake's Roboto font; the license text once, at the end
  of applibs.txt.
- **meshrepair** is Fragcolor's own (github.com/sinkingsugar/meshrepair, MIT
  OR Apache-2.0); the **leadsheet compiler** (`leadsheet_wasm`) is
  Fragcolor's own, under Stitch's license.
- **GeneralUser GS** 2.0.3 (`GeneralUserGS.sf3`), the soundfont, by
  S. Christian Collins (www.schristiancollins.com), under the GeneralUser GS
  License v2.0: its text is in applibs.txt.

To add or update one: change the file and its row in applibs/README.md, and
its entry in applibs.txt (the license file its package publishes, at that
version).

### Downloaded at run time

- **EmbeddingGemma 2** (`ggml-org/embeddinggemma-2-GGUF` on Hugging Face,
  the GGUF of `google/embeddinggemma-2`), the embedding model of memory,
  downloaded by the node when memory is turned on, not shipped with Stitch:
  Apache-2.0. Its use must also follow Google's
  [Gemma Prohibited Use Policy](https://ai.google.dev/gemma/prohibited_use_policy),
  as its model card requires.
- Models, MCP servers, Chromium and other tools the user configures are
  theirs, under their own terms; Stitch does not ship them.
