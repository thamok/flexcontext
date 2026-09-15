# flexcontext

This is flexcontext, the fast lexical context generator for agents.

Find useful code **before you know the right files or symbols**. flexcontext uses Tree-sitter, lexical ranking, and syntactic relationships to retrieve structural source units and fit them into a context budget.

It complements `rg`, rather than replacing it. Retrieval is syntax-aware, not compiler-grade semantic analysis.

**Supported languages:** Rust, TypeScript/TSX, JavaScript/JSX, and Python.

## Quick start

Build:

```console
cargo build --release
```

Search a repository:

```console
./target/release/flexcontext --code-search . "validate token"
```

Return JSON with an approximate source-token budget:

```console
./target/release/flexcontext --code-search . auth --budget 4096 --json
```

| Flag | Description |
|---|---|
| `--max-bytes <N>` | Source-byte limit. Default: 16 KiB. |
| `--budget <N>` | Approximate source-token limit. |
| `--max-results <N>` | Maximum structural units. Default: 12. |
| `--json` | Structured results, score components, and timings. |
| `--no-cache` | Bypass the repository-local incremental index. |

When both budget limits are supplied, the smaller effective limit wins. Token counts are estimates; metadata and protocol overhead are not included.

Oversized units may return explicitly incomplete slices, not executable code. Use `source_spans` to locate the original source and inspect complete behavior.

## MCP

Start a resident stdio server:

```console
./target/release/flexcontext --mcp /absolute/path/to/repository
```

Available tools:

- `code_search`: `{ "query": "auth", "budget": 4096, "max_results": 12 }`
- `refresh_index`: `{}`

Repeated queries reuse the loaded index without traversing or parsing the repository again. **Call `refresh_index` after file changes.** The server uses a snapshot and has no automatic file watcher.

## Performance

The incremental cache reuses unchanged files. New and changed files are parsed in parallel.

`rg` is expected to be faster. The benchmark question is whether structural retrieval produces denser, more useful agent context—not whether parsing beats literal text search.

See [measured results and limitations](benches/results/README.md).

## Development

Check formatting:

```console
cargo fmt --check
```

Lint:

```console
cargo clippy --all-targets --all-features -- -D warnings
```

Run tests:

```console
cargo test
```

Run pipeline benchmarks:

```console
cargo bench --bench pipeline
```