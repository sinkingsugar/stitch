// What deleting a thread takes, said plainly (design §2.5, "Deleting a
// thread"). The node's dry run lists `<id>\t<title>\t<role>`, the role
// `this` (the thread named), `earlier` or `later` (a part of the same
// conversation, compacted into it or from it); an older node sends no role.
// The same wording as the CLI's (`crates/stitch-cli/src/doomed.rs`).

export type Role = "this" | "earlier" | "later";
export interface Doomed { id: string; title: string; role: Role | null }

export function parse(line: string): Doomed {
  const tab = line.indexOf("\t");
  const id = tab < 0 ? line : line.slice(0, tab);
  const rest = tab < 0 ? "" : line.slice(tab + 1);
  const last = rest.lastIndexOf("\t");
  const role = last < 0 ? "" : rest.slice(last + 1);
  if (role === "this" || role === "earlier" || role === "later") return { id, title: rest.slice(0, last), role };
  return { id, title: rest, role: null };
}

export function label(d: Doomed): string {
  return d.role === "this" ? "this one" : d.role === "earlier" ? "earlier part" : d.role === "later" ? "later part" : "";
}

const times = (n: number) => (n === 1 ? "once" : n === 2 ? "twice" : `${n} times`);

/** What goes and why, in a sentence or two, and what stays; `title` is the
 *  dialog's question, `detail` what follows it. */
export function explain(list: Doomed[]): { title: string; detail: string; showList: boolean } {
  if (list.some((d) => d.role === null)) {
    return {
      title: `Delete ${list.length} thread${list.length === 1 ? "" : "s"} for good?`,
      detail: "On every node, with what they were summarized from or into. The threads they started and the forks made of them stay.",
      showList: list.length > 1,
    };
  }
  const parts = list.filter((d) => d.role !== "this").length;
  const named = list.filter((d) => d.role === "this");
  const one = named.length === 1;
  const title = one ? `Delete “${named[0]!.title}” for good?` : `Delete these ${named.length} threads for good?`;
  let detail = "On every node.";
  if (parts > 0 && one) {
    detail += ` It was summarized and continued in a new thread ${times(parts)} when it grew long, so it is ${list.length} parts of one conversation: all ${list.length} go.`;
  } else if (parts > 0) {
    detail += ` ${parts} more part${parts === 1 ? "" : "s"} of the same conversations go${parts === 1 ? "es" : ""} with them (summarized and continued in a new thread when they grew long).`;
  }
  detail += one ? " The threads it started and the forks made of it stay." : " The threads they started and the forks made of them stay.";
  return { title, detail, showList: list.length > 1 };
}
