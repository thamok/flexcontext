use flexcontext::{SearchOptions, SearchSession, search};
use serde_json::{Value, json};

#[test]
fn resident_scores_match_cli_and_refresh_replaces_changed_and_deleted_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.ts");
    std::fs::write(
        &path,
        "export function authenticateUser(token: string) { return token; }\n",
    )
    .unwrap();
    let mut session = SearchSession::open(dir.path(), true).unwrap();
    let resident = session.query("auth", 4096, 12).unwrap();
    let cli = search(&SearchOptions {
        root: dir.path().into(),
        query: "auth".into(),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(resident.results[0].score, cli.results[0].score);
    assert_eq!(resident.stats.cache_load_us, 0);
    assert_eq!(resident.stats.traversal_us, 0);
    assert_eq!(resident.stats.files_parsed, 0);
    std::fs::write(
        &path,
        "export function authorizationPolicy() { return false; }\n",
    )
    .unwrap();
    assert_eq!(
        session.query("auth", 4096, 12).unwrap().results[0].symbol,
        "authenticateUser"
    );
    session.refresh().unwrap();
    assert_eq!(
        session.query("auth", 4096, 12).unwrap().results[0].symbol,
        "authorizationPolicy"
    );
    std::fs::remove_file(path).unwrap();
    session.refresh().unwrap();
    assert!(session.query("auth", 4096, 12).unwrap().results.is_empty());
}

#[test]
fn stdio_lifecycle_errors_and_repeated_queries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("auth.ts"),
        "function auth() { return true; }",
    )
    .unwrap();
    let mut session = SearchSession::open(dir.path(), false).unwrap();
    let requests = [
        json!({"jsonrpc":"2.0","id":0,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"code_search","arguments":{"query":"auth","budget":128}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"code_search","arguments":{"query":"auth","budget":128}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"code_search","arguments":{"query":"auth","budget":0}}}),
        json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"refresh_index","arguments":{}}}),
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"code_search","arguments":{"query":"!!!"}}}),
    ];
    let input = requests
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n{bad json\n";
    let mut output = Vec::new();
    flexcontext::mcp::serve(&mut session, input.as_bytes(), &mut output).unwrap();
    let responses: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 9);
    assert_eq!(responses[0]["error"]["code"], -32002);
    assert_eq!(responses[1]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(responses[2]["result"]["tools"].as_array().unwrap().len(), 2);
    for response in &responses[3..5] {
        assert_eq!(
            response["result"]["structuredContent"]["results"][0]["symbol"],
            "auth"
        );
        assert_eq!(
            response["result"]["structuredContent"]["stats"]["cache_load_us"],
            0
        );
    }
    assert_eq!(responses[5]["error"]["code"], -32602);
    assert_eq!(responses[6]["result"]["isError"], false);
    assert_eq!(responses[7]["result"]["isError"], true);
    assert_eq!(responses[8]["error"]["code"], -32700);
}

#[test]
fn graph_does_not_resolve_loader_or_external_types_to_unrelated_locals() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("auth.ts"), "import type { Document } from 'mongoose';\nexport interface IPluginAuth extends Document { auth: string }\nexport function findAuth(): IPluginAuth { return require('./auth'); }\n").unwrap();
    std::fs::write(
        dir.path().join("vite.ts"),
        "const require = makeRequire();\nfunction test() { type Document = string; }\n",
    )
    .unwrap();
    let response = search(&SearchOptions {
        root: dir.path().into(),
        query: "auth".into(),
        use_cache: false,
        ..Default::default()
    })
    .unwrap();
    let relations: Vec<_> = response.results.iter().flat_map(|r| &r.relations).collect();
    assert!(
        !relations
            .iter()
            .any(|r| r.kind == "calls" && r.symbol == "require")
    );
    assert!(!relations.iter().any(|r| r.symbol == "type"));
    assert!(
        !relations
            .iter()
            .any(|r| r.kind == "type_reference" && r.path == "vite.ts")
    );
    assert!(
        relations
            .iter()
            .any(|r| r.kind == "imports" && r.symbol == "Document")
    );
    assert!(
        relations
            .iter()
            .any(|r| r.kind == "type_reference" && r.symbol == "IPluginAuth")
    );
}

#[test]
fn slices_late_relevant_branch_with_valid_utf8_ranges_and_budget() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = "// Größer\nfunction authenticateUser(token: string) {\n".to_owned();
    for i in 0..100 {
        source.push_str(&format!("  const padding{i} = 'irrelevant';\n"));
    }
    source.push_str("  if (!token) { throw new Error('auth denied 🔒'); }\n  return token;\n}\n");
    std::fs::write(dir.path().join("auth.ts"), &source).unwrap();
    let session = SearchSession::open(dir.path(), false).unwrap();
    let whole = session.query("auth", 32768, 12).unwrap();
    assert!(
        !whole
            .results
            .iter()
            .find(|r| r.symbol == "authenticateUser")
            .unwrap()
            .content_truncated
    );
    for budget in [512, 2048, 4096, 8192] {
        let response = session.query("auth", budget, 12).unwrap();
        assert!(response.stats.returned_bytes <= budget);
        assert!(response.stats.approximate_tokens <= budget / 4);
        let function = response
            .results
            .iter()
            .find(|r| r.symbol == "authenticateUser")
            .unwrap();
        assert!(function.content_truncated);
        if budget >= 2048 {
            assert!(function.content.contains("auth denied 🔒"));
        }
        for span in &function.source_spans {
            assert!(
                function
                    .content
                    .contains(&source[span.start_byte..span.end_byte])
            );
            assert_eq!(
                span.start_line,
                source[..span.start_byte]
                    .bytes()
                    .filter(|b| *b == b'\n')
                    .count()
                    + 1
            );
        }
    }
}

#[test]
fn field_postings_preserve_all_scores_for_composite_morphological_and_long_queries() {
    use flexcontext::{
        cache::load_indexed_repository,
        lexical::Query,
        ranking::{PreparedIndex, rank_indexed_candidates, rank_prepared_candidates},
        repository::discover_source_paths,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mini");
    let (paths, _) = discover_source_paths(&root).unwrap();
    let repository = load_indexed_repository(&root, &paths, false).unwrap();
    let prepared = PreparedIndex::build(&repository.symbols);
    for input in [
        "auth",
        "authentication",
        "validate token",
        "UserSession",
        "parseRequest",
        "configuration",
        "auth auth",
        "🔒",
        &"auth ".repeat(65),
    ] {
        let query = Query::parse(input);
        let reference = rank_indexed_candidates(&repository.symbols, &query, &repository.index);
        let indexed =
            rank_prepared_candidates(&repository.symbols, &query, &repository.index, &prepared);
        assert_eq!(reference.len(), indexed.len(), "{input}");
        for (left, right) in reference.iter().zip(&indexed) {
            assert_eq!(left.symbol_id, right.symbol_id, "{input}");
            assert_eq!(
                serde_json::to_value(&left.signals).unwrap(),
                serde_json::to_value(&right.signals).unwrap(),
                "{input}"
            );
        }
    }
}

#[test]
fn soft_diversity_retains_anchor_and_promotes_another_path_and_kind() {
    use flexcontext::{
        model::{Language, ScoreSignals, ScoredSymbol, SourceFile},
        parser::SymbolExtractor,
        relations::RelationGraph,
        selection::select_context,
    };
    let file = |path: &str, source: &str| SourceFile {
        absolute_path: path.into(),
        relative_path: path.into(),
        language: Language::TypeScript,
        source: source.into(),
    };
    let mut extractor = SymbolExtractor::new();
    let mut id = 0;
    let mut symbols = extractor
        .extract(
            &file(
                "one.ts",
                "function auth() {}\nfunction authTwo() {}\nfunction authThree() {}",
            ),
            &mut id,
        )
        .unwrap();
    symbols.extend(
        extractor
            .extract(
                &file("two.ts", "interface AuthConfig { token: string }"),
                &mut id,
            )
            .unwrap(),
    );
    let ranked: Vec<_> = symbols
        .iter()
        .enumerate()
        .map(|(i, symbol)| ScoredSymbol {
            symbol_id: symbol.id,
            score: 30.0 - i as f64 * 0.5,
            signals: ScoreSignals::default(),
        })
        .collect();
    let results = select_context(&ranked, &symbols, &RelationGraph::default(), 4096, 3);
    assert_eq!(results[0].symbol, "auth");
    assert_eq!(results[1].symbol, "AuthConfig");
}
