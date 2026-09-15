# Retrieval pass: measured results

Measured on the local AgentX checkout on 2026-09-15 in a release build. The snapshot contained **96,293 symbols**, **3,516 indexed files**, and **25,371,254 source bytes**. `auth` retained **5,292 scored candidates**. Platform, binary SHA-256, timestamps and individual query statistics are in [agentx-resident.json](agentx-resident.json).

## Resident MCP round trips

One process, one repository load, one discarded warmup per query, then 20 sequential measured requests. Timing includes request/response JSON serialization and Python client parsing. No repeated-query response cache is used. Startup from an existing v3 disk cache, including feature/posting construction and initialization: **1.75 seconds**. The initial v2-to-v3 rebuild was measured earlier at roughly 7.1 seconds, before the final field-posting implementation; it is not directly comparable to final startup.

| Query | Median ms | p95 ms | Ranking median ms | Relations median ms | Selection median ms |
|---|---:|---:|---:|---:|---:|
| `auth` | 4.77 | 6.37 | 3.00 | 0.48 | 0.41 |
| `authentication` | 4.30 | 4.93 | 2.35 | 0.43 | 0.67 |
| `validate token` | 4.92 | 7.32 | 3.58 | 0.28 | 0.36 |
| `IPluginAuth` | 6.38 | 7.04 | 4.28 | 0.35 | 1.07 |
| `createRemoteAgentAuth` | 38.45 | 48.40 | 29.09 | 4.37 | 4.18 |

Warm traversal, disk-cache load, and repository parse/extract timings are zero. Oversized result excerpts can still invoke Tree-sitter during selection. This is one machine/run, not a universal latency guarantee. There is no current peak-memory measurement; resident source data, lexical features/postings, and graph lookup tables trade memory and startup work for query speed.

The top four `auth` results remain `auth`, `IPluginAuth`, `findOnePluginAuth`, and `deletePluginAuth`. Later positions change under diversity-aware packing; this is not evidence that every new ordering is better. Whole source returned for this query is 8,565 bytes with 2,146 estimated source tokens. Pretty JSON is 42,699 bytes, and MCP wrapping adds more. Token-budget claims apply to source estimates only.

## One-shot CLI remains startup-bound

The same release binary's cached CLI `auth` query took **496.0 ms** internally: **240.7 ms** cache load and **116.8 ms** ranking. It does not build the full resident field index for one query. [Raw statistics](agentx-cli-stats.json).

The supplied earlier attachment reported 687.2 ms total, 276.7 ms cache load and 257.2 ms ranking. That is historical user-supplied evidence, not a controlled before/after run. The resident query numbers exclude startup and must not be described as a cold-CLI speedup.

## Repository-question evidence

[agentx-task-evidence.json](agentx-task-evidence.json) records three concrete repository questions, grounded against current source before querying:

| Question | 2048-token budget | 4096-token budget | 8192-token budget |
|---|---|---|---|
| Plugin-auth lookup filters / optional plugin key | 4/4 evidence fragments | 4/4 | 4/4 |
| Delete-all versus single-entry deletion | 6/6 evidence fragments | 6/6 | 6/6 |
| Four remote-agent API-key fallback branches | 2/4 conditions | 4/4 conditions | 4/4; full function |

At 2048 tokens, the remote-auth excerpt is 2,041 bytes and omits the disabled-OIDC and missing-token condition headers. At 4096 tokens it is a 3,845-byte excerpt containing all four expected condition headers. At 8192 tokens the complete 4,191-byte function fits. Fragment coverage does **not** prove full control-flow understanding or answer correctness. Omission markers and source ranges are essential; consumers must read complete source before making exhaustive behavioral claims.

The existing mixed-language relevance fixture has mean recall@5 **0.958** and mean precision@5 **0.633**. Its small authored cases cannot establish repository-wide retrieval quality. [Raw fixture results](mini-quality.json).

**Autonomous agent-task benchmarks remain unmeasured.** These runs exercise real MCP retrieval and task evidence, not an independent agent answering questions or implementing fixes. No agent success-rate claim is made.

## Verification

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`: 19 tests passed
- `cargo build --release`

New regression coverage checks resident/CLI score parity (including morphology, composite queries and the long-query fallback), repeated MCP calls and protocol errors, snapshot refresh after edits/deletions, loader/type-import graph regressions, diversity ordering, UTF-8 source spans, late relevant branches, and budget compliance.
