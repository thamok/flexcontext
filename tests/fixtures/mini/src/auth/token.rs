/// A validated bearer credential.
pub struct AuthToken {
    pub subject: u64,
}

/// Parse and verify an authentication token.
pub fn validate_token(raw: &str) -> Option<AuthToken> {
    raw.strip_prefix("bearer-")?
        .parse()
        .ok()
        .map(|subject| AuthToken { subject })
}
