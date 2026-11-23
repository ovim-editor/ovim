"""LSP peer with an explicit initialize barrier and an inspectable wire log."""

import json
import os
import pathlib
import sys
import time

root = pathlib.Path(sys.argv[1])


def respond(request, payload):
    body = json.dumps({"jsonrpc": "2.0", "id": request["id"], **payload}).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()


while True:
    headers = {}
    while line := sys.stdin.buffer.readline():
        if line == b"\r\n":
            break
        key, value = line.decode().split(":", 1)
        headers[key.lower()] = value.strip()
    if not headers:
        break
    request = json.loads(sys.stdin.buffer.read(int(headers["content-length"])))
    with (root / "events.jsonl").open("a") as log:
        log.write(json.dumps({**request, "peerPid": os.getpid()}) + "\n")
    method = request.get("method")
    if method is None:
        continue  # a response to one of our own requests; already logged
    if method == "initialize":
        while not (root / "initialize-response.json").exists():
            if not root.exists():
                sys.exit(0)
            time.sleep(0.01)
        respond(request, json.loads((root / "initialize-response.json").read_text()))
    elif method == "initialized":
        # Tests can have the server register watchers / other capabilities:
        # register-capability.json holds the `registrations` array.
        registration = root / "register-capability.json"
        if registration.exists():
            body = json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": 9001,
                    "method": "client/registerCapability",
                    "params": {"registrations": json.loads(registration.read_text())},
                }
            ).encode()
            sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
            sys.stdout.buffer.flush()
        # outbox.json: raw JSON-RPC messages (notifications/requests) to push
        # to the client right after `initialized`.
        outbox = root / "outbox.json"
        if outbox.exists():
            for message in json.loads(outbox.read_text()):
                body = json.dumps({"jsonrpc": "2.0", **message}).encode()
                sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
            sys.stdout.buffer.flush()
    elif method == "exit":
        break
    elif "id" in request:
        # A test can script an error or result for one method by writing
        # response-<method with / replaced by _>.json ({"error": ...} or {"result": ...}).
        scripted = root / ("response-" + (method or "").replace("/", "_") + ".json")
        if scripted.exists():
            respond(request, json.loads(scripted.read_text()))
        else:
            respond(request, {"result": None})
        # push-after-<method>.json: raw JSON-RPC messages sent once, right
        # after answering that method (e.g. a server->client refresh request).
        push = root / ("push-after-" + (method or "").replace("/", "_") + ".json")
        if push.exists():
            messages = json.loads(push.read_text())
            push.unlink()
            for message in messages:
                body = json.dumps({"jsonrpc": "2.0", **message}).encode()
                sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
            sys.stdout.buffer.flush()
