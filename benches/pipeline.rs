use std::path::Path;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use flexcontext::index::LexicalIndex;
use flexcontext::lexical::Query;
use flexcontext::model::SearchOptions;
use flexcontext::parser::{SymbolExtractor, extract_symbols};
use flexcontext::ranking::rank_indexed_candidates;
use flexcontext::relations::{build_relation_graph_for, expand_ranked};
use flexcontext::repository::{discover_source_paths, load_source_file};
use flexcontext::selection::select_context;

fn fixture_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mini")
        .leak()
}

fn parsed_fixture() -> Vec<flexcontext::model::Symbol> {
    let root = fixture_root();
    let (paths, _) = discover_source_paths(root).unwrap();
    let mut next_id = 0;
    paths
        .iter()
        .filter_map(|path| load_source_file(root, path).unwrap())
        .flat_map(|file| extract_symbols(&file, &mut next_id).unwrap())
        .collect()
}

fn pipeline_benches(criterion: &mut Criterion) {
    let root = fixture_root();
    let fixture_bytes = discover_source_paths(root)
        .unwrap()
        .0
        .iter()
        .map(|path| std::fs::metadata(path).unwrap().len())
        .sum();

    let mut traversal = criterion.benchmark_group("repository_traversal");
    traversal.throughput(Throughput::Bytes(fixture_bytes));
    traversal.bench_function("discover_fixture", |bencher| {
        bencher.iter(|| discover_source_paths(root).unwrap())
    });
    traversal.finish();

    let (paths, _) = discover_source_paths(root).unwrap();
    let files: Vec<_> = paths
        .iter()
        .filter_map(|path| load_source_file(root, path).unwrap())
        .collect();
    let mut parsing = criterion.benchmark_group("tree_sitter_parse_and_extract");
    parsing.throughput(Throughput::Bytes(
        files.iter().map(|file| file.source.len() as u64).sum(),
    ));
    parsing.bench_function("all_languages", |bencher| {
        bencher.iter(|| {
            let mut id = 0;
            let mut extractor = SymbolExtractor::new();
            files
                .iter()
                .flat_map(|file| extractor.extract(file, &mut id).unwrap())
                .collect::<Vec<_>>()
        })
    });
    parsing.finish();

    let symbols = parsed_fixture();
    let query = Query::parse("validate token");
    criterion.bench_function("lexical_index_build", |bencher| {
        bencher.iter(|| LexicalIndex::build(&symbols))
    });
    let index = LexicalIndex::build(&symbols);
    let mut candidates = criterion.benchmark_group("lexical_candidates_and_ranking");
    candidates.throughput(Throughput::Elements(symbols.len() as u64));
    candidates.bench_function("rank", |bencher| {
        bencher.iter(|| rank_indexed_candidates(&symbols, &query, &index))
    });
    candidates.finish();

    let ranked = rank_indexed_candidates(&symbols, &query, &index);
    let shortlist: Vec<_> = ranked.iter().take(16).map(|item| item.symbol_id).collect();
    criterion.bench_function("relationship_graph_and_expansion", |bencher| {
        bencher.iter(|| {
            let graph = build_relation_graph_for(&symbols, &shortlist);
            expand_ranked(ranked.clone(), &graph, &symbols)
        })
    });

    let graph = build_relation_graph_for(&symbols, &shortlist);
    let expanded = expand_ranked(ranked, &graph, &symbols);
    criterion.bench_function("context_selection", |bencher| {
        bencher.iter(|| select_context(&expanded, &symbols, &graph, 16_384, 12))
    });

    criterion.bench_function("end_to_end_retrieval", |bencher| {
        bencher.iter(|| {
            flexcontext::search(&SearchOptions {
                root: root.to_owned(),
                query: "validate token".to_owned(),
                max_bytes: 16_384,
                max_results: 12,
                use_cache: false,
            })
            .unwrap()
        })
    });

    let cached_options = SearchOptions {
        root: root.to_owned(),
        query: "validate token".to_owned(),
        max_bytes: 16_384,
        max_results: 12,
        use_cache: true,
    };
    flexcontext::search(&cached_options).unwrap();
    criterion.bench_function("end_to_end_cached_retrieval", |bencher| {
        bencher.iter(|| flexcontext::search(&cached_options).unwrap())
    });
}

criterion_group!(benches, pipeline_benches);
criterion_main!(benches);
