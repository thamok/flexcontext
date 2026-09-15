use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

use crate::index::LexicalIndex;
use crate::lexical::{Query, identifier_tokens, lexically_related};
use crate::model::{ScoreSignals, ScoredSymbol, Symbol};

pub const WEIGHT_EXACT_SYMBOL_NAME: f64 = 12.0;
pub const WEIGHT_NORMALIZED_SYMBOL_NAME: f64 = 10.0;
pub const WEIGHT_PREFIX_SUFFIX: f64 = 3.0;
pub const WEIGHT_SYMBOL_NAME_TOKENS: f64 = 6.0;
pub const WEIGHT_CONTAINING_SYMBOL: f64 = 2.5;
pub const WEIGHT_PATH: f64 = 2.5;
pub const WEIGHT_COMMENTS: f64 = 2.0;
pub const WEIGHT_IDENTIFIERS: f64 = 2.5;
pub const WEIGHT_SIGNATURE: f64 = 2.0;
pub const WEIGHT_BODY: f64 = 1.2;
pub const WEIGHT_QUERY_COVERAGE: f64 = 3.0;
pub const WEIGHT_MATCH_DENSITY: f64 = 1.5;
pub const WEIGHT_TOP_LEVEL_STRUCTURE: f64 = 2.0;

pub fn rank_candidates(symbols: &[Symbol], query: &Query) -> Vec<ScoredSymbol> {
    let mut scored: Vec<_> = symbols
        .iter()
        .filter_map(|symbol| score_symbol(symbol, query))
        .collect();
    sort_scored(&mut scored, symbols);
    scored
}

pub fn rank_indexed_candidates(
    symbols: &[Symbol],
    query: &Query,
    index: &LexicalIndex,
) -> Vec<ScoredSymbol> {
    let candidate_ids = index.candidates(query);
    let broad_query = candidate_ids.len() >= 8;
    let mut scored: Vec<_> = candidate_ids
        .into_iter()
        .filter_map(|id| symbols.get(id))
        .filter_map(|symbol| {
            let mut scored = score_symbol(symbol, query)?;
            if broad_query && symbol.kind == "declaration" && symbol.structural_depth > 0 {
                scored.signals.structural_priority -= 32.0;
                scored.score = scored.signals.total();
            }
            (scored.score > 0.0).then_some(scored)
        })
        .collect();
    sort_scored(&mut scored, symbols);
    scored
}

/// Query-independent lexical features, built once per resident snapshot.
#[derive(Debug)]
pub struct PreparedSymbol {
    fields: Vec<(Vec<String>, String)>,
    all_tokens: Vec<String>,
}

impl PreparedSymbol {
    pub fn new(symbol: &Symbol) -> Self {
        let identifiers = symbol.identifiers.join(" ");
        let fields: Vec<_> = [
            symbol.name.as_str(),
            symbol.containing_symbol.as_deref().unwrap_or(""),
            &symbol.path,
            &symbol.comments,
            &identifiers,
            &symbol.signature,
            &symbol.body,
        ]
        .into_iter()
        .map(|text| {
            let tokens = identifier_tokens(text);
            let normalized = tokens.join("");
            (
                tokens
                    .into_iter()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>(),
                normalized,
            )
        })
        .collect();
        let all_tokens = fields
            .iter()
            .flat_map(|(tokens, _)| tokens.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Self { fields, all_tokens }
    }
}

/// Field postings map lexical terms to symbol/field pairs. Query masks avoid
/// rescanning candidate token lists while preserving the scalar score exactly.
#[derive(Debug)]
pub struct PreparedIndex {
    symbols: Vec<PreparedSymbol>,
    postings: HashMap<String, Vec<(usize, usize)>>,
    vocabulary: HashMap<String, Vec<String>>,
}
impl PreparedIndex {
    pub fn build(symbols: &[Symbol]) -> Self {
        let prepared: Vec<_> = symbols.iter().map(PreparedSymbol::new).collect();
        let mut postings: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        for (id, symbol) in prepared.iter().enumerate() {
            for (field, (tokens, _)) in symbol.fields.iter().enumerate() {
                for token in tokens {
                    postings.entry(token.clone()).or_default().push((id, field));
                }
            }
        }
        let mut vocabulary: HashMap<String, Vec<String>> = HashMap::new();
        for token in postings.keys() {
            for key in crate::index::token_keys(token) {
                vocabulary.entry(key).or_default().push(token.clone());
            }
        }
        Self {
            symbols: prepared,
            postings,
            vocabulary,
        }
    }
    fn matches(&self, query: &Query) -> Option<Vec<[u64; 7]>> {
        if query.tokens.len() > 64 {
            return None;
        }
        let mut matches = vec![[0; 7]; self.symbols.len()];
        for (term_index, term) in query.tokens.iter().enumerate() {
            let mut words = BTreeSet::new();
            for key in crate::index::token_keys(term) {
                for word in self.vocabulary.get(&key).into_iter().flatten() {
                    if lexically_related(term, word) {
                        words.insert(word);
                    }
                }
            }
            for word in words {
                for &(id, field) in &self.postings[word] {
                    matches[id][field] |= 1 << term_index;
                }
            }
        }
        Some(matches)
    }
}

pub fn rank_prepared_candidates(
    symbols: &[Symbol],
    query: &Query,
    index: &LexicalIndex,
    prepared: &PreparedIndex,
) -> Vec<ScoredSymbol> {
    let ids = index.candidates(query);
    let broad = ids.len() >= 8;
    let masks = prepared.matches(query);
    let mut scored: Vec<_> = ids
        .into_iter()
        .filter_map(|id| {
            let symbol = &symbols[id];
            let counts = masks.as_ref().map(|masks| {
                let fields = masks[id];
                let mut counts = [0; 8];
                for i in 0..7 {
                    counts[i] = fields[i].count_ones() as usize;
                }
                counts[7] = fields.iter().fold(0, |all, field| all | field).count_ones() as usize;
                counts
            });
            let mut item = score_prepared(symbol, query, &prepared.symbols[id], counts)?;
            if broad && symbol.kind == "declaration" && symbol.structural_depth > 0 {
                item.signals.structural_priority -= 32.0;
                item.score = item.signals.total();
            }
            (item.score > 0.0).then_some(item)
        })
        .collect();
    sort_scored(&mut scored, symbols);
    scored
}

pub fn score_symbol(symbol: &Symbol, query: &Query) -> Option<ScoredSymbol> {
    score_prepared(symbol, query, &PreparedSymbol::new(symbol), None)
}

fn score_prepared(
    symbol: &Symbol,
    query: &Query,
    prepared: &PreparedSymbol,
    counts: Option<[usize; 8]>,
) -> Option<ScoredSymbol> {
    if query.is_empty() {
        return None;
    }
    let name_lower = symbol.name.to_lowercase();
    let count = |field: usize| {
        counts.map_or_else(
            || {
                if field == 7 {
                    matched_tokens(query, &prepared.all_tokens)
                } else {
                    matched_tokens(query, &prepared.fields[field].0)
                }
            },
            |counts| counts[field],
        )
    };
    let matched_name_tokens = count(0);
    let term_ratio = |matched: usize| matched as f64 / query.tokens.len() as f64;

    let exact_symbol_name = if name_lower == query.lowercase {
        WEIGHT_EXACT_SYMBOL_NAME
    } else {
        0.0
    };
    let normalized_symbol_name = if symbol.normalized_name == query.normalized {
        WEIGHT_NORMALIZED_SYMBOL_NAME
    } else {
        0.0
    };
    let prefix_suffix = if !query.normalized.is_empty()
        && (symbol.normalized_name.starts_with(&query.normalized)
            || symbol.normalized_name.ends_with(&query.normalized)
            || query.normalized.starts_with(&symbol.normalized_name)
            || query.normalized.ends_with(&symbol.normalized_name))
    {
        WEIGHT_PREFIX_SUFFIX
    } else {
        0.0
    };
    let symbol_name_tokens = WEIGHT_SYMBOL_NAME_TOKENS * term_ratio(matched_name_tokens);
    let containing_symbol = prepared_field_score(
        query,
        &prepared.fields[1],
        WEIGHT_CONTAINING_SYMBOL,
        count(1),
    );
    let path = prepared_field_score(query, &prepared.fields[2], WEIGHT_PATH, count(2));
    let comments = prepared_field_score(query, &prepared.fields[3], WEIGHT_COMMENTS, count(3));
    let identifiers_score =
        prepared_field_score(query, &prepared.fields[4], WEIGHT_IDENTIFIERS, count(4));
    let signature = prepared_field_score(query, &prepared.fields[5], WEIGHT_SIGNATURE, count(5));
    let body = prepared_field_score(query, &prepared.fields[6], WEIGHT_BODY, count(6));

    let matched_terms = count(7);
    let query_coverage = WEIGHT_QUERY_COVERAGE * term_ratio(matched_terms);
    let token_count = prepared.all_tokens.len().max(1);
    let match_density = if matched_terms == 0 {
        0.0
    } else {
        WEIGHT_MATCH_DENSITY * (matched_terms as f64 / token_count as f64).sqrt().min(1.0)
    };
    let lexical_total = exact_symbol_name
        + normalized_symbol_name
        + prefix_suffix
        + symbol_name_tokens
        + containing_symbol
        + path
        + comments
        + identifiers_score
        + signature
        + body
        + query_coverage
        + match_density;
    if lexical_total <= 0.0 {
        return None;
    }
    let size_penalty = if symbol.content.len() > 2_048 {
        -((symbol.content.len() as f64 / 2_048.0).log2() * 0.35).min(3.0)
    } else {
        0.0
    };
    let structural_priority = structural_priority(symbol);
    let signals = ScoreSignals {
        exact_symbol_name,
        normalized_symbol_name,
        prefix_suffix,
        symbol_name_tokens,
        containing_symbol,
        path,
        comments,
        identifiers: identifiers_score,
        signature,
        body,
        query_coverage,
        match_density,
        structural_priority,
        structural_relation: 0.0,
        size_penalty,
    };
    Some(ScoredSymbol {
        symbol_id: symbol.id,
        score: signals.total(),
        signals,
    })
}

fn structural_priority(symbol: &Symbol) -> f64 {
    match symbol.kind.as_str() {
        "function" | "class" | "struct" | "enum" | "trait" | "interface" | "module"
            if symbol.structural_depth <= 1 =>
        {
            WEIGHT_TOP_LEVEL_STRUCTURE
        }
        "type" if symbol.structural_depth <= 1 => WEIGHT_TOP_LEVEL_STRUCTURE,
        "method" => 0.75,
        "declaration" if symbol.structural_depth == 0 => 0.5,
        "declaration" => -2.0,
        "import" => -0.5,
        _ => 0.0,
    }
}

pub fn sort_scored(scored: &mut [ScoredSymbol], symbols: &[Symbol]) {
    scored.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                symbols[left.symbol_id]
                    .path
                    .cmp(&symbols[right.symbol_id].path)
            })
            .then_with(|| {
                symbols[left.symbol_id]
                    .start_line
                    .cmp(&symbols[right.symbol_id].start_line)
            })
            .then_with(|| {
                symbols[left.symbol_id]
                    .name
                    .cmp(&symbols[right.symbol_id].name)
            })
    });
}

fn matched_tokens(query: &Query, tokens: &[String]) -> usize {
    query
        .tokens
        .iter()
        .filter(|term| tokens.iter().any(|token| lexically_related(term, token)))
        .count()
}

fn prepared_field_score(
    query: &Query,
    field: &(Vec<String>, String),
    weight: f64,
    matched: usize,
) -> f64 {
    let bonus = if !query.normalized.is_empty() && field.1.contains(&query.normalized) {
        weight * 0.25
    } else {
        0.0
    };
    weight * matched as f64 / query.tokens.len() as f64 + bonus
}

#[cfg(test)]
mod tests {
    use crate::model::Language;

    use super::*;

    fn symbol(name: &str, body: &str) -> Symbol {
        Symbol {
            id: 0,
            path: "src/auth/session.rs".to_owned(),
            language: Language::Rust,
            name: name.to_owned(),
            normalized_name: crate::lexical::normalize_identifier(name),
            kind: "function".to_owned(),
            containing_symbol: None,
            structural_depth: 0,
            start_byte: 0,
            end_byte: body.len(),
            start_line: 1,
            end_line: 1,
            signature: format!("fn {name}()"),
            body: body.to_owned(),
            comments: String::new(),
            content: body.to_owned(),
            imports: Vec::new(),
            identifiers: identifier_tokens(body),
            type_references: Vec::new(),
            calls: Vec::new(),
        }
    }

    #[test]
    fn name_match_is_stronger_than_body_match() {
        let query = Query::parse("auth");
        let named = score_symbol(&symbol("auth", "{}"), &query).unwrap();
        let body = score_symbol(&symbol("helper", "auth();"), &query).unwrap();
        assert!(named.score > body.score);
    }

    #[test]
    fn identifier_variants_match_multi_token_query() {
        let query = Query::parse("validate token");
        let result = score_symbol(&symbol("validateToken", "{}"), &query).unwrap();
        assert!(result.signals.normalized_symbol_name > 0.0);
        assert_eq!(result.signals.symbol_name_tokens, WEIGHT_SYMBOL_NAME_TOKENS);
    }

    #[test]
    fn top_level_function_beats_local_declaration_for_broad_query() {
        let query = Query::parse("auth");
        let mut function = symbol("auth_handler", "{}");
        function.id = 0;
        let mut local = symbol("auth", "const auth = true;");
        local.id = 1;
        local.kind = "declaration".to_owned();
        local.structural_depth = 1;
        let mut symbols = vec![function, local];
        for id in 2..8 {
            let mut declaration = symbol(&format!("auth_value_{id}"), "const value = true;");
            declaration.id = id;
            declaration.kind = "declaration".to_owned();
            declaration.structural_depth = 1;
            symbols.push(declaration);
        }
        let index = LexicalIndex::build(&symbols);
        let ranked = rank_indexed_candidates(&symbols, &query, &index);
        assert_eq!(symbols[ranked[0].symbol_id].kind, "function");
    }
}
