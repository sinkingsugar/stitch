// Images: the node serves them to this origin alone at `/_stitch/blob/<hash>`
// (apps/blob.rs) and takes pasted ones at `POST /_stitch/blob`.
import type { FileRef, ImageRef } from "../payload";

/** Stand-ins for `?mock`: hash -> URL. */
export const mockBlobs = new Map<string, string>();

export const blobUrl = (hash: string) => mockBlobs.get(hash) ?? `/_stitch/blob/${hash}`;

/** A file waiting in the composer (an image has a preview). */
export interface Staged extends ImageRef {
  /** Where to show it while composing (images). */
  preview: string | null;
  bytes: number;
}

export const isImage = (type: string) => type.startsWith("image/");

/** A blob's download address, named. */
export const fileUrl = (f: { hash: string; name: string | null }) =>
  blobUrl(f.hash) + (mockBlobs.has(f.hash) ? "" : "?name=" + encodeURIComponent(f.name ?? "file"));

/** What `Request::Chat` takes for a staged file: the hash, with its name. */
export const attachmentArg = (s: { hash: string; name: string | null }) =>
  s.hash + (s.name ? "#" + encodeURIComponent(s.name) : "");

export const sizeText = (n: number) =>
  n < 1024 ? `${n} B` : n < 1 << 20 ? `${(n / 1024).toFixed(0)} KB` : `${(n / (1 << 20)).toFixed(1)} MB`;

export const MAX_FILE = 50 << 20;

/** Stores a file the user pasted, dropped or picked; the node names it by hash,
 * which `Request::Chat` takes as an attachment (with its name). */
export async function upload(file: Blob, name: string | null): Promise<Staged> {
  if (file.size > MAX_FILE) throw new Error(`${name ?? "file"}: more than 50 MB`);
  const mock = new URLSearchParams(location.search).has("mock");
  if (mock) {
    const preview = URL.createObjectURL(file);
    const hash = "blake3:mock-" + Math.random().toString(16).slice(2);
    mockBlobs.set(hash, preview);
    const type = file.type || "application/octet-stream";
    return { hash, type, name, preview: isImage(type) ? preview : null, bytes: file.size };
  }
  const r = await fetch("/_stitch/blob?name=" + encodeURIComponent(name ?? "file"), { method: "POST", body: file, credentials: "same-origin" });
  if (!r.ok) throw new Error(`${name ?? "file"} not taken: ${await r.text()}`);
  const j = (await r.json()) as { hash: string; type: string; bytes: number };
  return { hash: j.hash, type: j.type, name, preview: isImage(j.type) ? URL.createObjectURL(file) : null, bytes: j.bytes };
}
