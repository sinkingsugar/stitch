import { render } from "preact";
import "./style.css";
import { App } from "./app";
import { WsConn } from "./conn";
import { MockConn } from "./mock";
import { applyRoute, current, init, select } from "./store";
import { openNewRoom } from "./room/state";
import { start } from "./nav";
import { peek } from "./sidebar/Sidebar";

init(new URLSearchParams(location.search).has("mock") ? new MockConn() : new WsConn());
render(<App />, document.getElementById("app")!);
start(applyRoute);
// `?mock&open=<thread>` opens a thread of the mock (screenshots; `&unfold` unfolds its digest).
const open = new URLSearchParams(location.search).get("open");
if (open && new URLSearchParams(location.search).has("mock")) setTimeout(() => void select(open), 300);
if (open && new URLSearchParams(location.search).has("top")) setTimeout(() => { const t = document.querySelector(".transcript"); if (t) t.scrollTop = 0; }, 1500);
const hover = new URLSearchParams(location.search).get("hover");
if (hover && new URLSearchParams(location.search).has("mock")) setTimeout(() => peek(hover), 800);
// `?mock&newroom` opens the New Room dialog (with `open=<thread>`: "Pull in model…" on it), for screenshots.
if (new URLSearchParams(location.search).has("mock") && new URLSearchParams(location.search).has("newroom"))
  setTimeout(() => openNewRoom(open ? { from: { id: open, title: current.value?.title ?? open } } : {}), 900);
