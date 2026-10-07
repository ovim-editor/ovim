"""Scriptable stdio LSP peer for review repros.

argv[1] = control directory, or "@root" to use the workspace root the client
announces in `initialize` (one control directory per project root). Files read
fresh on every message:
  capabilities.json            server capabilities (default: incremental sync + everything)
  response-<method>.json       {"result": ...} or {"error": ...}; method '/' -> '_'
  delay-<method>.txt           seconds to sleep before answering
  push-after-<method>.json     raw messages pushed once after answering (file deleted)
  crash-on-<method>            if present, exit(1) when that method arrives (file deleted)
Writes:
  events.jsonl                 every message received (+ pid)
  docs/<urlencoded uri>.txt    server-side mirror of every open document, maintained
                               by applying didOpen/didChange (UTF-16 positions)
  sync-errors.log              protocol violations noticed (didChange w/o open,
                               version not increasing, double open, bad range)
"""

import json
import os
import pathlib
import sys
import time
import urllib.parse

per_root = sys.argv[1] == "@root"
root = pathlib.Path(sys.argv[1])
if not per_root:
    (root / "docs").mkdir(exist_ok=True)
docs = {}  # uri -> (version, text)


def send(message):
    body = json.dumps({"jsonrpc": "2.0", **message}).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()


def read_json(name, default=None):
    path = root / name
    try:
        return json.loads(path.read_text()) if path.exists() else default
    except Exception:
        return default


def err(msg):
    with (root / "sync-errors.log").open("a") as f:
        f.write(f"[pid {os.getpid()}] {msg}\n")


def utf16_offset(text, line, character):
    lines = text.split("\n")
    if line > len(lines) or line < 0:
        err(f"range line {line} out of bounds ({len(lines)} lines)")
        return len(text)
    if line == len(lines):
        return len(text)
    off = sum(len(l) + 1 for l in lines[:line])
    l = lines[line]
    units = 0
    for i, ch in enumerate(l):
        if units >= character:
            if units > character:
                err(f"position {line}:{character} splits a surrogate pair")
            return off + i
        units += 2 if ord(ch) > 0xFFFF else 1
    if character > units:
        err(f"position {line}:{character} beyond line length {units} (utf16)")
    return off + len(l)


def save(uri):
    name = urllib.parse.quote(uri, safe="") + ".txt"
    v, t = docs[uri]
    (root / "docs" / name).write_text(t)
    (root / "docs" / (name + ".version")).write_text(str(v))


while True:
    headers = {}
    while line := sys.stdin.buffer.readline():
        if line == b"\r\n":
            break
        key, value = line.decode().split(":", 1)
        headers[key.lower()] = value.strip()
    if not headers:
        break
    msg = json.loads(sys.stdin.buffer.read(int(headers["content-length"])))
    if per_root and msg.get("method") == "initialize":
        root = pathlib.Path(urllib.parse.unquote(urllib.parse.urlparse(msg["params"]["rootUri"]).path))
        (root / "docs").mkdir(exist_ok=True)
    with (root / "events.jsonl").open("a") as log:
        log.write(json.dumps({**msg, "pid": os.getpid(), "t": time.time()}) + "\n")
    method = msg.get("method")
    key = (method or "").replace("/", "_")
    crash = root / ("crash-on-" + key)
    if method and crash.exists():
        crash.unlink()
        sys.exit(1)
    p = msg.get("params") or {}
    if method == "textDocument/didOpen":
        td = p["textDocument"]
        if td["uri"] in docs:
            err(f"double didOpen {td['uri']}")
        docs[td["uri"]] = (td["version"], td["text"])
        save(td["uri"])
    elif method == "textDocument/didChange":
        td = p["textDocument"]
        uri = td["uri"]
        if uri not in docs:
            err(f"didChange for unopened {uri} v{td['version']}")
        else:
            v, t = docs[uri]
            if td["version"] <= v:
                err(f"didChange version {td['version']} <= {v} for {uri}")
            for ch in p["contentChanges"]:
                if ch.get("range") is None:
                    t = ch["text"]
                else:
                    r = ch["range"]
                    s = utf16_offset(t, r["start"]["line"], r["start"]["character"])
                    e = utf16_offset(t, r["end"]["line"], r["end"]["character"])
                    t = t[:s] + ch["text"] + t[e:]
            docs[uri] = (td["version"], t)
            save(uri)
    if method in ("textDocument/didOpen", "textDocument/didChange"):
        # auto-diagnostics.json: {"diagnostics": [...], "versioned": bool}
        # published after every didOpen/didChange for that document.
        auto = read_json("auto-diagnostics.json")
        if auto is not None:
            uri = p["textDocument"]["uri"]
            params = {"uri": uri, "diagnostics": auto.get("diagnostics", [])}
            if auto.get("versioned", True) and uri in docs:
                params["version"] = docs[uri][0]
            send({"method": "textDocument/publishDiagnostics", "params": params})
    if method == "textDocument/didClose":
        uri = p["textDocument"]["uri"]
        if uri not in docs:
            err(f"didClose for unopened {uri}")
        docs.pop(uri, None)
    if method is None or "id" not in msg:
        if method == "exit":
            break
        continue
    if method == "initialize":
        caps = read_json(
            "capabilities.json",
            {
                "textDocumentSync": {"openClose": True, "change": 2, "save": {"includeText": False}},
                "completionProvider": {"triggerCharacters": ["."], "resolveProvider": False},
                "hoverProvider": True,
                "definitionProvider": True,
                "referencesProvider": True,
                "renameProvider": True,
                "codeActionProvider": True,
                "documentFormattingProvider": True,
                "signatureHelpProvider": {"triggerCharacters": ["(", ","]},
                "documentSymbolProvider": True,
                "workspaceSymbolProvider": True,
                "foldingRangeProvider": True,
            },
        )
        send({"id": msg["id"], "result": {"capabilities": caps}})
        continue
    delay = root / ("delay-" + key + ".txt")
    if delay.exists():
        time.sleep(float(delay.read_text()))
    scripted = read_json("response-" + key + ".json")
    if scripted is None:
        send({"id": msg["id"], "result": None})
    else:
        send({"id": msg["id"], **scripted})
    push = root / ("push-after-" + key + ".json")
    if push.exists():
        messages = json.loads(push.read_text())
        push.unlink()
        for m in messages:
            send(m)
