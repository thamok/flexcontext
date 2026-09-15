use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::lexical::{Query, identifier_tokens, light_stem};
use crate::model::Symbol;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LexicalIndex {
    postings: BTreeMap<String, Vec<usize>>,
}

impl LexicalIndex {
    pub fn build(symbols: &[Symbol]) -> Self {
        let mut postings: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for symbol in symbols {
            let mut keys = BTreeSet::new();
            keys.insert(format!("n:{}", symbol.normalized_name));
            for text in [
                symbol.name.as_str(),
                symbol.containing_symbol.as_deref().unwrap_or(""),
                &symbol.path,
                &symbol.comments,
                &symbol.signature,
                &symbol.body,
            ] {
                add_text_keys(&mut keys, text);
            }
            for identifier in &symbol.identifiers {
                add_text_keys(&mut keys, identifier);
            }
            for key in keys {
                postings.entry(key).or_default().push(symbol.id);
            }
        }
        Self { postings }
    }

    pub fn candidates(&self, query: &Query) -> Vec<usize> {
        let mut candidates = BTreeSet::new();
        if let Some(ids) = self.postings.get(&format!("n:{}", query.normalized)) {
            candidates.extend(ids);
        }
        for token in &query.tokens {
            for key in token_keys(token) {
                if let Some(ids) = self.postings.get(&key) {
                    candidates.extend(ids);
                }
            }
        }
        candidates.into_iter().collect()
    }

    pub fn posting_count(&self) -> usize {
        self.postings.len()
    }
}

fn add_text_keys(keys: &mut BTreeSet<String>, text: &str) {
    for token in identifier_tokens(text) {
        keys.extend(token_keys(&token));
    }
}

pub(crate) fn token_keys(token: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::from([format!("t:{token}"), format!("s:{}", light_stem(token))]);
    let prefix: String = token.chars().take(4).collect();
    if prefix.chars().count() == 4 {
        keys.insert(format!("p:{prefix}"));
    }
    keys
}

#[cfg(test)]
mod tests {
    use crate::model::Language;

    use super::*;

    #[test]
    fn retrieves_morphological_candidates_without_scanning() {
        let symbol = Symbol {
            id: 0,
            path: "auth.rs".to_owned(),
            language: Language::Rust,
            name: "authenticate_user".to_owned(),
            normalized_name: "authenticateuser".to_owned(),
            kind: "function".to_owned(),
            containing_symbol: None,
            structural_depth: 0,
            start_byte: 0,
            end_byte: 2,
            start_line: 1,
            end_line: 1,
            signature: "fn authenticate_user()".to_owned(),
            body: "{}".to_owned(),
            comments: String::new(),
            content: "{}".to_owned(),
            imports: Vec::new(),
            identifiers: Vec::new(),
            type_references: Vec::new(),
            calls: Vec::new(),
        };
        let index = LexicalIndex::build(&[symbol]);
        assert_eq!(index.candidates(&Query::parse("authentication")), [0]);
    }
}
