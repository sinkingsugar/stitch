#!/usr/bin/env python3
"""A stand-in for Anthropic's Messages API for the Atelier rebuild end to
end (design §2.23): asked to show the app, it calls show_app; asked by the
app for a shorter hem, it edits the project's recipe (write through
edit_file in the app's data folder), then runs rebuild_garment, then says
what came back. Streams SSE. Usage: mock_model.py <port> <app dir> <name>"""

import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT, APP, NAME = int(sys.argv[1]), sys.argv[2], sys.argv[3]
RECIPE = "projects/technical-skirt/recipe.json"


def sse(blocks, stop):
    events = [("message_start", {"type": "message_start", "message": {
        "model": "claude-opus-5-5", "usage": {"input_tokens": 100}}})]
    for i, (start, delta) in enumerate(blocks):
        events.append(("content_block_start", {"type": "content_block_start", "index": i,
                                               "content_block": start}))
        events.append(("content_block_delta", {"type": "content_block_delta", "index": i,
                                               "delta": delta}))
        events.append(("content_block_stop", {"type": "content_block_stop", "index": i}))
    events.append(("message_delta", {"type": "message_delta", "delta": {"stop_reason": stop},
                                     "usage": {"output_tokens": 20}}))
    events.append(("message_stop", {"type": "message_stop"}))
    return "".join(f"event: {n}\ndata: {json.dumps(d)}\n\n" for n, d in events)


def tool(id_, name, inp):
    return sse([({"type": "tool_use", "id": id_, "name": name, "input": {}},
                 {"type": "input_json_delta", "partial_json": json.dumps(inp)})], "tool_use")


def text(t):
    return sse([({"type": "text", "text": ""}, {"type": "text_delta", "text": t})], "end_turn")


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["content-length"])))
        last = body["messages"][-1]
        content = last["content"] if isinstance(last["content"], list) else [
            {"type": "text", "text": last["content"]}]
        results = {c.get("tool_use_id"): c for c in content if c.get("type") == "tool_result"}
        said = "\n".join(c.get("text", "") for c in content if c.get("type") == "text")
        tools = [t["name"] for t in body.get("tools", [])]
        if "show the app" in said.lower():
            out = tool("show1", "show_app", {"path": APP, "name": NAME})
        elif "show1" in results:
            out = text("Shown.")
        elif "hem" in said.lower() and not results:
            out = tool("edit1", "edit_file", {"path": RECIPE, "old_string": '"hemZ": 0.62',
                                              "new_string": '"hemZ": 0.55'})
        elif "edit1" in results:
            out = tool("run1", "app_command", {"command": "rebuild_garment",
                                               "args": {"recipe": RECIPE}})
        elif "run1" in results:
            r = results["run1"]
            parts = r.get("content") if isinstance(r.get("content"), list) else [
                {"type": "text", "text": r.get("content", "")}]
            first = next((p.get("text", "") for p in parts if p.get("type") == "text"), "")
            out = text("Rebuilt with the hem at 0.55 m. " + first.splitlines()[0][:300])
        else:
            out = text(f"(mock) tools: {', '.join(tools)}")
        data = out.encode()
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
