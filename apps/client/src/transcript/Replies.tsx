import type { ReplyBlock } from "./suggested";

/** A turn's preset answers (design §2.30): buttons while live; once
 * answered, the one picked, marked, the others dimmed. */
export function Replies({ b, onPick }: { b: ReplyBlock; onPick: (text: string) => void }) {
  return (
    <div class={"replies" + (b.live ? " live" : " used")} role="group" aria-label="Suggested replies">
      {b.replies.map((r, i) => (
        <button key={i} type="button" class={i === b.picked ? "picked" : undefined} disabled={!b.live}
          title={r.text !== r.label ? r.text : undefined} onClick={() => b.live && onPick(r.text)}>
          {i === b.picked && <span aria-hidden="true">✓ </span>}{r.label}
        </button>
      ))}
    </div>
  );
}
