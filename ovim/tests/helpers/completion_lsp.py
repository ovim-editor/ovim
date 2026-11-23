"""Scriptable stdio LSP peer for completion tests.

Everything it answers comes from files in the directory given as argv[1], so a
test can change the script between requests:

  capabilities.json          server capabilities (default: completion + '.' trigger)
  completion.json            answer to textDocument/completion (list or array)
  completion-incomplete.json answer when triggerKind == 3 (incomplete re-request)
  completion-delay.txt       seconds to wait before answering completion
  resolve.json               fields merged into the item of completionItem/resolve

Every message received is appended to events.jsonl.
"""

import json
import pathlib
import sys
import time

root = pathlib.Path(sys.argv[1])


def send(message):
    body = json.dumps({"jsonrpc": "2.0", **message}).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()


def respond(request, result):
    send({"id": request["id"], "result": result})


def read_json(name, default=None):
    path = root / name
    return json.loads(path.read_text()) if path.exists() else default


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
        log.write(json.dumps(request) + "\n")
    method = request.get("method")
    if method is None or "id" not in request:
        if method == "exit":
            break
        continue
    if method == "initialize":
        capabilities = read_json(
            "capabilities.json",
            {"textDocumentSync": 1, "completionProvider": {"triggerCharacters": ["."]}},
        )
        respond(request, {"capabilities": capabilities})
    elif method == "textDocument/completion":
        delay = (root / "completion-delay.txt")
        if delay.exists():
            time.sleep(float(delay.read_text()))
        context = request["params"].get("context") or {}
        answer = None
        if context.get("triggerKind") == 3:
            answer = read_json("completion-incomplete.json")
        if answer is None:
            answer = read_json("completion.json")
        respond(request, answer)
    elif method == "completionItem/resolve":
        item = request["params"]
        item.update(read_json("resolve.json", {}))
        respond(request, item)
    else:
        respond(request, None)
