#!/usr/bin/env python3
"""B0 spike: the native messaging host. Relays Chrome's length-prefixed
JSON (stdio) to newline-delimited JSON on the node stub's Unix socket, and
reconnects when the node goes away (the hello is sent again)."""
import json, os, socket, struct, sys, threading, time

SOCK = os.environ.get("STITCH_B0_SOCK", "/tmp/stitch-b0/node.sock")
LOG = open("/tmp/stitch-b0/host-%d.log" % os.getpid(), "a", buffering=1)
lock = threading.Lock()
conn = None
hello = None


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, file=LOG)


def to_chrome(obj):
    data = json.dumps(obj).encode()
    with lock:
        sys.stdout.buffer.write(struct.pack("=I", len(data)) + data)
        sys.stdout.buffer.flush()


def node_loop():
    global conn
    while True:
        try:
            s = socket.socket(socket.AF_UNIX)
            s.connect(SOCK)
        except OSError:
            time.sleep(1)
            continue
        log("connected to node")
        conn = s
        if hello:
            s.sendall((json.dumps(hello) + "\n").encode())
        buf = b""
        try:
            while True:
                chunk = s.recv(1 << 20)
                if not chunk:
                    break
                buf += chunk
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    to_chrome(json.loads(line))
        except OSError:
            pass
        log("node gone")
        conn = None
        time.sleep(1)


threading.Thread(target=node_loop, daemon=True).start()
log("started", sys.argv)
stdin = sys.stdin.buffer
while True:
    head = stdin.read(4)
    if len(head) < 4:
        log("chrome closed stdin")
        break
    (n,) = struct.unpack("=I", head)
    msg = json.loads(stdin.read(n))
    if msg.get("t") == "hello":
        msg["host_pid"] = os.getpid()
        hello = msg
    c = conn
    if c:
        try:
            c.sendall((json.dumps(msg) + "\n").encode())
        except OSError:
            pass
