use std::collections::HashMap;

use crate::language::is_container;
use crate::lexical::{Query, normalize_identifier};
use crate::model::{ScoredSymbol, SearchResult, Symbol};
use crate::relations::{RelationGraph, serializable_relations};

pub fn select_context(
    ranked: &[ScoredSymbol],
    symbols: &[Symbol],
    graph: &RelationGraph,
    max_bytes: usize,
    max_results: usize,
) -> Vec<SearchResult> {
    select_context_for_query(
        ranked,
        symbols,
        graph,
        max_bytes,
        max_results,
        &Query::parse(""),
    )
}

pub fn select_context_for_query(
    ranked: &[ScoredSymbol],
    symbols: &[Symbol],
    graph: &RelationGraph,
    max_bytes: usize,
    max_results: usize,
    query: &Query,
) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut used = 0;
    let mut name_clusters: HashMap<String, usize> = HashMap::new();
    let mut structural_clusters: HashMap<(String, String), usize> = HashMap::new();
    let per_container_limit = (max_bytes / 4).clamp(256, 8 * 1024);
    let mut pending: Vec<_> = ranked
        .iter()
        .take(max_results.saturating_mul(32).clamp(64, 4096))
        .collect();
    let mut paths: HashMap<&str, usize> = HashMap::new();
    let mut kinds: HashMap<&str, usize> = HashMap::new();

    while !pending.is_empty() {
        // Soft diversity: keep the first lexical anchor, then discount repeated paths/kinds.
        // Original rank wins ties; reported lexical scores remain unchanged.
        let next = pending
            .iter()
            .enumerate()
            .max_by(|(ai, a), (bi, b)| {
                let utility = |item: &&ScoredSymbol| {
                    let symbol = &symbols[item.symbol_id];
                    item.score
                        - 2.0 * paths.get(symbol.path.as_str()).copied().unwrap_or(0) as f64
                        - 0.75 * kinds.get(symbol.kind.as_str()).copied().unwrap_or(0) as f64
                };
                utility(a).total_cmp(&utility(b)).then_with(|| bi.cmp(ai))
            })
            .map(|(index, _)| index)
            .unwrap();
        let scored = pending.remove(next);
        if results.len() >= max_results || used >= max_bytes {
            break;
        }
        let symbol = &symbols[scored.symbol_id];
        let name_cluster = normalize_identifier(&symbol.name);
        let name_limit = if symbol.kind == "declaration" { 1 } else { 2 };
        if name_clusters.get(&name_cluster).copied().unwrap_or(0) >= name_limit {
            continue;
        }
        let structural_cluster = (
            symbol.path.clone(),
            symbol
                .containing_symbol
                .clone()
                .unwrap_or_else(|| "<top-level>".to_owned()),
        );
        let structural_limit = if symbol.containing_symbol.is_some() {
            2
        } else {
            3
        };
        if structural_clusters
            .get(&structural_cluster)
            .copied()
            .unwrap_or(0)
            >= structural_limit
        {
            continue;
        }
        if overlaps_selected(symbol, &results) {
            continue;
        }
        let remaining = (max_bytes - used) / 4 * 4;
        let (content, truncated, source_spans) =
            budgeted_content(symbol, remaining, per_container_limit, query);
        if content.is_empty() {
            continue;
        }
        let content_bytes = content.len();
        used += content_bytes.div_ceil(4) * 4;
        *paths.entry(&symbol.path).or_default() += 1;
        *kinds.entry(&symbol.kind).or_default() += 1;
        *name_clusters.entry(name_cluster).or_default() += 1;
        *structural_clusters.entry(structural_cluster).or_default() += 1;
        results.push(SearchResult {
            path: symbol.path.clone(),
            language: symbol.language,
            symbol: symbol.name.clone(),
            kind: symbol.kind.clone(),
            containing_symbol: symbol.containing_symbol.clone(),
            start_byte: symbol.start_byte,
            end_byte: symbol.end_byte,
            start_line: symbol.start_line,
            end_line: symbol.end_line,
            signature: symbol.signature.clone(),
            score: scored.score,
            signals: scored.signals.clone(),
            content,
            content_bytes,
            approximate_tokens: content_bytes.div_ceil(4),
            content_truncated: truncated,
            source_spans,
            relations: serializable_relations(symbol.id, graph, symbols),
        });
    }
    results
}

fn budgeted_content(
    symbol: &Symbol,
    remaining: usize,
    container_limit: usize,
    query: &Query,
) -> (String, bool, Vec<crate::model::SourceSpan>) {
    if symbol.content.len() <= remaining && symbol.content.len() <= container_limit {
        return (
            symbol.content.clone(),
            false,
            vec![crate::model::SourceSpan {
                start_byte: symbol.start_byte,
                end_byte: symbol.end_byte,
                start_line: symbol.start_line,
                end_line: symbol.end_line,
            }],
        );
    }
    if matches!(symbol.kind.as_str(), "function" | "method")
        && let Some((content, spans)) =
            crate::slicing::slice_symbol(symbol, query, remaining.min(container_limit))
    {
        return (content, true, spans);
    }
    if is_container(&symbol.kind) {
        let compact = compact_container(symbol);
        if compact.len() <= remaining.min(container_limit) {
            return (compact, true, Vec::new());
        }
    }
    // Keep small non-containers whole if they fit the overall budget.
    if !is_container(&symbol.kind)
        && !matches!(symbol.kind.as_str(), "function" | "method")
        && symbol.content.len() <= remaining
    {
        return (
            symbol.content.clone(),
            false,
            vec![crate::model::SourceSpan {
                start_byte: symbol.start_byte,
                end_byte: symbol.end_byte,
                start_line: symbol.start_line,
                end_line: symbol.end_line,
            }],
        );
    }
    (String::new(), false, Vec::new())
}

fn compact_container(symbol: &Symbol) -> String {
    let mut content = String::new();
    if !symbol.comments.is_empty() {
        content.push_str(&symbol.comments);
        content.push('\n');
    }
    content.push_str(symbol.signature.trim_end());
    if symbol.language == crate::model::Language::Python {
        content.push_str("\n    # … members omitted by context budget");
    } else {
        content.push_str(" { /* … members omitted by context budget */ }");
    }
    content
}

fn overlaps_selected(symbol: &Symbol, selected: &[SearchResult]) -> bool {
    selected.iter().any(|result| {
        result.path == symbol.path
            && !result.content_truncated
            && ((symbol.start_byte >= result.start_byte && symbol.end_byte <= result.end_byte)
                || (result.start_byte >= symbol.start_byte && result.end_byte <= symbol.end_byte))
    })
}

#[cfg(test)]
mod tests {
    use crate::model::{Language, ScoreSignals};

    use super::*;

    #[test]
    fn skips_whole_oversized_container_in_favor_of_compact_declaration() {
        let symbol = Symbol {
            id: 0,
            path: "large.rs".to_owned(),
            language: Language::Rust,
            name: "Large".to_owned(),
            normalized_name: "large".to_owned(),
            kind: "struct".to_owned(),
            containing_symbol: None,
            structural_depth: 0,
            start_byte: 0,
            end_byte: 10_000,
            start_line: 1,
            end_line: 500,
            signature: "struct Large".to_owned(),
            body: "x".repeat(10_000),
            comments: String::new(),
            content: "x".repeat(10_000),
            imports: Vec::new(),
            identifiers: Vec::new(),
            type_references: Vec::new(),
            calls: Vec::new(),
        };
        let scored = ScoredSymbol {
            symbol_id: 0,
            score: 1.0,
            signals: ScoreSignals::default(),
        };
        let results = select_context(&[scored], &[symbol], &RelationGraph::default(), 1_024, 2);
        assert_eq!(results.len(), 1);
        assert!(results[0].content_truncated);
        assert!(results[0].content.len() < 1_024);
    }

    #[test]
    fn diversifies_repeated_local_declarations() {
        let base = Symbol {
            id: 0,
            path: "one.rs".to_owned(),
            language: Language::Rust,
            name: "auth".to_owned(),
            normalized_name: "auth".to_owned(),
            kind: "declaration".to_owned(),
            containing_symbol: Some("first".to_owned()),
            structural_depth: 1,
            start_byte: 0,
            end_byte: 18,
            start_line: 1,
            end_line: 1,
            signature: "const auth: bool".to_owned(),
            body: String::new(),
            comments: String::new(),
            content: "const auth = true;".to_owned(),
            imports: Vec::new(),
            identifiers: vec!["auth".to_owned()],
            type_references: Vec::new(),
            calls: Vec::new(),
        };
        let mut duplicate = base.clone();
        duplicate.id = 1;
        duplicate.path = "two.rs".to_owned();
        duplicate.containing_symbol = Some("second".to_owned());
        let scored = [
            ScoredSymbol {
                symbol_id: 0,
                score: 10.0,
                signals: ScoreSignals::default(),
            },
            ScoredSymbol {
                symbol_id: 1,
                score: 9.0,
                signals: ScoreSignals::default(),
            },
        ];
        let results = select_context(
            &scored,
            &[base, duplicate],
            &RelationGraph::default(),
            1_024,
            5,
        );
        assert_eq!(results.len(), 1);
    }
}
