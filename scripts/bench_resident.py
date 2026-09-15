#!/usr/bin/env python3
"""Measure real newline-delimited MCP round trips; keep startup separate."""
import argparse
import json
import math
import hashlib
import platform
from datetime import datetime, timezone
from pathlib import Path
import statistics
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument('root')
parser.add_argument('--binary', default='./target/release/flexcontext')
parser.add_argument('--repeats', type=int, default=20)
parser.add_argument('--output')
args = parser.parse_args()
if args.repeats < 1:
    parser.error("--repeats must be positive")
started = time.perf_counter()
process = subprocess.Popen([args.binary, '--mcp', args.root], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
request_id = 0

def call(method, params):
    global request_id
    request_id += 1
    t = time.perf_counter()
    process.stdin.write(json.dumps(dict(jsonrpc='2.0', id=request_id, method=method, params=params)) + '\n')
    process.stdin.flush()
    line = process.stdout.readline()
    if not line:
        raise RuntimeError('MCP server exited')
    response = json.loads(line)
    if 'error' in response:
        raise RuntimeError(response['error'])
    return response['result'], (time.perf_counter() - t) * 1000

try:
    call('initialize', dict(protocolVersion='2025-11-25', capabilities={}, clientInfo=dict(name='resident-bench', version='1')))
    startup_ms = (time.perf_counter() - started) * 1000
    process.stdin.write(json.dumps(dict(jsonrpc='2.0', method='notifications/initialized')) + '\n')
    process.stdin.flush()
    results = []
    for query in ['auth', 'authentication', 'validate token', 'IPluginAuth', 'createRemoteAgentAuth']:
        timings, ranking, relations, selection = [], [], [], []
        for i in range(args.repeats + 1):
            result, elapsed = call('tools/call', dict(name='code_search', arguments=dict(query=query, budget=4096)))
            if result.get('isError'):
                raise RuntimeError(result)
            response = result['structuredContent']
            if i:
                timings.append(elapsed)
                ranking.append(response['stats']['candidate_and_ranking_us'] / 1000)
                relations.append(response['stats']['relationship_us'] / 1000)
                selection.append(response['stats']['selection_us'] / 1000)
        results.append(dict(query=query, median_ms=statistics.median(timings), p95_ms=sorted(timings)[max(0, math.ceil(len(timings)*.95)-1)], ranking_median_ms=statistics.median(ranking), relationships_median_ms=statistics.median(relations), selection_median_ms=statistics.median(selection), stats=response['stats'], symbols=[dict(symbol=r['symbol'], path=r['path'], bytes=r['content_bytes'], sliced=r['content_truncated']) for r in response['results']]))
    budgets = []
    for budget in [2048, 4096, 8192]:
        result, elapsed = call('tools/call', dict(name='code_search', arguments=dict(query='auth', budget=budget)))
        response = result['structuredContent']
        budgets.append(dict(budget=budget, latency_ms=elapsed, returned_tokens=response['stats']['approximate_tokens'], returned_bytes=response['stats']['returned_bytes'], symbols=[r['symbol'] for r in response['results']]))
    report = dict(root=args.root, measured_at=datetime.now(timezone.utc).isoformat(), platform=platform.platform(), binary_sha256=hashlib.sha256(Path(args.binary).read_bytes()).hexdigest(), startup_ms=startup_ms, repeats=args.repeats, results=results, budgets=budgets)
    text = json.dumps(report, indent=2)
    if args.output:
        with open(args.output, 'w') as out:
            out.write(text + '\n')
    print(text)
finally:
    process.stdin.close()
    process.wait(timeout=30)
