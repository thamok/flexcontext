#!/usr/bin/env python3
"""Grounded repository-question evidence benchmark; not an autonomous coding-agent eval."""
import argparse
import json
import subprocess
import time
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('root', type=Path)
parser.add_argument('--binary', default='./target/release/flexcontext')
parser.add_argument('--output')
args = parser.parse_args()
cases = [
    dict(question='Which filters constrain plugin-auth lookup, and when is pluginKey applied?',
         query='findOnePluginAuth', symbol='findOnePluginAuth',
         path='apps/chat-agentx/packages/data-schemas/src/methods/pluginAuth.ts',
         evidence=['userId,', 'authField,', '...(pluginKey && { pluginKey })', '.lean<IPluginAuth>()']),
    dict(question='How does deleting all plugin-auth entries differ from deleting one?',
         query='deletePluginAuth', symbol='deletePluginAuth',
         path='apps/chat-agentx/packages/data-schemas/src/methods/pluginAuth.ts',
         evidence=['if (all)', 'const filter: DeletePluginAuthParams = { userId }', 'filter.pluginKey = pluginKey', 'deleteMany(filter)', "throw new Error('authField is required when all is false')", 'deleteOne({ userId, authField })']),
    dict(question='Which four branches fall back to API-key authentication in remote-agent auth?',
         query='createRemoteAgentAuth', symbol='createRemoteAgentAuth',
         path='apps/chat-agentx/packages/api/src/middleware/remoteAgentAuth.ts',
         evidence=['if (authConfig?.oidc?.enabled !== true)', 'if (token == null)', 'catch (oidcErr)', "if (userResolution.status === 'missing')"]),
]
process = subprocess.Popen([args.binary, '--mcp', str(args.root)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
request_id = 0

def call(method, params):
    global request_id
    request_id += 1
    process.stdin.write(json.dumps(dict(jsonrpc='2.0', id=request_id, method=method, params=params)) + '\n')
    process.stdin.flush()
    response = json.loads(process.stdout.readline())
    if 'error' in response:
        raise RuntimeError(response['error'])
    return response['result']

try:
    call('initialize', dict(protocolVersion='2025-11-25', capabilities={}, clientInfo=dict(name='task-evidence-bench', version='1')))
    process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
    process.stdin.flush()
    rows = []
    for case in cases:
        source = (args.root / case['path']).read_text()
        assert all(fragment in source for fragment in case['evidence']), 'Ground truth changed; re-review the task.'
        for budget in [2048, 4096, 8192]:
            started = time.perf_counter()
            result = call('tools/call', dict(name='code_search', arguments=dict(query=case['query'], budget=budget)))
            response = result['structuredContent']
            target = next((r for r in response['results'] if r['path'] == case['path'] and r['symbol'] == case['symbol']), None)
            context = target['content'] if target else ''
            found = [fragment for fragment in case['evidence'] if fragment in context]
            rows.append(dict(question=case['question'], query=case['query'], budget=budget, target_found=target is not None,
                evidence_found=len(found), evidence_total=len(case['evidence']), missing=[fragment for fragment in case['evidence'] if fragment not in context],
                source_bytes=response['stats']['returned_bytes'], target_bytes=target['content_bytes'] if target else 0,
                target_sliced=target['content_truncated'] if target else None, latency_ms=(time.perf_counter()-started)*1000))
    report = dict(method='Exact evidence coverage on three repository questions; no model answer or code-change success is measured.', cases=rows)
    text = json.dumps(report, indent=2)
    if args.output:
        Path(args.output).write_text(text + '\n')
    print(text)
finally:
    process.stdin.close()
    process.wait(timeout=30)
