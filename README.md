# flexcontext

This is flexcontext, the fast lexical context generator for agents.

flexcontext performs **structural lexical code retrieval**, not ordinary text search. It uses Tree-sitter to decompose source files into language-level units, ranks those units with explicit lexical signals, expands strong matches through conservative syntactic relationships, and returns the most useful structures that fit a context budget.

It does not claim compiler-grade semantic understanding. Name resolution is local, syntactic, and deliberately conservative.

## Install and use

Build the release binary:

```console
cargo build --release
```

Search a repository:

```console
./target/release/flexcontext --code-search . auth
./target/release/flexcontext --code-search . "validate token" --max-bytes 12000
```

Request JSON, including every score component and timing statistic:

```console
./target/release/flexcontext --code-search . auth --json
```

Useful flags:

- `--max-bytes <N>` limits returned UTF-8 source bytes (default: 16 KiB).
- `--budget <N>` sets an approximate source-token budget, e.g. 2048, 4096, or 8192. If both byte and token limits are supplied, the smaller limit wins.
- `--max-results <N>` limits returned structural units (default: 12).
- `--json` emits explicit serializable result and statistics types.
- `--no-cache` bypasses the repository-local incremental index.

The token count is intentionally only an estimate: returned bytes divided by four, rounded up per result. No model-specific tokenizer is used.

## Resident MCP mode

```console
./target/release/flexcontext --mcp /absolute/path/to/repository
```

This starts a synchronous stdio server implementing the [MCP 2025-11-25 lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle). Stdout contains newline-delimited JSON-RPC only. It exposes:

- `code_search`: `{ "query": "auth", "budget": 4096, "max_results": 12 }`
- `refresh_index`: `{}`

The server loads the repository once, builds field-specific lexical postings and graph lookup tables, and answers repeated queries without traversal, JSON loading, parsing, or field tokenization. The disk cache retains its compact candidate index; the richer field postings are resident-only. Startup preparation is therefore separate from query latency.

**Snapshot semantics:** call `refresh_index` after edits, additions, or deletions. Queries otherwise continue to read the loaded snapshot. Refresh replaces the snapshot only after rebuilding successfully. Restarting also refreshes it. There is no automatic file watcher.

`budget` measures approximate **source** tokens, not the complete MCP payload. Signatures, scores, relationships, metadata, and protocol wrapping add overhead. Results retain their lexical scores even when diversity changes output order. Sliced content is explicitly incomplete, is not executable code, and may omit relevant control-flow conditions. Use `source_spans` for exact original-file byte/line ranges and increase the budget or read the source for complete behavior.

## Supported languages

| Language | Extensions | Representative units |
|---|---|---|
| Rust | `.rs` | functions, methods, structs, enums, traits, impls, modules, types, constants/statics, imports |
| TypeScript / TSX | `.ts`, `.mts`, `.cts`, `.tsx` | functions, methods, classes, interfaces, types, enums, declarations, imports |
| JavaScript | `.js`, `.mjs`, `.cjs`, `.jsx` | functions, methods, classes, declarations, imports |
| Python | `.py`, `.pyi` | functions, methods, classes, imports |

Language detection and grammar configuration live in `language.rs`; extraction is grammar-aware but shares one traversal, making another Tree-sitter language a localized addition.

## Retrieval pipeline

1. `ignore` traverses the repository while respecting Git ignore files, hidden-file conventions, and common dependency/build directories.
2. File size and modification time are compared with the schema-versioned `.flexcontext/index-v3.json` cache.
3. Only new or changed files are parsed. Changed files are processed in parallel, with each worker reusing one Tree-sitter parser per language.
4. The extractor creates structural units with names, kinds, containing symbols, byte/line ranges, declarations, bodies, attached comments or docstrings, file imports, identifiers, type references, and cheap call references.
5. Query and identifier text is segmented across camelCase, PascalCase, snake_case, kebab-case, paths, and punctuation. A small deterministic morphology rule relates forms such as `authenticate`/`authentication` and `validate`/`validation`.
6. A persisted inverted lexical index retrieves a candidate ID set. Only those candidates are scored.
7. Top-level functions, types, and modules receive a structural prior; throwaway local declarations receive a penalty.
8. Only the bounded lexical shortlist is examined for calls, explicit type references, containment, members, and associated file imports. Strong seeds receive a one-hop boost.
9. A deterministic selector packs a bounded shortlist (32 times the result limit, clamped to 64–4096 candidates), retaining the first lexical anchor and discounting repeated paths and kinds. Existing name/structural cluster limits and overlap removal still apply.
10. Each symbol receives at most a quarter of the byte budget (clamped to 256–8192 bytes). Oversized functions/methods return their signature and query-ranked complete AST statements or branches, with omission markers and original-file source spans. Other oversized containers return compact declarations. Larger budgets can retain entire functions. A non-container declaration can still use the remaining global budget.
11. Per-result token rounding is reserved during packing, so the sum of source-token estimates fits `--budget`.
12. Human or JSON output exposes source boundaries, relations, cache activity, source bytes, complete payload bytes, scores, and stage timings.

The one-shot library entry point is `flexcontext::search(SearchOptions)`. `SearchSession::open(root, use_cache)` prepares a resident snapshot; `query(text, max_bytes, max_results)` reuses it and `refresh()` replaces it after edits. Traversal, extraction, ranking, relationship expansion, and selection are public modules so they can be benchmarked or wrapped by a later interface without coupling that interface to the CLI.

## Ranking weights in v0.0.1

All signals add linearly and remain visible in JSON.

| Signal | Maximum / rule |
|---|---:|
| Case-insensitive exact symbol name | 12.0 |
| Exact normalized symbol name | 10.0 |
| Normalized prefix/suffix relation | 3.0 |
| Symbol-name query-token overlap | 6.0 |
| Containing symbol | 2.5 plus a small normalized-substring bonus |
| File path | 2.5 plus a small normalized-substring bonus |
| Comment/docstring | 2.0 plus a small normalized-substring bonus |
| Referenced identifiers | 2.5 plus a small normalized-substring bonus |
| Signature/declaration | 2.0 plus a small normalized-substring bonus |
| Body | 1.2 plus a small normalized-substring bonus |
| Query-term coverage across useful fields | 3.0 |
| Match density | up to 1.5 |
| Structural priority | top-level function/type/module 2.0, method 0.75, top-level declaration 0.5, local declaration -2.0, import -0.5; local declarations receive an additional -32.0 when an index lookup produces at least eight candidates |
| Structural relation | calls 4.0, enclosing structure 3.0, explicit type reference 2.5, member 2.0, import 0.75; accumulated boost capped at 6.0 |
| Oversized-unit penalty | `-0.35 * log2(bytes / 2048)`, capped at -3.0 |

Exact spelling and normalized exactness are separate signals by design. Ties are resolved deterministically by path, start line, and symbol name.

## Relationship resolution

The parser records containing structures, explicit type identifiers, call-expression targets, and imports. Generic identifier-to-definition edges are intentionally not created. A shortlisted reference is linked only when there is exactly one plausible definition repository-wide or exactly one same-file definition. Cross-file candidates must be top-level, and type references must target a type-bearing definition. `require` and enclosing structures are resolved only within their source file. Type-only imports obtain their display name from syntax identifiers, excluding the `type` keyword. This avoids presenting syntactic guesses as compiler-level resolution.

Relations do not expand recursively in v0.0.1. Only the top lexical seeds contribute a bounded one-hop boost, preventing a common name from flooding the context.

## Incremental cache and index

The cache stores Tree-sitter-derived structural units per source file and an inverted lexical posting index. A file is reused when its byte size and nanosecond modification timestamp match. Deleted files disappear from the next index; changed and new files alone are reparsed. Cache installation uses a temporary file followed by an atomic rename.

The cache is an implementation index, not a service or database, and can be bypassed with `--no-cache`. JSON statistics distinguish reused/reparsed files, reused/rebuilt indexes, cache time, source-context bytes, human payload bytes, and pretty-JSON payload bytes.

## Benchmarks

Criterion benchmarks cover repository traversal, Tree-sitter parsing plus extraction, index construction, indexed candidate generation plus ranking, shortlisted relationship expansion, context selection, uncached end-to-end retrieval, and cached end-to-end retrieval:

```console
cargo bench --bench pipeline
```

The comparison binary runs identical literal queries through `rg` and structural queries through flexcontext, recording output bytes and wall-clock latency:

```console
./target/release/flexcontext-bench .
```

The checked-in mixed Rust/TypeScript/Python/JavaScript fixture includes human-authored relevance judgments. It reports precision@k, recall@k, and relevant returned bytes / all returned bytes:

```console
./target/release/flexcontext-bench tests/fixtures/mini \
  --quality-file tests/fixtures/mini/relevance.json \
  --quality-k 5
```

`rg` is expected to be faster. The benchmark tests whether extra retrieval work improves the density and structural usefulness of agent context, not whether parsing can beat literal byte search.

### Resident latency and repository-question evidence

```console
python3 scripts/bench_resident.py /path/to/abilex-agentx --repeats 20
python3 scripts/bench_tasks.py /path/to/abilex-agentx
```

The first script measures real MCP round trips, reports startup separately, discards one warmup per query, and reports median/p95 over repeated calls. The second checks evidence coverage for three concrete AgentX questions at 2048/4096/8192-token budgets. It validates its expected snippets against the checkout first. These are retrieval/evidence tests, not autonomous agent code-change success benchmarks. See [measured results and limitations](benches/results/README.md).

## Development checks

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## Deliberate v0.0.1 limits

There are no embeddings, vector databases, LSP interfaces, compiler-grade name resolution, Git history, remote repository support, background file watching, or async runtime. Files above 2 MiB and non-UTF-8 files are skipped. The persistent cache is deliberately replaceable and repository-local; it is not a cross-repository database or background service.
