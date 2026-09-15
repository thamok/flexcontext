use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct Query {
    pub original: String,
    pub lowercase: String,
    pub normalized: String,
    pub tokens: Vec<String>,
}

impl Query {
    pub fn parse(input: &str) -> Self {
        let tokens = identifier_tokens(input);
        Self {
            original: input.to_owned(),
            lowercase: input.to_lowercase(),
            normalized: tokens.join(""),
            tokens,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

pub fn normalize_identifier(input: &str) -> String {
    identifier_tokens(input).join("")
}

pub fn identifier_tokens(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();

    for (index, &ch) in chars.iter().enumerate() {
        if !ch.is_alphanumeric() {
            push_word(&mut words, &mut current);
            continue;
        }
        let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(index + 1).copied();
        let boundary = !current.is_empty()
            && (previous.is_some_and(|prev| prev.is_lowercase() && ch.is_uppercase())
                || previous.is_some_and(|prev| prev.is_alphabetic() != ch.is_alphabetic())
                || (previous.is_some_and(char::is_uppercase)
                    && ch.is_uppercase()
                    && next.is_some_and(char::is_lowercase)));
        if boundary {
            push_word(&mut words, &mut current);
        }
        current.extend(ch.to_lowercase());
    }
    push_word(&mut words, &mut current);
    words
}

fn push_word(words: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        words.push(std::mem::take(current));
    }
}

pub fn text_tokens(input: &str) -> BTreeSet<String> {
    identifier_tokens(input).into_iter().collect()
}

pub fn matching_query_terms(query: &Query, text: &str) -> usize {
    let text_tokens = text_tokens(text);
    query
        .tokens
        .iter()
        .filter(|term| {
            text_tokens
                .iter()
                .any(|text_token| lexically_related(term, text_token))
        })
        .count()
}

pub fn lexically_related(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let left_stem = light_stem(left);
    let right_stem = light_stem(right);
    (left_stem.len() >= 4 && left_stem == right_stem)
        || (left.len() >= 4 && right.starts_with(left))
        || (right.len() >= 4 && left.starts_with(right))
}

pub fn light_stem(token: &str) -> &str {
    for suffix in ["ations", "ation", "ating", "ated", "ates", "ate"] {
        if let Some(stem) = token.strip_suffix(suffix)
            && stem.len() >= 4
        {
            return stem;
        }
    }
    token
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_identifier_styles() {
        assert_eq!(
            identifier_tokens("authenticateUser"),
            ["authenticate", "user"]
        );
        assert_eq!(identifier_tokens("AuthToken"), ["auth", "token"]);
        assert_eq!(identifier_tokens("HTTPServer2"), ["http", "server", "2"]);
        assert_eq!(
            identifier_tokens("src/auth-token.rs"),
            ["src", "auth", "token", "rs"]
        );
    }

    #[test]
    fn relates_common_identifier_morphology() {
        assert!(lexically_related("authentication", "authenticate"));
        assert!(lexically_related("validation", "validate"));
        assert!(lexically_related("auth", "authentication"));
        assert!(!lexically_related("user", "usage"));
    }
}
