// Keeps the client from flooding the node (design §2.24): the node pushes
// `changed` very often while threads work, and each push used to start a
// full round of requests, piling up behind a slow node.

/** `fn` with at most one run in flight and at most one queued behind it:
 * calls while one runs share the queued run (and its promise). */
export function coalesced(fn: () => Promise<void>): () => Promise<void> {
  let running: Promise<void> | null = null;
  let queued: Promise<void> | null = null;
  const start = (): Promise<void> => {
    const p: Promise<void> = fn()
      .catch(() => {})
      .finally(() => { if (running === p) running = null; });
    running = p;
    return p;
  };
  return () => {
    if (!running) return start();
    queued ??= running.then(() => { queued = null; return start(); });
    return queued;
  };
}

/** `fn` once things go quiet for `wait` ms, and at the latest `maxWait` ms
 * after the first call of a burst. */
export function debounced(fn: () => void, wait = 500, maxWait = 2000): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let first = 0;
  return () => {
    const now = Date.now();
    if (timer === undefined) first = now;
    else clearTimeout(timer);
    timer = setTimeout(() => { timer = undefined; fn(); }, Math.max(0, Math.min(wait, first + maxWait - now)));
  };
}
