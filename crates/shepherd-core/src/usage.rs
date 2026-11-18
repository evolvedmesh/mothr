//! Rate-limit / quota / context usage model shared by every provider.

use crate::agent::Provider;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What kind of limit a window represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    /// Rolling 5-hour session window.
    FiveHour,
    /// Rolling weekly window.
    Weekly,
    /// Weekly window scoped to a premium model family (e.g. Opus).
    WeeklyModel,
    /// Monthly credit quota, e.g. Copilot AI credits (AIC).
    MonthlyCredits,
    /// Spend / budget limit in currency.
    Spend,
    Other,
}

/// One limit window with how much of it has been consumed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitWindow {
    pub kind: WindowKind,
    /// Short label: "5h", "Weekly", "Opus weekly", "AIC".
    pub label: String,
    /// 0.0 ..= 1.0 (may exceed 1.0 when in overage).
    pub used_fraction: f32,
    pub resets_at: Option<DateTime<Utc>>,
    /// Absolute numbers when known: (used, limit) e.g. AIC 212 / 300.
    pub used: Option<f64>,
    pub limit: Option<f64>,
    /// Extra free-form detail ("unlimited", "overage enabled" …).
    pub detail: Option<String>,
}

impl LimitWindow {
    pub fn level(&self) -> UsageLevel {
        UsageLevel::from_fraction(self.used_fraction)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageLevel {
    Ok,
    Warn,
    Critical,
    Exhausted,
}

impl UsageLevel {
    pub fn from_fraction(f: f32) -> Self {
        if f >= 1.0 {
            UsageLevel::Exhausted
        } else if f >= 0.9 {
            UsageLevel::Critical
        } else if f >= 0.75 {
            UsageLevel::Warn
        } else {
            UsageLevel::Ok
        }
    }
}

/// Snapshot of a provider's account limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub provider: Provider,
    /// Plan name if known ("Max 20x", "Pro", "Copilot Business").
    pub plan: Option<String>,
    pub windows: Vec<LimitWindow>,
    pub updated_at: DateTime<Utc>,
    /// Set when fetching failed or the provider has no limit API.
    pub error: Option<String>,
    /// Where the data came from ("oauth api", "session stream", "local logs").
    pub source: String,
}

impl ProviderUsage {
    pub fn new(provider: Provider, source: impl Into<String>) -> Self {
        Self {
            provider,
            plan: None,
            windows: Vec::new(),
            updated_at: Utc::now(),
            error: None,
            source: source.into(),
        }
    }

    pub fn worst_level(&self) -> UsageLevel {
        self.windows
            .iter()
            .map(|w| w.level())
            .max()
            .unwrap_or(UsageLevel::Ok)
    }

    /// Merge another snapshot of the same provider, keeping the newest window of each kind.
    pub fn merge(&mut self, other: ProviderUsage) {
        if other.plan.is_some() {
            self.plan = other.plan;
        }
        for w in other.windows {
            if let Some(existing) = self
                .windows
                .iter_mut()
                .find(|e| e.kind == w.kind && e.label == w.label)
            {
                *existing = w;
            } else {
                self.windows.push(w);
            }
        }
        self.updated_at = other.updated_at.max(self.updated_at);
        if other.error.is_some() || !self.windows.is_empty() {
            self.error = other.error;
        }
    }
}

/// Context-window usage for a single session.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ContextUsage {
    /// Tokens currently occupying the context window.
    pub used_tokens: u64,
    /// Context window size in tokens (0 = unknown).
    pub window_tokens: u64,
    /// Cumulative cost of the session in USD, when reported.
    pub cost_usd: Option<f64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl ContextUsage {
    pub fn fraction(&self) -> f32 {
        if self.window_tokens == 0 {
            0.0
        } else {
            (self.used_tokens as f64 / self.window_tokens as f64) as f32
        }
    }
}

/// Format a token count compactly: 12.3k, 1.2M.
pub fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// "in 2h 14m", "in 3d 4h", "now".
pub fn format_reset(at: DateTime<Utc>) -> String {
    let secs = (at - Utc::now()).num_seconds();
    if secs <= 0 {
        return "now".into();
    }
    let (d, h, m) = (secs / 86_400, (secs % 86_400) / 3600, (secs % 3600) / 60);
    if d > 0 {
        format!("in {d}d {h}h")
    } else if h > 0 {
        format!("in {h}h {m}m")
    } else {
        format!("in {}m", m.max(1))
    }
}
