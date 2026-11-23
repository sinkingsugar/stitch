// The built-in dashboard's entry (design §2.10): the client's home view on
// `stitch.live`, bundled by `just client` into dist/dashboard/.
import { render } from "preact";
import "../style.css";
import "./dashboard.css";
import { Dashboard } from "./Dashboard";
import { initDashboard } from "./source";

initDashboard();
render(<Dashboard />, document.getElementById("app")!);
