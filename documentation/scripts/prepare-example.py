"""Copy the repository's checked-in Vault artifacts into a self-contained project."""
import argparse
from pathlib import Path
import shutil

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', required=True, type=Path, help='A new project directory')
parser.add_argument('--candidate', choices=['good', 'broken'], default='good')
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
source = root / 'examples/vault-runtime-devnet/doppelganger.toml'
artifacts = root / 'fixtures/devnet-snapshots/vault-devnet/artifacts'
text = source.read_text()
replacements = {
    '../../fixtures/devnet-snapshots/vault-devnet/artifacts/v1.bin': 'v1.bin',
    '../../fixtures/devnet-snapshots/vault-devnet/artifacts/v2-good.bin': 'v2.bin',
    '../../fixtures/devnet-snapshots/vault-devnet/artifacts/v1.json': 'v1.json',
    '../../fixtures/devnet-snapshots/vault-devnet/artifacts/v2.json': 'v2.json',
}
for old, new in replacements.items():
    if text.count(old) != 1:
        raise SystemExit(f'Example changed: expected exactly one reference to {old}')
    text = text.replace(old, new)
# All source files are checked before creating the destination.
files = {'v1.bin': artifacts / 'v1.bin', 'v2.bin': artifacts / f'v2-{args.candidate}.bin',
         'v1.json': artifacts / 'v1.json', 'v2.json': artifacts / 'v2.json',
         'doppelganger-policy.toml': root / 'examples/vault-runtime/doppelganger-policy.toml'}
for path in files.values():
    if not path.is_file():
        raise SystemExit(f'Missing tracked input: {path}')
args.output.mkdir(parents=True, exist_ok=False)
try:
    for name, path in files.items():
        shutil.copyfile(path, args.output / name)
    (args.output / 'doppelganger.toml').write_text(text)
except BaseException:
    shutil.rmtree(args.output)
    raise
print(f'Prepared {args.candidate} Vault: {args.output}')
