"""Run the documented local workflow without altering existing projects or cases."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', default='doppelganger')
parser.add_argument('--repository', type=Path, default=Path(__file__).resolve().parents[2])
parser.add_argument('--record', type=Path)
args = parser.parse_args()
root = args.repository.resolve()
records = []
def run(command, expected=0, cwd=None):
    env = dict(os.environ, DG_DEVNET_RPC_URL='http://127.0.0.1:1')
    result = subprocess.run(command, cwd=cwd or root, env=env, text=True, capture_output=True)
    records.append({'command': command, 'exit': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr})
    assert result.returncode == expected, records[-1]
    print(f'PASS (exit {expected}): {" ".join(map(str, command))}', flush=True)
    return result.stdout

with tempfile.TemporaryDirectory(prefix='doppelganger-docs-') as task:
    task = Path(task)
    good, bad = task / 'vault', task / 'vault-broken'
    for directory, candidate in [(good, 'good'), (bad, 'broken')]:
        run(['python3', str(Path(__file__).with_name('prepare-example.py')), '--output', str(directory), '--candidate', candidate])
    cli = args.binary
    run([cli, '--version'])
    run([cli, '--help'])
    run([cli, 'doctor', str(good), '--json'])
    run([cli, 'analyze', '--from', str(good / 'v1.json'), '--to', str(good / 'v2.json'), '--account', 'VaultV1:VaultV2', '--format', 'html'])
    for action in ['build', 'migrate', 'verify', 'lifecycle']:
        run([cli, 'runtime', action, str(good), '--output', str(task / f'{action}.json')])
    run([cli, 'migration', 'inspect', str(good)])
    run([cli, 'test', str(good), '--ci', '--output', str(task / 'pass')])
    run([cli, 'migration', 'test', str(good), '--json', '--output', str(task / 'alias')])
    run([cli, 'test', str(bad), '--ci', '--output', str(task / 'fail')], 1)
    passed = json.loads((task / 'pass/report.json').read_text())
    failed = json.loads((task / 'fail/report.json').read_text())
    assert passed['result'] == 'PASS' and failed['result'] == 'FAIL'
    finding = next(f for f in failed['preservation']['findings'] if f['id'] == 'PRESERVE-balance-field')
    assert finding['status'] == 'FAIL'
    for directory in ['pass', 'fail']:
        assert all((task / directory / name).is_file() for name in ['report.json', 'report.html', 'summary.md'])
    run([cli, 'test', str(task / 'missing'), '--json', '--output', str(task / 'error')], 2)
    run([cli, 'regression', 'capture', '--from-report', str(task / 'fail/report.json'), '--store', str(task / 'portable')])
    run([cli, 'regression', 'replay', str(task / 'portable'), '--output', str(task / 'replay')])
    native = json.loads(run([cli, 'regression', 'create', '--from-report', str(task / 'fail/report.json'), '--manifest', str(bad), '--finding', 'PRESERVE-balance-field', '--store', str(task / 'cases')]))
    case = next(p.parent for p in (task / 'cases').glob('*/case.json'))
    run([cli, 'regression', 'list', '--store', str(task / 'cases')])
    run([cli, 'regression', 'inspect', str(case)])
    reproduced = json.loads(run([cli, 'regression', 'reproduce', str(case)]))
    fixed = json.loads(run([cli, 'regression', 'reproduce', str(case), '--candidate', str(good / 'v2.bin'), '--candidate-idl', str(good / 'v2.json')]))
    assert reproduced['result'] == 'REPRODUCED' and fixed['result'] == 'NOT_REPRODUCED'
    snapshot = root / 'fixtures/devnet-snapshots/vault-devnet'
    project = root / 'examples/vault-runtime-devnet'
    policy = root / 'examples/vault-runtime/doppelganger-policy.toml'
    run([cli, 'snapshot', '--help'])
    run([cli, 'snapshot', 'inspect', str(snapshot)])
    run([cli, 'snapshot', 'replay', str(snapshot), '--project', str(project), '--lifecycle', '--output', str(task / 'snapshot.json')])
    run([cli, 'test', str(project), '--policy', str(policy), '--snapshot', str(snapshot), '--ci', '--output', str(task / 'frozen')])
    # Registry mutations are isolated from the developer's registry.
    run([cli, 'project', 'add', str(good)], cwd=task)
    run([cli, 'project', 'list'], cwd=task)
    # Registered static analysis still expects the legacy project schema.
    run([cli, 'project', 'analyze', 'vault-runtime', '--format', 'json'], expected=1, cwd=task)
    legacy = task / 'legacy.toml'
    legacy.write_text(f'name = "vault-schema"\nprogram_id = "DTbPK89xmJH65ay3YyJTZnPnDWsyXU3KNLiEvXQ7bjoT"\n[v1]\nidl = "{good / "v1.json"}"\n[v2]\nidl = "{good / "v2.json"}"\n[[accounts]]\nv1 = "VaultV1"\nv2 = "VaultV2"\n')
    run([cli, 'project', 'add', str(legacy)], cwd=task)
    run([cli, 'project', 'analyze', 'vault-schema', '--format', 'json'], cwd=task)
    run([cli, 'project', 'check', 'vault-schema', str(task / 'legacy-report.json')], cwd=task)
    if args.record:
        args.record.parent.mkdir(parents=True, exist_ok=True)
        args.record.write_text(json.dumps({'commands': records, 'pass_summary': passed['policy'], 'failed_finding': finding}, indent=2) + '\n')
print(f'Verified {len(records)} CLI commands. Temporary projects removed.')
