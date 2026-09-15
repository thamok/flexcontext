use std::path::PathBuf;

use flexcontext::{SearchOptions, search};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mini")
}

#[test]
fn known_relevant_symbols_have_recall_at_five() {
    let cases = [
        ("auth", ["UserSession", "AuthToken"]),
        (
            "validate token",
            ["validate_token", "decode_bearer_credential"],
        ),
        ("UserSession", ["UserSession", "authenticate_user"]),
        ("config", ["RuntimeConfig", "runtimeConfig"]),
        ("parseRequest", ["parseRequest", "ParsedRequest"]),
    ];
    for (query, relevant) in cases {
        let response = search(&SearchOptions {
            root: fixture(),
            query: query.to_owned(),
            max_bytes: 8_192,
            max_results: 5,
            use_cache: false,
        })
        .unwrap();
        let names: Vec<_> = response
            .results
            .iter()
            .map(|result| result.symbol.as_str())
            .collect();
        let hits = relevant.iter().filter(|name| names.contains(name)).count();
        let recall_at_five = hits as f64 / relevant.len() as f64;
        assert!(
            recall_at_five >= 0.5,
            "query {query:?} recall@5={recall_at_five}; got {names:?}"
        );
    }
}

#[test]
fn context_budget_is_never_exceeded() {
    let response = search(&SearchOptions {
        root: fixture(),
        query: "authentication".to_owned(),
        max_bytes: 500,
        max_results: 20,
        use_cache: false,
    })
    .unwrap();
    assert!(response.stats.returned_bytes <= 500);
    assert_eq!(
        response.stats.returned_bytes,
        response
            .results
            .iter()
            .map(|result| result.content.len())
            .sum::<usize>()
    );
    assert_eq!(
        response.stats.human_payload_bytes,
        flexcontext::output::render_human(&response).len()
    );
    assert_eq!(
        response.stats.json_payload_bytes,
        serde_json::to_vec_pretty(&response).unwrap().len() + 1
    );
}

#[test]
fn persistent_cache_reuses_unchanged_files_and_reparses_only_changes() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("auth.rs");
    std::fs::write(&source, "pub fn authenticate_user() {}\n").unwrap();
    std::fs::write(
        temporary.path().join("session.rs"),
        "pub struct UserSession;\n",
    )
    .unwrap();
    let options = SearchOptions {
        root: temporary.path().to_owned(),
        query: "auth".to_owned(),
        max_bytes: 2_048,
        max_results: 5,
        use_cache: true,
    };

    let cold = search(&options).unwrap();
    assert_eq!(cold.stats.files_reparsed, 2);
    assert!(!cold.stats.index_reused);

    let warm = search(&options).unwrap();
    assert_eq!(warm.stats.files_reparsed, 0);
    assert_eq!(warm.stats.files_reused, 2);
    assert!(warm.stats.index_reused);

    std::fs::write(
        &source,
        "pub fn authenticate_user_with_token(token: &str) -> bool { !token.is_empty() }\n",
    )
    .unwrap();
    let changed = search(&SearchOptions {
        query: "authenticate user token".to_owned(),
        ..options
    })
    .unwrap();
    assert_eq!(changed.stats.files_reparsed, 1);
    assert_eq!(changed.stats.files_reused, 1);
    assert!(!changed.stats.index_reused);
    assert_eq!(changed.results[0].symbol, "authenticate_user_with_token");
}
