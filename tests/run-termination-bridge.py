#!/usr/bin/env python3
"""Compile and run isolated own-process AppKit tests; no desktop UI automation."""
import json
import platform
import subprocess
import tempfile
from pathlib import Path

app = Path(__file__).resolve().parents[1]
report = app.parent / 'reports' / 'termination-bridge-native-results.json'
if platform.system() != 'Darwin':
    raise SystemExit('AppKit tests require macOS; no passing result recorded.')
results = []
with tempfile.TemporaryDirectory(prefix='branchdesk-appkit-tests-') as tmp:
    for name in ['termination-bridge-test', 'termination-bridge-integration']:
        executable = Path(tmp) / name
        subprocess.run(['clang', '-fobjc-arc', '-Wall', '-Wextra', '-Werror',
                        '-framework', 'AppKit', str(app / 'src-tauri/native/termination.m'),
                        str(app / f'tests/{name}.m'), '-o', str(executable)], check=True)
        try:
            result = subprocess.run([str(executable)], capture_output=True, text=True, timeout=15)
            passed = result.returncode == 0 and 'PASS ' in result.stdout
            results.append({'test': name, 'passed': passed, 'exitCode': result.returncode,
                            'stdout': result.stdout, 'stderr': result.stderr})
        except subprocess.TimeoutExpired as error:
            results.append({'test': name, 'passed': False, 'timeoutSeconds': 15,
                            'stdout': (error.stdout or b'').decode(errors='replace'),
                            'stderr': (error.stderr or b'').decode(errors='replace')})
report.write_text(json.dumps({'platform': platform.platform(), 'tests': results,
                            'scope': 'Own-process delegate and asynchronous AppKit lifecycle; not Tauri IPC or Dock UI'}, indent=2)+'\n')
for result in results:
    print(result['test'], 'PASS' if result['passed'] else 'FAIL')
raise SystemExit(0 if all(result['passed'] for result in results) else 1)
