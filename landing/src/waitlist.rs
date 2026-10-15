









use std::fmt;

use serde::{Deserialize, Serialize};
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        content::Form,
        error::{SeeOther, see_other},
        href, route,
    },
};

use crate::landing::home;

const INSERT_SQL: &str =
    "INSERT INTO waitlist (email) VALUES (?) ON CONFLICT(email) DO NOTHING";


#[derive(Debug, Deserialize)]
pub struct WaitlistForm {
    pub email: String,
    
    #[serde(default)]
    pub website: Option<String>,
}


#[derive(Serialize)]
pub struct JoinedQuery {
    pub joined: String,
}

#[derive(Debug)]
pub enum WishlistError {
    NotConfigured,
    Transport(String),
    Api(String),
}

impl fmt::Display for WishlistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "Cloudflare D1 credentials are not configured"),
            Self::Transport(err) => write!(f, "transport error: {err}"),
            Self::Api(err) => write!(f, "api error: {err}"),
        }
    }
}

impl std::error::Error for WishlistError {}

impl From<reqwest::Error> for WishlistError {
    fn from(err: reqwest::Error) -> Self {
        Self::Transport(err.to_string())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum JoinOutcome {
    
    Joined,
    
    AlreadyJoined,
}


pub struct WishlistStore {
    client: reqwest::Client,
    api_base: String,
    account_id: String,
    database_id: String,
    api_token: String,
}

impl WishlistStore {
    pub fn from_env() -> Self {
        Self {
            client: reqwest::Client::new(),
            api_base: std::env::var("CLOUDFLARE_API_BASE")
                .unwrap_or_else(|_| "https://api.cloudflare.com/client/v4".to_owned()),
            account_id: std::env::var("CLOUDFLARE_ACCOUNT_ID").unwrap_or_default(),
            database_id: std::env::var("CLOUDFLARE_D1_DATABASE_ID").unwrap_or_default(),
            api_token: std::env::var("CLOUDFLARE_API_TOKEN").unwrap_or_default(),
        }
    }

    fn is_configured(&self) -> bool {
        !self.account_id.is_empty()
            && !self.database_id.is_empty()
            && !self.api_token.is_empty()
    }

    
    pub async fn add_email(&self, email: &str) -> Result<JoinOutcome, WishlistError> {
        if !self.is_configured() {
            eprintln!(
                "wishlist: refusing to write; set CLOUDFLARE_ACCOUNT_ID, \
                 CLOUDFLARE_D1_DATABASE_ID and CLOUDFLARE_API_TOKEN"
            );
            return Err(WishlistError::NotConfigured);
        }

        let url = format!(
            "{}/accounts/{}/d1/database/{}/query",
            self.api_base, self.account_id, self.database_id
        );
        let response: D1QueryResponse = self
            .client
            .post(&url)
            .bearer_auth(&self.api_token)
            .json(&serde_json::json!({
                "sql": INSERT_SQL,
                "params": [email],
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        if !response.success {
            return Err(WishlistError::Api(
                response
                    .errors
                    .into_iter()
                    .map(|err| err.message)
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }

        let changes = response
            .result
            .first()
            .and_then(|result| result.meta.changes)
            .unwrap_or_default();

        Ok(if changes == 0 {
            JoinOutcome::AlreadyJoined
        } else {
            JoinOutcome::Joined
        })
    }
}

#[derive(Deserialize)]
struct D1QueryResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    errors: Vec<D1Error>,
    #[serde(default)]
    result: Vec<D1QueryResult>,
}

#[derive(Deserialize)]
struct D1Error {
    message: String,
}

#[derive(Deserialize)]
struct D1QueryResult {
    #[serde(default)]
    meta: D1Meta,
}

#[derive(Default, Deserialize)]
struct D1Meta {
    #[serde(default)]
    changes: Option<i64>,
}


pub fn is_valid_email(email: &str) -> bool {
    let email = email.trim();
    if !(5..=254).contains(&email.len()) {
        return false;
    }
    let mut parts = email.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if local.is_empty()
        || !local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c))
    {
        return false;
    }
    if domain.len() > 253
        || !domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
    {
        return false;
    }
    let mut labels = domain.split('.');
    let tld = labels.next_back().unwrap_or_default();
    labels.all(|label| !label.is_empty())
        && !tld.is_empty()
        && domain.split('.').count() >= 2
        && tld.len() >= 2
        && tld.chars().all(|c| c.is_ascii_alphabetic())
}



#[route(POST "/waitlist")]
pub async fn join(cx: &Cx, Form(form): Form<WaitlistForm>) -> Result<SeeOther> {
    let store = app_context::<WishlistStore>(cx);
    let email = form.email.trim().to_ascii_lowercase();
    let honeypot = form.website.as_deref().unwrap_or_default();

    let joined = if !honeypot.trim().is_empty() {
        
        "joined"
    } else if !is_valid_email(&email) {
        "invalid"
    } else {
        match store.add_email(&email).await {
            Ok(JoinOutcome::Joined) => "joined",
            Ok(JoinOutcome::AlreadyJoined) => "duplicate",
            Err(err) => {
                eprintln!("wishlist: could not record {email}: {err}");
                "error"
            }
        }
    };

    Ok(see_other(
        href!(home)
            .query(JoinedQuery {
                joined: joined.into(),
            })
            .resolve(cx),
    ))
}

#[cfg(test)]
mod tests {
    use super::is_valid_email;

    #[test]
    fn accepts_normal_addresses() {
        assert!(is_valid_email("alice@example.com"));
        assert!(is_valid_email("alice.bob+tag@sub.example.co.uk"));
        assert!(is_valid_email(" a@b.io "));
    }

    #[test]
    fn rejects_broken_addresses() {
        assert!(!is_valid_email(""));
        assert!(!is_valid_email("alice"));
        assert!(!is_valid_email("alice@"));
        assert!(!is_valid_email("@example.com"));
        assert!(!is_valid_email("alice@@example.com"));
        assert!(!is_valid_email("alice example@test.com"));
        assert!(!is_valid_email("alice@example"));
        assert!(!is_valid_email("alice@example.c"));
        assert!(!is_valid_email("alice@.com"));
        assert!(!is_valid_email("alice@example..com"));
        assert!(!is_valid_email(&format!("{}@example.com", "a".repeat(250))));
    }
}
