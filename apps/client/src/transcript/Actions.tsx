// Action buttons (design §2.30, step 2): a set's buttons, each running its
// call on the node when pressed (`Request::Press`), no message sent. A
// button shows running while its press is out, then ✓ or ✗ (its latest
// press, from the transcript's `press` lines), and stays pressable while
// its set is the thread's latest.
import { signal } from "@preact/signals";
import type { ActionBlock } from "./suggested";
import * as S from "../store";

type Local = "running" | { ok: boolean; error?: string };
/** Presses made here, by `thread/set/item`, until the transcript has them. */
const pressing = signal<Record<string, Local>>({});

async function press(thread: string, set: string, item: number) {
  const key = `${thread}/${set}/${item}`;
  pressing.value = { ...pressing.value, [key]: "running" };
  let local: Local;
  try {
    const r = await S.rpc({ Press: { thread, set, item } });
    if (typeof r === "object" && "Lines" in r) {
      const v = JSON.parse(r.Lines[0] ?? "{}") as { ok?: boolean };
      local = { ok: !!v.ok };
    } else if (typeof r === "object" && "Error" in r) {
      local = { ok: false, error: r.Error };
    } else local = { ok: false, error: "unexpected reply" };
  } catch (e) {
    local = { ok: false, error: String(e) };
  }
  pressing.value = { ...pressing.value, [key]: local };
  await S.reloadOpen();
}

export function Actions({ b, thread }: { b: ActionBlock; thread: string }) {
  return (
    <div class={"actions" + (b.live ? " live" : " used")} role="group" aria-label="Action buttons">
      {b.actions.map((a, i) => {
        const local = pressing.value[`${thread}/${b.set}/${i}`];
        const running = local === "running";
        const ok = local && local !== "running" ? local.ok : b.last[i];
        const error = local && local !== "running" ? local.error : undefined;
        return (
          <button key={i} type="button" disabled={!b.live || running} class={running ? "running" : ok === true ? "ok" : ok === false ? "failed" : undefined}
            title={error ?? (a.call + (a.wake ? " (then tells the agent)" : ""))} onClick={() => b.live && !running && void press(thread, b.set, i)}>
            <span aria-hidden="true">{running ? "… " : ok === true ? "✓ " : ok === false ? "✗ " : "▶ "}</span>{a.label}
          </button>
        );
      })}
    </div>
  );
}
