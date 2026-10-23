#!/usr/bin/env python3
"""Local, non-publishing release audit. Keep complete command logs and exit codes."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'dist' / 'v501-shipping-audit'
CHECKS = [
    ('versions', ['python3','-B','scripts/prepare-release.py','check-version','--version','5.0.1']),
    ('format', ['cargo','fmt','--all','--','--check']),
    ('store-format', ['cargo','fmt','--manifest-path','crates/strata-store/Cargo.toml','--','--check']),
    ('native-build', ['cargo','build','-p','vestige-mcp','--offline']),
    ('original-rsb-tests', ['cargo','test','-p','vestige-core','retroactive_backfill','--offline']),
    ('native-rsb-tests', ['cargo','test','-p','vestige-mcp','--test','rsb_personal_stdio','--offline','--','--nocapture']),
    ('workspace-tests', ['cargo','test','--workspace','--no-fail-fast','--offline']),
    ('workspace-clippy', ['cargo','clippy','--workspace','--all-targets','--offline','--','-D','warnings']),
    ('store-tests', ['cargo','test','--manifest-path','crates/strata-store/Cargo.toml','--offline']),
    ('store-clippy', ['cargo','clippy','--manifest-path','crates/strata-store/Cargo.toml','--all-targets','--offline','--','-D','warnings']),
    ('pnpm-lock', ['pnpm','install','--frozen-lockfile','--offline']),
    ('dashboard-check', ['pnpm','--filter','@vestige/dashboard','check']),
    ('dashboard-tests', ['pnpm','--filter','@vestige/dashboard','test']),
    ('dashboard-build', ['pnpm','--filter','@vestige/dashboard','build']),
    ('init-tests', ['node','--test','--test-concurrency=1',*map(str, sorted((ROOT/'packages/vestige-init/test').glob('*.test.js')))]),
    ('npm-binary-tests', ['node','--test',*map(str, sorted((ROOT/'packages/vestige-mcp-npm/tests').glob('*.test.js')))]),
    ('release-preparation-tests', ['python3','-B','-m','unittest','discover','-s','tests','-p','test_prepare_release.py']),
    ('init-pack-preview', ['npm','pack','./packages/vestige-init','--dry-run','--json','--ignore-scripts']),
    ('diff-check', ['git','diff','--check']),
]

def main():
    global OUT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=OUT)
    args = parser.parse_args()
    OUT = args.output.resolve()
    OUT.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, VESTIGE_TEST_MCP_BIN=str(ROOT/'target/debug/vestige-mcp'), npm_config_cache=str(OUT/'npm-cache'))
    results = []
    corpus_home = OUT / 'lite-corpus-home'
    (corpus_home/'Developer/vestige/.git').mkdir(parents=True, exist_ok=True)
    checks = CHECKS[:-1] + [('guardfall', ['env', f'HOME={corpus_home}', f'OPERATOR_HOME={corpus_home}/.operator', 'python3', '-I', 'operator-lite/operator-gate.py', 'test'])] + CHECKS[-1:]
    for name, command in checks:
        print(f'START {name}', flush=True)
        started = time.monotonic()
        with (OUT/f'{name}.log').open('w') as log:
            run = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=False)
        row = {'check':name,'command':command,'exit_code':run.returncode,'elapsed_seconds':round(time.monotonic()-started,2),'log':f'{name}.log'}
        results.append(row)
        (OUT/'results.json').write_text(json.dumps(results,indent=2)+'\n')
        print(f'{"PASS" if run.returncode==0 else "FAIL"} {name} ({row["elapsed_seconds"]}s)',flush=True)
        if run.returncode:
            raise SystemExit(run.returncode)
    raise SystemExit(1 if any(r['exit_code'] for r in results) else 0)

if __name__=='__main__':
    main()
