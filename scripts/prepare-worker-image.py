#!/usr/bin/env python3
"""Assemble a local scratch image context from trusted Linux release artifacts.

No package downloads or remote base image. Build on the target Linux host;
container acceptance, rather than packaging, establishes runtime qualification.
"""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import re
import shutil
import subprocess


def main():
    repo = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--sandbox", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        parser.error("require the target Linux x86_64 host")
    lock = json.loads((repo / "docs/backend-engine-lock.json").read_text())
    if hashlib.sha256(args.engine.read_bytes()).hexdigest() != lock["sha256"]:
        parser.error("engine differs from docs/backend-engine-lock.json")
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    root = args.output / "rootfs"
    for directory in ["opt", "work", "submission", "dev", "proc", "sys", "tmp"]:
        (root / directory).mkdir(mode=0o755, parents=True, exist_ok=True)
    files = {}

    def copy(source, destination):
        destination.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        destination.chmod(0o755)
        files["/" + str(destination.relative_to(root))] = hashlib.sha256(destination.read_bytes()).hexdigest()

    for binary, name in [(args.engine, "doppelganger"), (args.sandbox, "dg-sandbox")]:
        binary = binary.resolve(strict=True)
        if binary.read_bytes()[:4] != b"\x7fELF":
            parser.error("only trusted ELF binaries are accepted")
        copy(binary, root / "opt" / name)
        # ldd is only used on these operator-provided, trusted executables.
        listing = subprocess.run(["/usr/bin/ldd", str(binary)], check=True, text=True, capture_output=True).stdout
        if "not found" in listing:
            parser.error("missing trusted runtime library")
        for line in listing.splitlines():
            match = re.search(r"(?:=>\s+|^\s*)(/\S+)\s+\(", line)
            if match:
                library = Path(match[1])
                copy(library, root / library.relative_to("/"))
    shutil.copyfile(repo / "deploy/worker.Dockerfile", args.output / "Dockerfile")
    manifest = {"schema_version": 1, "engine_sha256": lock["sha256"], "protocol": 1, "files": files, "container_qualified": False}
    (args.output / "image-inputs.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"context": str(args.output.resolve()), "build_argument": "ENGINE_SHA256=" + lock["sha256"], "container_qualified": False}))


if __name__ == "__main__":
    main()
