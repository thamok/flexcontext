use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub root: PathBuf,
    pub query: String,
    pub max_bytes: usize,
    pub max_results: usize,
    pub use_cache: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            query: String::new(),
            max_bytes: 16 * 1024,
            max_results: 12,
            use_cache: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
}

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub absolute_path: PathBuf,
    pub relative_path: String,
    pub language: Language,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: usize,
    pub path: String,
    pub language: Language,
    pub name: String,
    pub normalized_name: String,
    pub kind: String,
    pub containing_symbol: Option<String>,
    pub structural_depth: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub signature: String,
    pub body: String,
    pub comments: String,
    pub content: String,
    pub imports: Vec<String>,
    pub identifiers: Vec<String>,
    pub type_references: Vec<String>,
    pub calls: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScoreSignals {
    pub exact_symbol_name: f64,
    pub normalized_symbol_name: f64,
    pub prefix_suffix: f64,
    pub symbol_name_tokens: f64,
    pub containing_symbol: f64,
    pub path: f64,
    pub comments: f64,
    pub identifiers: f64,
    pub signature: f64,
    pub body: f64,
    pub query_coverage: f64,
    pub match_density: f64,
    pub structural_priority: f64,
    pub structural_relation: f64,
    pub size_penalty: f64,
}

impl ScoreSignals {
    pub fn total(&self) -> f64 {
        self.exact_symbol_name
            + self.normalized_symbol_name
            + self.prefix_suffix
            + self.symbol_name_tokens
            + self.containing_symbol
            + self.path
            + self.comments
            + self.identifiers
            + self.signature
            + self.body
            + self.query_coverage
            + self.match_density
            + self.structural_priority
            + self.structural_relation
            + self.size_penalty
    }
}

#[derive(Debug, Clone)]
pub struct ScoredSymbol {
    pub symbol_id: usize,
    pub score: f64,
    pub signals: ScoreSignals,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub kind: String,
    pub symbol: String,
    pub path: String,
    pub start_line: usize,
}

/// Exact original-file ranges included as source excerpts (exclusive end byte).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub path: String,
    pub language: Language,
    pub symbol: String,
    pub kind: String,
    pub containing_symbol: Option<String>,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub signature: String,
    pub score: f64,
    pub signals: ScoreSignals,
    pub content: String,
    pub content_bytes: usize,
    pub approximate_tokens: usize,
    pub content_truncated: bool,
    #[serde(default)]
    pub source_spans: Vec<SourceSpan>,
    pub relations: Vec<Relation>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchStats {
    pub files_scanned: usize,
    pub files_parsed: usize,
    pub files_indexed: usize,
    pub source_bytes: usize,
    pub symbols: usize,
    pub candidate_symbols: usize,
    pub returned_symbols: usize,
    pub returned_bytes: usize,
    pub human_payload_bytes: usize,
    pub json_payload_bytes: usize,
    pub approximate_tokens: usize,
    pub traversal_us: u128,
    pub cache_load_us: u128,
    pub parse_and_extract_us: u128,
    pub index_us: u128,
    pub candidate_and_ranking_us: u128,
    pub relationship_us: u128,
    pub selection_us: u128,
    pub cache_write_us: u128,
    pub elapsed_us: u128,
    pub files_reused: usize,
    pub files_reparsed: usize,
    pub index_reused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub root: String,
    pub results: Vec<SearchResult>,
    pub stats: SearchStats,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub metadata: BTreeMap<String, String>,
}
