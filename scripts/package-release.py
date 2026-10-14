"""Build a portable local release from already qualified artifacts; never publish it."""
import hashlib
import json
import platform
import shutil
import subprocess
import tarfile
import tomllib
from pathlib import Path

root = Path(__file__).resolve().parents[1]
version = tomllib.loads((root / "crates/dg-cli/Cargo.toml").read_text())["package"]["version"]
if platform.system() != "Linux" or platform.machine() != "x86_64":
    raise SystemExit("This release script qualifies Linux x86_64 only")
name = f"doppelganger-{version}-linux-x86_64"
release = root / "report/releases"
release.mkdir(parents=True, exist_ok=True)
stage = release / name
stage.mkdir()  # Never replace a previously qualified release.
try:
    (stage / "bin").mkdir()
    shutil.copy2(root / "target/release/doppelganger", stage / "bin/doppelganger")
    for file in ["LICENSE", "SECURITY.md", "CHANGELOG.md", "Cargo.lock"]:
        shutil.copy2(root / file, stage / file)
    shutil.copytree(root / "docs", stage / "docs")
    (stage / "README.md").write_text(f"# Doppelganger {version}\n\nRun `bin/doppelganger --version`, then `bin/doppelganger doctor examples/vault` and `bin/doppelganger test examples/vault --ci`. Vault, Escrow and Record examples carry all their program and IDL artifacts. No Rust, Anchor, Solana CLI or cluster is required to run these prepared cases. See docs/getting-started.md for your own project.\n")
    qualification = stage / "qualification"
    qualification.mkdir()
    for filename in ["qualification.json", "audit.json", "account-scale.json", "snapshot-scale.json", "acceptance.log"]:
        shutil.copy2(root / "report/mvp-qualification" / filename, qualification / filename)
    for source, destination in [("vault-runtime", "vault"), ("escrow-runtime", "escrow"), ("record-runtime", "record")]:
        base = root / "examples" / source
        output = stage / "examples" / destination
        output.mkdir(parents=True)
        text = (base / "doppelganger.toml").read_text()
        manifest = tomllib.loads(text)
        for label in ["v1", "v2"]:
            for field, suffix in [("program_path", "bin"), ("idl", "json")]:
                relative = manifest["versions"][label][field]
                source_path = (base / relative).resolve(strict=True)
                filename = f"{label}.{suffix}"
                shutil.copy2(source_path, output / filename)
                text = text.replace(f'{field} = "{relative}"', f'{field} = "{filename}"', 1)
        (output / "doppelganger.toml").write_text(text)
        shutil.copy2(base / "doppelganger-policy.toml", output / "doppelganger-policy.toml")
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"], cwd=root))
    notices = stage / "licenses"
    notices.mkdir()
    dependencies = []
    for package in metadata["packages"]:
        if package["source"] is None:
            continue
        directory = Path(package["manifest_path"]).parent
        identifier = f'{package["name"]}-{package["version"]}'
        files = set()
        for child in directory.iterdir():
            if child.is_file() and any(word in child.name.upper() for word in ["LICENSE", "LICENCE", "COPYING", "NOTICE"]):
                files.add(child)
        if package.get("license_file"):
            path = directory / package["license_file"]
            if path.is_file():
                files.add(path)
        for file in files:
            destination = notices / identifier / file.name
            destination.parent.mkdir(exist_ok=True)
            shutil.copy2(file, destination)
        dependencies.append({"name": package["name"], "version": package["version"], "license": package["license"], "repository": package.get("repository"), "license_files": sorted(file.name for file in files)})
    (stage / "THIRD-PARTY-NOTICES.json").write_text(json.dumps(dependencies, indent=2) + "\n")
    checksums = []
    for path in sorted(stage.rglob("*")):
        if path.is_file():
            checksums.append(f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(stage)}')
    (stage / "SHA256SUMS").write_text("\n".join(checksums) + "\n")
    archive = release / f"{name}.tar.gz"
    with tarfile.open(archive, "x:gz") as tar:
        tar.add(stage, arcname=name)
    (release / f"{name}.tar.gz.sha256").write_text(f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n')
    print(archive)
except Exception:
    shutil.rmtree(stage)
    raise
