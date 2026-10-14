#!/usr/bin/python3
"""Controlled runtime double. Never executes an uploaded file or proves isolation."""
import hashlib
import json
from pathlib import Path
import sys
import time

root = Path(__file__).resolve().parent
args = sys.argv[1:]
assert args[0] == "--host" and args[1].startswith("unix://")
assert args[2] == "--config"
args = args[4:]
with (root / "calls.jsonl").open("a") as file:
    file.write(json.dumps(args) + "\n")
state_path = root / "containers.json"
containers = json.loads(state_path.read_text()) if state_path.exists() else {}
identity = json.loads((root / "identity.json").read_text())

def save():
    state_path.write_text(json.dumps(containers))

if args[0] == "info":
    print(json.dumps({"OSType": "linux", "Architecture": "x86_64", "CgroupVersion": "2", "SecurityOptions": ["name=rootless", "name=seccomp,profile=builtin"]}))
elif args[:2] == ["image", "inspect"]:
    print(json.dumps([{"Id": args[2], "Config": {"Labels": {"org.doppelganger.engine-sha256": identity["sha256"], "org.doppelganger.protocol": "1"}}}]))
elif args[0] == "create":
    name = args[args.index("--name") + 1]
    label = args[args.index("--label") + 1]
    key = hashlib.sha256(name.encode()).hexdigest()
    containers[key] = {"name": name, "label": label, "probe": args[-1] == "--probe"}
    save()
    print(key)
elif args[0] == "ps":
    filters = [args[i+1] for i, arg in enumerate(args) if arg == "--filter"]
    for key, value in containers.items():
        if all((f[6:] == value["label"] if f.startswith("label=") else f[7:-1] == value["name"]) for f in filters):
            print(key)
elif args[0] == "rm":
    if (root / "refuse-removal").exists():
        sys.exit(1)
    del containers[args[-1]]
    save()
elif args[0] == "start":
    value = next(v for v in containers.values() if v["name"] == args[-1])
    if value["probe"]:
        print(json.dumps({"ready": not (root / "unready").exists(), "protocol": 1, "engine_sha256": identity["sha256"]}))
    else:
        stream = (root / "stream.bin").read_bytes()
        mode = (root / "mode").read_text()
        if mode == "logs":
            sys.stderr.buffer.write(b"x" * (9 * 1024 * 1024))
            sys.stderr.buffer.flush()
        if mode == "hang":
            sys.stdout.buffer.write(stream[:17])
            sys.stdout.buffer.flush()
            time.sleep(60)
        sys.stdout.buffer.write(stream)
        sys.stdout.buffer.flush()
        if mode == "wrapper_error":
            sys.exit(1)
else:
    sys.exit(2)
