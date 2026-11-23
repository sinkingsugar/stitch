// Loading Space (the map in 3D) on demand: three.js and the engine are a separate file, `space.js`, next to `app.js`
// (embedded in the node beside it), fetched by a native `import()` the first time 3D is shown. The URL is built at
// run time so the bundler leaves it alone; `?v=` is the client's version, so a cached chunk of another build is never
// used with this bundle.
import { CLIENT_VERSION } from "../../version";
import type { SpaceModule } from "./types";

let loading: Promise<SpaceModule> | null = null;

/** Whether this browser gives a WebGL 2 context here (three.js needs it); the context is let go at once. */
export function webgl2(): boolean {
  try {
    const gl = document.createElement("canvas").getContext("webgl2");
    if (!gl) return false;
    gl.getExtension("WEBGL_lose_context")?.loseContext();
    return true;
  } catch {
    return false;
  }
}

export function loadSpace(): Promise<SpaceModule> {
  if (!loading) {
    const url = new URL("space.js?v=" + encodeURIComponent(CLIENT_VERSION), document.baseURI).href;
    loading = (import(url) as Promise<SpaceModule>).catch((e: unknown) => {
      loading = null; // a later try may work (the node restarted, the network came back)
      throw e;
    });
  }
  return loading;
}
