"""Scriptable stdio Debug Adapter for launch/debug flow tests.

argv[1] is a directory shared with the test:
  events.jsonl      every request received, one JSON object per line
                    (plus {"adapterPid": ...} on the first line)
  scenario.json     optional script:
      "launch_error":  message; respond to launch/attach with a failure
      "initialize_error": message; respond to initialize with a failure
      "on_configuration_done": list of DAP event/response bodies to emit
          each item: {"event": "output", "body": {...}, "delay": 0.1}
      "capabilities": extra capabilities advertised (supportsLogPoints, ...)
      "exception_filters": advertised exceptionBreakpointFilters
      "threads": answer to threads; "frames_by_thread": {"<threadId>": [frame...]}
      "frames", "scopes": answers for stackTrace / scopes
      "variables": {"<variablesReference>": [variable, ...]}
      "exception_info": body answered to exceptionInfo (else it fails)
      "evaluate": {"<expression>": {"result": ..., "type": ..., "variablesReference": ...}}
      "move_breakpoints": {"<line>": <line>}: setBreakpoints puts those lines elsewhere
      "unverified_breakpoints": [<line>]: answered as {"verified": false} without a line
"""

import json
import os
import pathlib
import sys
import time

root = pathlib.Path(sys.argv[1])
scenario_path = root / "scenario.json"
scenario = json.loads(scenario_path.read_text()) if scenario_path.exists() else {}
seq = 0


def send(message):
    global seq
    seq += 1
    body = json.dumps({"seq": seq, **message}).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()


def respond(request, body=None, success=True, message=None):
    reply = {
        "type": "response",
        "request_seq": request["seq"],
        "command": request["command"],
        "success": success,
    }
    if body is not None:
        reply["body"] = body
    if message is not None:
        reply["message"] = message
    send(reply)


def event(name, body=None):
    message = {"type": "event", "event": name}
    if body is not None:
        message["body"] = body
    send(message)


with (root / "events.jsonl").open("a") as log:
    log.write(json.dumps({"adapterPid": os.getpid()}) + "\n")

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
    command = request.get("command")
    if command == "initialize":
        if "initialize_error" in scenario:
            respond(request, success=False, message=scenario["initialize_error"])
            continue
        capabilities = {"supportsConfigurationDoneRequest": True}
        if "exception_filters" in scenario:
            capabilities["exceptionBreakpointFilters"] = scenario["exception_filters"]
        capabilities.update(scenario.get("capabilities", {}))
        respond(request, capabilities)
        event("initialized")
    elif command in ("launch", "attach"):
        if "launch_error" in scenario:
            respond(request, success=False, message=scenario["launch_error"])
        else:
            respond(request)
    elif command == "setBreakpoints":
        moves = scenario.get("move_breakpoints", {})
        unverified = scenario.get("unverified_breakpoints", [])
        answers = []
        for b in request["arguments"].get("breakpoints", []):
            if b["line"] in unverified:
                answers.append({"verified": False})
            else:
                answers.append({"verified": True, "line": moves.get(str(b["line"]), b["line"])})
        respond(request, {"breakpoints": answers})
    elif command == "configurationDone":
        respond(request)
        for item in scenario.get("on_configuration_done", []):
            time.sleep(item.get("delay", 0))
            if item["event"] == "crash":
                sys.stderr.write(item.get("message", "boom") + "\n")
                sys.stderr.flush()
                os._exit(item.get("code", 101))
            event(item["event"], item.get("body"))
    elif command == "threads":
        respond(request, {"threads": scenario.get("threads", [{"id": 1, "name": "main"}])})
    elif command == "stackTrace":
        by_thread = scenario.get("frames_by_thread", {})
        thread = str(request["arguments"].get("threadId"))
        respond(request, {"stackFrames": by_thread.get(thread, scenario.get("frames", []))})
    elif command == "scopes":
        respond(request, {"scopes": scenario.get("scopes", [])})
    elif command == "variables":
        ref = str(request["arguments"]["variablesReference"])
        respond(request, {"variables": scenario.get("variables", {}).get(ref, [])})
    elif command == "evaluate":
        expression = request["arguments"]["expression"]
        known = scenario.get("evaluate", {})
        if expression in known:
            respond(request, known[expression])
        else:
            respond(request, success=False, message=f"cannot evaluate {expression}")
    elif command == "exceptionInfo":
        if "exception_info" in scenario:
            respond(request, scenario["exception_info"])
        else:
            respond(request, success=False, message="no exception")
    elif command == "disconnect":
        respond(request)
        if scenario.get("linger_after_disconnect"):
            time.sleep(300)
        break
    else:
        respond(request)
