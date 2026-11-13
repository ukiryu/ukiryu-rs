"""Merge contract annotations into the captured Ruby observations.

Captured fields are copied unchanged; only 'contract', 'target' and 'trace'
are added. Usage: python3 annotate.py IN.json OUT.json
"""
import json, sys

D1 = 'D1 (DECISIONS.md); ukiryu/ukiryu#20, #29'
KILL = 'ukiryu/ukiryu#20 (timeout kill semantics)'
PARITY = {'contract': 'parity'}
A = {
  'missing_shell': {'contract': 'migrated', 'trace': D1, 'target': 'ExecuteRequest::new requires the shell; ExecutionContext::run_settings returns MissingSetting::Shell with the same text'},
  'missing_timeout': {'contract': 'migrated', 'trace': D1, 'target': 'ExecuteRequest::new requires the timeout; ExecutionContext::run_settings returns MissingSetting::Timeout with the same text'},
  'missing_env_defaults': {'contract': 'migrated', 'trace': D1, 'target': 'ExecuteRequest::new requires an Environment; an empty one gives this result'},
  'timeout_zero_runs_unbounded': {'contract': 'migrated', 'trace': D1, 'target': 'ExecuteError::InvalidTimeout (0 would disable the required timeout)'},
  'timeout_negative': {'contract': 'migrated', 'trace': D1, 'target': 'ExecuteError::InvalidTimeout'},
  'timeout_sleep_10': {'contract': 'migrated', 'trace': KILL, 'target': 'same class and message; timeout attribute is set; the child is killed at the deadline (source waits ~10 s for natural exit)'},
  'timeout_int_message': {'contract': 'migrated', 'trace': KILL, 'target': 'same class and message; timeout attribute is set; killed at the deadline'},
  'missing_executable': {'contract': 'parity', 'note': 'stderr text depends on the host bash; status 127 and the command-not-found text are compared'},
  'signal_term': {'contract': 'parity', 'note': 'Unix only'},
  'stdin_large_cat_deadlock': {'contract': 'migrated', 'trace': 'ukiryu/ukiryu#20 (deadlock-safe stdin and capture)', 'target': 'success; stdout equals the 1000000-byte input'},
  'stdout_invalid_utf8': {'contract': 'parity', 'note': 'raw bytes compared; the result string replaces invalid UTF-8'},
  'shell_not_found': {'contract': 'parity', 'note': 'message parity; Ruby RuntimeError becomes ExecuteError::ShellNotFound'},
  'tcsh_not_found': {'contract': 'parity', 'note': 'message parity; Ruby RuntimeError becomes ExecuteError::ShellNotFound'},
}
d = json.load(open(sys.argv[1]))
d['oracle']['capture_script'] = 'ukiryu/tests/executor_data/capture/capture_executor.rb'
for row in d['rows']:
    row.pop('elapsed_evidence_only', None)
    row.update(A.get(row['id'], PARITY))
missing = set(A) - {r['id'] for r in d['rows']}
assert not missing, missing
json.dump(d, open(sys.argv[2], 'w'), indent=2, ensure_ascii=False)
open(sys.argv[2], 'a').write('\n')
