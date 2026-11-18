//! Account rate-limit / quota usage for every provider Shepherd drives.
//!
//! * Claude Code — `GET api.anthropic.com/api/oauth/usage` with the CLI's OAuth
//!   login, plus live `rate_limit_event`s from stream-json ([`claude_rate_limit_event`]).
//! * Codex — no public API: the newest `token_count` event in the rollout logs,
//!   plus live events ([`codex_rate_limits`]).
//! * GitHub Copilot — `GET api.github.com/copilot_internal/user` quota snapshots.
//! * OpenCode — no account API; presence only.
//! * Hermes — `hermes usage --json` for its configured upstream provider.
//!
//! Every fetcher is defensive: failures end up in [`ProviderUsage::error`],
//! never in a panic, and secrets are never logged.

mod claude;
mod codex;
mod copilot;
mod local;
mod util;

pub use claude::claude_rate_limit_event;
pub use codex::codex_rate_limits;
pub use shepherd_core::{LimitWindow, Provider, ProviderUsage, WindowKind};

use std::time::Duration;

pub(crate) const USER_AGENT: &str = "shepherd/0.1";

/// Every provider, in display order.
pub const ALL_PROVIDERS: [Provider; 5] = [
    Provider::Anthropic,
    Provider::OpenAi,
    Provider::GitHubCopilot,
    Provider::OpenCode,
    Provider::Hermes,
];

/// Fetches account-level usage for the providers configured on this machine.
#[derive(Debug, Clone)]
pub struct UsageFetcher {
    client: reqwest::Client,
}

impl Default for UsageFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageFetcher {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!("usage http client: {e}; falling back to defaults");
                reqwest::Client::new()
            });
        Self { client }
    }

    /// Use a caller-provided HTTP client (shared pool, proxies …).
    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Current usage for one provider. `None` means the provider isn't
    /// configured on this machine; failures come back as `Some` with `error` set.
    pub async fn fetch(&self, provider: Provider) -> Option<ProviderUsage> {
        let res = match provider {
            Provider::Anthropic => claude::fetch(&self.client).await,
            Provider::OpenAi => codex::fetch().await,
            Provider::GitHubCopilot => copilot::fetch(&self.client).await,
            Provider::OpenCode => local::fetch_opencode().await,
            Provider::Hermes => local::fetch_hermes().await,
        };
        if let Some(u) = &res
            && let Some(e) = &u.error
        {
            tracing::debug!("usage {}: {e}", provider.display_name());
        }
        res
    }

    /// All configured providers, fetched concurrently, in [`ALL_PROVIDERS`] order.
    pub async fn fetch_all(&self) -> Vec<ProviderUsage> {
        let (a, b, c, d, e) = tokio::join!(
            self.fetch(Provider::Anthropic),
            self.fetch(Provider::OpenAi),
            self.fetch(Provider::GitHubCopilot),
            self.fetch(Provider::OpenCode),
            self.fetch(Provider::Hermes),
        );
        [a, b, c, d, e].into_iter().flatten().collect()
    }
}

/// Human-readable reqwest error without URLs or headers.
pub(crate) fn describe_reqwest(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "request timed out".into()
    } else if e.is_connect() {
        "connection failed (offline?)".into()
    } else {
        let s = e.without_url().to_string();
        util::one_line(&s, 160)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetcher_builds() {
        let f = UsageFetcher::default();
        let _ = f.clone();
    }
}
