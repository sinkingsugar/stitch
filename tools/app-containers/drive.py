#!/usr/bin/env python3
"""Drives Atelier's page calls for atelier_e2e.sh, as its JavaScript
would: stitch.files.write (the recipe), stitch.threads.create (a shorter
hem), stitch.threads.messages until the reply, then reads the preview and
the GLB through stitch.files. Usage: drive.py <port> <host> <token> <recipe> <out dir>"""

import http.client
import json
import sys
import time

port, host, token, recipe, out = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]


def call(method, path, body=None, raw=False):
    c = http.client.HTTPConnection("127.0.0.1", int(port), timeout=120)
    headers = {"Host": host, "Cookie": f"stitch={token}"}
    if method == "POST":
        headers["Origin"] = f"http://{host}"
    if isinstance(body, (dict, list)):
        body = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    c.request(method, path, body=body, headers=headers)
    r = c.getresponse()
    data = r.read()
    if raw:
        return r.status, {k.lower(): v for k, v in r.getheaders()}, data
    try:
        return r.status, json.loads(data)
    except ValueError:
        return r.status, data.decode(errors="replace")


# The recipe into the data folder, as the page writes it.
st, d = call("POST", "/_stitch/files/projects/technical-skirt/recipe.json", open(recipe, "rb").read())
print("write recipe:", st, d)
assert st == 200, d
# Outside the folder: refused.
for bad in ["/_stitch/files/../x", "/_stitch/files/%2e%2e/x", "/_stitch/files/.stitch/ignore"]:
    st, d = call("POST", bad, b"x")
    print("write", bad, "->", st)
    assert st != 200
st, d = call("GET", "/_stitch/files")
print("list:", st, [f["path"] for f in d["files"]], d["quota"])

st, d = call("POST", "/_stitch/threads", {"goal": "Make the technical skirt's hem shorter (0.55 m) and rebuild it.",
                                          "title": "Shorter hem"})
print("create:", st, d)
assert st == 200, d
tid = d["id"]
start = time.time()
while True:
    st, d = call("GET", f"/_stitch/threads/{tid}/messages")
    state = (d.get("state") or {}).get("state")
    if state in ("idle", "done", "cancelled") and time.time() - start > 5:
        break
    if time.time() - start > 1800:
        raise SystemExit("no reply after 30 minutes")
    time.sleep(5)
print(f"thread {tid}: {state} after {int(time.time() - start)} s")
images = []
for m in d["messages"]:
    print(f"  {m['role']}: {(m.get('text') or m.get('name') or '')[:300]!r} images={len(m.get('images') or [])}")
    images += m.get("images") or []
assert images, "no image came back"
st, h, png = call("GET", images[-1]["url"], raw=True)
print("preview image:", st, h.get("content-type"), len(png), "bytes")
assert st == 200 and png.startswith(b"\x89PNG")
open(f"{out}/preview.png", "wb").write(png)
st, d = call("GET", "/_stitch/files?dir=out")
glbs = [f["path"] for f in d["files"] if f["path"].endswith("garment.glb")]
print("outputs:", [f["path"] for f in d["files"]])
st, h, glb = call("GET", "/_stitch/files/" + glbs[-1], raw=True)
print("garment.glb:", st, h.get("content-type"), h.get("content-disposition"), len(glb), "bytes", glb[:4])
assert st == 200 and glb[:4] == b"glTF"
st, h, blend = call("GET", "/_stitch/files/" + glbs[-1].replace("garment.glb", "technical-skirt.blend"), raw=True)
print(".blend:", st, h.get("content-type"), h.get("content-disposition"))
assert h.get("content-disposition") == "attachment"
st, h, used = call("GET", "/_stitch/files/" + glbs[-1].replace("garment.glb", "recipe.json"), raw=True)
print("recipe used:", json.loads(used)["hemZ"])
assert json.loads(used)["hemZ"] == 0.55
print("OK")
