use std::collections::BTreeMap;
use std::time::Instant;

use anyhow::{Context, Result, bail};

use crate::cache::load_indexed_repository;
use crate::lexical::Query;
use crate::model::{SearchOptions, SearchResponse, SearchStats};
use crate::ranking::{PreparedIndex, rank_indexed_candidates, rank_prepared_candidates};
use crate::relations::{RelationIndex, build_relation_graph_for, expand_ranked};
use crate::repository::discover_source_paths;
use crate::selection::select_context_for_query;

pub fn search(options: &SearchOptions) -> Result<SearchResponse> {
    let started = Instant::now();
    let query = Query::parse(&options.query);
    if query.is_empty() {
        bail!("query must contain at least one letter or number");
    }
    if options.max_bytes == 0 {
        bail!("--max-bytes must be greater than zero");
    }
    let root = options
        .root
        .canonicalize()
        .with_context(|| format!("cannot access repository root {}", options.root.display()))?;

    let stage = Instant::now();
    let (paths, files_scanned) = discover_source_paths(&root)?;
    let traversal_us = stage.elapsed().as_micros();

    let repository = load_indexed_repository(&root, &paths, options.use_cache)?;
    search_repository(
        options,
        &root,
        &repository,
        None,
        None,
        files_scanned,
        traversal_us,
        started,
        false,
    )
}

/// An immutable repository snapshot. Queries perform no filesystem reads.
/// Call refresh after edits to atomically replace the snapshot.
pub struct SearchSession {
    root: std::path::PathBuf,
    repository: crate::cache::IndexedRepository,
    prepared: PreparedIndex,
    relations: RelationIndex,
    use_cache: bool,
    files_scanned: usize,
}

impl SearchSession {
    pub fn open(root: &std::path::Path, use_cache: bool) -> Result<Self> {
        let root = root.canonicalize()?;
        let (paths, files_scanned) = discover_source_paths(&root)?;
        let repository = load_indexed_repository(&root, &paths, use_cache)?;
        let prepared = PreparedIndex::build(&repository.symbols);
        let relations = RelationIndex::build(&repository.symbols);
        Ok(Self {
            root,
            repository,
            prepared,
            relations,
            use_cache,
            files_scanned,
        })
    }

    pub fn refresh(&mut self) -> Result<()> {
        *self = Self::open(&self.root, self.use_cache)?;
        Ok(())
    }

    pub fn query(
        &self,
        query: &str,
        max_bytes: usize,
        max_results: usize,
    ) -> Result<SearchResponse> {
        let options = SearchOptions {
            root: self.root.clone(),
            query: query.to_owned(),
            max_bytes,
            max_results,
            use_cache: self.use_cache,
        };
        search_repository(
            &options,
            &self.root,
            &self.repository,
            Some(&self.prepared),
            Some(&self.relations),
            self.files_scanned,
            0,
            Instant::now(),
            true,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn search_repository(
    options: &SearchOptions,
    root: &std::path::Path,
    repository: &crate::cache::IndexedRepository,
    prepared: Option<&PreparedIndex>,
    relations: Option<&RelationIndex>,
    files_scanned: usize,
    traversal_us: u128,
    started: Instant,
    resident: bool,
) -> Result<SearchResponse> {
    let query = Query::parse(&options.query);
    if query.is_empty() {
        bail!("query must contain at least one letter or number");
    }
    if options.max_bytes == 0 || options.max_results == 0 {
        bail!("budgets must be greater than zero");
    }
    let symbols = &repository.symbols;
    let stage = Instant::now();
    let ranked = if let Some(prepared) = prepared {
        rank_prepared_candidates(symbols, &query, &repository.index, prepared)
    } else {
        rank_indexed_candidates(symbols, &query, &repository.index)
    };
    let candidate_symbols = ranked.len();
    let candidate_and_ranking_us = stage.elapsed().as_micros();

    let stage = Instant::now();
    let shortlist_size = options.max_results.saturating_mul(4).clamp(16, 64);
    let shortlist: Vec<_> = ranked
        .iter()
        .take(shortlist_size)
        .map(|candidate| candidate.symbol_id)
        .collect();
    let graph = relations.map_or_else(
        || build_relation_graph_for(symbols, &shortlist),
        |index| index.graph_for(symbols, &shortlist),
    );
    let ranked = expand_ranked(ranked, &graph, symbols);
    let relationship_us = stage.elapsed().as_micros();

    let stage = Instant::now();
    let results = select_context_for_query(
        &ranked,
        symbols,
        &graph,
        options.max_bytes,
        options.max_results,
        &query,
    );
    let selection_us = stage.elapsed().as_micros();
    let returned_bytes = results.iter().map(|result| result.content_bytes).sum();
    let approximate_tokens = results.iter().map(|result| result.approximate_tokens).sum();
    let stats = SearchStats {
        files_scanned,
        files_parsed: if resident {
            0
        } else {
            repository.files_reparsed
        },
        files_indexed: repository.files_available,
        source_bytes: repository.source_bytes,
        symbols: symbols.len(),
        candidate_symbols,
        returned_symbols: results.len(),
        returned_bytes,
        human_payload_bytes: 0,
        json_payload_bytes: 0,
        approximate_tokens,
        traversal_us,
        cache_load_us: if resident {
            0
        } else {
            repository.cache_load_us
        },
        parse_and_extract_us: if resident {
            0
        } else {
            repository.parse_and_extract_us
        },
        index_us: if resident { 0 } else { repository.index_us },
        candidate_and_ranking_us,
        relationship_us,
        selection_us,
        cache_write_us: if resident {
            0
        } else {
            repository.cache_write_us
        },
        elapsed_us: started.elapsed().as_micros(),
        files_reused: if resident {
            repository.files_available
        } else {
            repository.files_reused
        },
        files_reparsed: if resident {
            0
        } else {
            repository.files_reparsed
        },
        index_reused: resident || repository.index_reused,
    };
    let metadata = BTreeMap::from([
        (
            "snapshot".to_owned(),
            if resident {
                "resident; explicit refresh required after edits"
            } else {
                "fresh filesystem scan"
            }
            .to_owned(),
        ),
        ("retrieval".to_owned(), "structural lexical".to_owned()),
        (
            "token_estimate".to_owned(),
            "bytes divided by four, rounded up per result".to_owned(),
        ),
        (
            "cache".to_owned(),
            if options.use_cache {
                ".flexcontext/index-v3.json"
            } else {
                "disabled"
            }
            .to_owned(),
        ),
    ]);
    let mut response = SearchResponse {
        query: options.query.clone(),
        root: root.to_string_lossy().into_owned(),
        results,
        stats,
        metadata,
    };
    for _ in 0..4 {
        response.stats.elapsed_us = started.elapsed().as_micros();
        response.stats.human_payload_bytes = crate::output::render_human(&response).len();
        response.stats.json_payload_bytes = serde_json::to_vec_pretty(&response)?.len() + 1;
    }
    Ok(response)
}
