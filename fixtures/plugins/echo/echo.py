#!/usr/bin/python3
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    method = request["method"]
    if method in ("initialize", "health", "lifecycle.stop"):
        result = {"status": "ok"}
        response = {"id": request["id"], "result": result}
    elif method == "tool.echo":
        response = {"id": request["id"], "result": request["params"]}
    elif method == "tool.effect":
        response = {"id": request["id"], "effect": {"capability": "workspace.read", "operation": "read", "arguments": request["params"]}}
    elif method == "crash":
        sys.exit(7)
    else:
        response = {"id": request["id"], "error": "unknown method"}
    print(json.dumps(response), flush=True)
