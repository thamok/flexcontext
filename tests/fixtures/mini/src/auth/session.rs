use super::token::{AuthToken, validate_token};

/// An authenticated user's active session.
pub struct UserSession {
    pub user_id: u64,
    pub token: AuthToken,
}

impl UserSession {
    /// Authenticate a user-supplied token and establish a session.
    pub fn authenticate_user(raw_token: &str) -> Option<Self> {
        let token = validate_token(raw_token)?;
        Some(Self { user_id: token.subject, token })
    }
}

pub fn session_display_name(session: &UserSession) -> String {
    format!("user-{}", session.user_id)
}
