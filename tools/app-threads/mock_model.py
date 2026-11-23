#!/usr/bin/env python3
"""A stand-in for Anthropic's Messages API, for trying the demo without a
real model: asked to show the app, it calls show_app; otherwise it answers
with what it was told (and how many images came with it). Streams SSE, as
the node expects. Usage: mock_model.py <port> <app dir>"""

import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
APP = sys.argv[2]


def sse(blocks, stop):
    events = [("message_start", {"type": "message_start", "message": {
        "model": "mock", "usage": {"input_tokens": 100}}})]
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


def text_of(message):
    content = message.get("content")
    if isinstance(content, str):
        return content, 0
    texts = [c.get("text", "") for c in content if c.get("type") == "text"]
    images = sum(1 for c in content if c.get("type") == "image")
    return "\n".join(texts), images


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["content-length"])))
        if not self.path.endswith("/messages"):
            self.send_response(404)
            self.end_headers()
            return
        last = body["messages"][-1]
        said, images = text_of(last)
        tool_result = any(c.get("type") == "tool_result" for c in last["content"]
                          if isinstance(c, dict)) if isinstance(last["content"], list) else False
        if "show the app" in said.lower() and not tool_result:
            out = sse([({"type": "tool_use", "id": "show1", "name": "show_app", "input": {}},
                        {"type": "input_json_delta", "partial_json": json.dumps(
                            {"path": APP, "name": "Studio"})})], "tool_use")
        elif tool_result:
            out = sse([({"type": "text", "text": ""},
                        {"type": "text_delta", "text": "Shown."})], "end_turn")
        else:
            tools = ", ".join(t["name"] for t in body.get("tools", []))
            first = said.strip().splitlines()[-1] if said.strip() else ""
            reply = (f"(mock model) You said: {first[:200]}"
                     + (f" — with {images} image(s)" if images else "")
                     + f". My tools: {tools or 'none'}.")
            out = sse([({"type": "text", "text": ""},
                        {"type": "text_delta", "text": reply})], "end_turn")
        data = out.encode()
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
