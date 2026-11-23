// The transcript's rich entries, as the iOS app shows a thread: images (a
// thumbnail, a click to zoom), apps inline, and diffs.
import "./rich.css";
import { useEffect, useRef, useState } from "preact/hooks";
import { signal } from "@preact/signals";
import type { FileRef, ImageRef, TranscriptEntry } from "../payload";
import { blobUrl, fileUrl, sizeText } from "./blob";

type Tool = Extract<TranscriptEntry, { kind: "tool" }>;

export const zoomed = signal<ImageRef | null>(null);

export function Lightbox() {
  const z = zoomed.value;
  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === "Escape" && (zoomed.value = null);
    addEventListener("keydown", k);
    return () => removeEventListener("keydown", k);
  }, []);
  if (!z) return null;
  return (
    <div class="lightbox" onClick={() => (zoomed.value = null)}>
      <img src={blobUrl(z.hash)} alt={z.name ?? "image"} />
      <div class="cap">{z.name ?? z.type} · click or Esc to close · <a href={blobUrl(z.hash)} target="_blank" rel="noopener" onClick={(e) => e.stopPropagation()}>open original</a></div>
    </div>
  );
}

export function Images({ images }: { images: ImageRef[] | undefined }) {
  if (!images?.length) return null;
  return (
    <div class="images">
      {images.map((im) => (
        <button key={im.hash} class="thumb" title={im.name ?? im.type} onClick={() => (zoomed.value = im)}>
          <img src={blobUrl(im.hash)} alt={im.name ?? "image"} loading="lazy" />
        </button>
      ))}
    </div>
  );
}

/** Files that are not images: a chip with the name and size, a download. */
export function Files({ files }: { files: FileRef[] | undefined }) {
  if (!files?.length) return null;
  return (
    <div class="files">
      {files.map((f) => (
        <a key={f.hash} class="fchip dl" href={fileUrl(f)} download={f.name ?? "file"} title={`${f.type} · download`}>
          <span>📄</span> <b>{f.name ?? "file"}</b> <span class="meta">{sizeText(f.bytes)}</span> <span class="get">⬇</span>
        </a>
      ))}
    </div>
  );
}

/** Where an app's address came from, by version, so a re-render asks once. */
const urls = new Map<string, Promise<string | null>>();
/** Resolves an app version to its address with the node's token
 * (`Request::AppOpen`, as `stitch app open`). */
export type OpenApp = (manifest: string) => Promise<string | null>;
let opener: OpenApp = async () => null;
export const setAppOpener = (f: OpenApp) => void (opener = f);

/** Pinning an app from its frame, the store's (set by `init`): `pinned`
 * reads the pinned apps (a signal, so the frame follows them) and says the
 * pinned app this version is, if any. */
export type AppPins = {
  pinned: (manifest: string, name: string | null) => string | null;
  pin: (manifest: string) => Promise<void>;
  unpin: (app: string) => Promise<void>;
};
let pins: AppPins | null = null;
export const setAppPins = (p: AppPins) => void (pins = p);

export function AppFrame({ manifest, name }: { manifest: string; name: string | null }) {
  const [url, setUrl] = useState<string | null | undefined>(undefined);
  const [tall, setTall] = useState(false);
  const [busy, setBusy] = useState(false);
  const pinnedAs = pins?.pinned(manifest, name) ?? null;
  const togglePin = async () => {
    if (!pins || busy) return;
    setBusy(true);
    try {
      await (pinnedAs ? pins.unpin(pinnedAs) : pins.pin(manifest));
    } finally {
      setBusy(false);
    }
  };
  const [fit, setFit] = useState<number | null>(null);
  const frame = useRef<HTMLIFrameElement>(null);
  // The one message honored from the app's document: its content height
  // (bridge.js), only from this frame's window and its origin, clamped.
  useEffect(() => {
    if (!url) return;
    const origin = new URL(url, location.href).origin;
    const on = (e: MessageEvent) => {
      const d = e.data as { type?: unknown; height?: unknown } | null;
      if (e.source !== frame.current?.contentWindow || e.origin !== origin) return;
      if (!d || typeof d !== "object" || d.type !== "stitch:resize") return;
      if (typeof d.height !== "number" || !Number.isFinite(d.height)) return;
      setFit(Math.round(Math.min(1600, Math.max(120, d.height))));
    };
    window.addEventListener("message", on);
    return () => window.removeEventListener("message", on);
  }, [url]);
  useEffect(() => {
    let p = urls.get(manifest);
    if (!p) urls.set(manifest, (p = opener(manifest).catch(() => null)));
    void p.then(setUrl);
  }, [manifest]);
  return (
    <div class="appframe">
      <div class="apphead">
        <span class="appname">▣ {name ?? "app"}</span>
        <span class="grow" />
        {pins && (
          <button
            disabled={busy}
            title={pinnedAs ? "Remove it from the pinned apps" : "Pin it: it follows this thread's newer versions of it"}
            onClick={() => void togglePin()}
          >
            {pinnedAs ? "Unpin" : "Pin"}
          </button>
        )}
        <button onClick={() => setTall(!tall)}>{tall ? "Smaller" : "Taller"}</button>
        <button disabled={!url} onClick={() => url && window.open(url, "_blank", "noopener")}>Open in tab</button>
      </div>
      {url === undefined ? (
        <div class="appwait">opening…</div>
      ) : url === null ? (
        <div class="appwait">this app is not on this device</div>
      ) : (
        // Its own origin, none of the client's authority: no bridge to the
        // WebSocket, and a sandbox (scripts and forms, no top navigation).
        <iframe
          ref={frame}
          src={url}
          title={name ?? "app"}
          style={{ height: fit !== null && !tall ? `${fit}px` : tall ? "760px" : "380px" }}
          sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-downloads allow-modals"
          referrerpolicy="no-referrer"
          loading="lazy"
        />
      )}
    </div>
  );
}

export function Diff({ path, text }: { path: string; text: string }) {
  const lines = text.split("\n");
  if (lines.at(-1) === "") lines.pop();
  const add = lines.filter((l) => l.startsWith("+") && !l.startsWith("+++")).length;
  const del = lines.filter((l) => l.startsWith("-") && !l.startsWith("---")).length;
  const cls = (l: string) =>
    l.startsWith("@@") ? "hunk" : l.startsWith("+") ? "add" : l.startsWith("-") ? "del" : l.startsWith("[") ? "hunk" : "";
  return (
    <details class="diff" open={lines.length <= 40}>
      <summary>
        <code>{path}</code> <span class="plus">+{add}</span> <span class="minus">−{del}</span>
      </summary>
      <pre>{lines.map((l, i) => <div key={i} class={"dl " + cls(l)}>{l || " "}</div>)}</pre>
    </details>
  );
}

/** A tool call: its line and output, collapsed; what it showed beside it. */
export function ToolEntry({ e }: { e: Tool }) {
  const out = e.result ? e.result.output || "(no output)" : "running…";
  return (
    <div class="toolbox">
      {e.diff ? (
        <>
          <div class={"toolline" + (e.result && !e.result.ok ? " bad" : "")}>
            <code>{e.call}</code>
            {!e.result && <span class="run"> …</span>}
          </div>
          <Diff path={e.diff.path} text={e.diff.text} />
        </>
      ) : (
        <details class={"tool" + (e.result ? (e.result.ok ? "" : " bad") : " run")}>
          <summary><code>{e.call}</code></summary>
          <pre>{out}</pre>
        </details>
      )}
      <Images images={e.images} />
      {e.app && <AppFrame manifest={e.app.manifest} name={e.app.name} />}
    </div>
  );
}
