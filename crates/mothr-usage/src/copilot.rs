//! GitHub Copilot: `copilot_internal/user` quota snapshots (premium requests / AI credits).

use crate::util::{boolean, field, find_program, home, num, one_line, parse_time, run_command, sane_fraction, string, title_case};
use serde_json::Value;
use mothr_core::{LimitWindow, Provider, ProviderUsage, WindowKind};
use std::path::PathBuf;
use std::time::Duration;

const USER_URL: &str = "https://api.github.com/copilot_internal/user";
pub(crate) const SOURCE: &str = "github api";

/// Where a candidate token came from (for messages; never the token itself).
#[derive(Debug, Clone)]
struct Candidate {
    origin: String,
    token: String,
}

fn push(cands: &mut Vec<Candidate>, origin: impl Into<String>, token: &str) {
    let token = token.trim();
    if !token.is_empty() && !cands.iter().any(|c| c.token == token) {
        cands.push(Candidate { origin: origin.into(), token: token.to_owned() });
    }
}

/// Pull `oauth_token`s out of Copilot's `apps.json` / `hosts.json`, preferring github.com.
pub(crate) fn tokens_from_copilot_config(v: &Value) -> Vec<String> {
    let Some(obj) = v.as_object() else {
        return Vec::new();
    };
    let mut entries: Vec<(&String, &Value)> = obj.iter().collect();
    entries.sort_by_key(|(k, _)| !k.starts_with("github.com"));
    entries
        .into_iter()
        .filter_map(|(_, e)| string(e.get("oauth_token")).map(str::to_owned))
        .collect()
}

fn copilot_config_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|x| !x.is_empty()) {
        v.push(PathBuf::from(x).join("github-copilot"));
    }
    if let Some(h) = home() {
        v.push(h.join(".config").join("github-copilot"));
    }
    if let Some(c) = dirs::config_dir() {
        v.push(c.join("github-copilot"));
    }
    #[cfg(windows)]
    if let Some(l) = dirs::data_local_dir() {
        v.push(l.join("github-copilot"));
    }
    v.dedup();
    v
}

async fn candidates() -> Vec<Candidate> {
    let mut c = Vec::new();
    for var in ["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(t) = std::env::var(var) {
            push(&mut c, format!("${var}"), &t);
        }
    }
    let gh_extra = [PathBuf::from("/usr/bin/gh"), PathBuf::from("/usr/local/bin/gh"), PathBuf::from("/opt/homebrew/bin/gh")];
    if let Some(gh) = find_program("gh", &gh_extra)
        && let Some(out) = run_command(&gh.to_string_lossy(), &["auth", "token"], Duration::from_secs(5)).await
        && out.success
    {
        push(&mut c, "gh auth token", out.stdout.lines().next().unwrap_or(""));
    }
    for dir in copilot_config_dirs() {
        for file in ["apps.json", "hosts.json"] {
            let path = dir.join(file);
            let Ok(text) = tokio::fs::read_to_string(&path).await else {
                continue;
            };
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                for t in tokens_from_copilot_config(&v) {
                    push(&mut c, path.display().to_string(), &t);
                }
            }
        }
    }
    c
}

/// "enterprise" → "Copilot Enterprise", "individual_pro" → "Copilot Pro".
pub(crate) fn plan_name(v: &Value) -> Option<String> {
    let raw = string(field(v, &["copilot_plan", "plan"]))?;
    let name = match raw.to_ascii_lowercase().as_str() {
        "individual" => "Pro".to_owned(),
        "individual_pro" | "pro_plus" | "pro+" => "Pro+".to_owned(),
        "free" | "free_limited_copilot" | "limited" => "Free".to_owned(),
        "business" => "Business".to_owned(),
        "enterprise" => "Enterprise".to_owned(),
        other => title_case(other),
    };
    Some(format!("Copilot {name}"))
}

fn is_credit_quota(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("premium") || n.contains("credit") || n == "aic" || n.starts_with("ai_")
}

/// Map `copilot_internal/user` to a usage snapshot.
pub(crate) fn parse_user(v: &Value) -> ProviderUsage {
    let mut usage = ProviderUsage::new(Provider::GitHubCopilot, SOURCE);
    usage.plan = plan_name(v);
    let global_reset = parse_time(field(v, &["quota_reset_date_utc", "quota_reset_date"]));

    let Some(snaps) = v.get("quota_snapshots").and_then(Value::as_object) else {
        // Older/limited responses: `limited_user_quotas` + `monthly_quotas`.
        if let (Some(rem), Some(tot)) = (v.get("limited_user_quotas"), v.get("monthly_quotas")) {
            for key in ["chat", "completions"] {
                let (Some(r), Some(t)) = (num(rem.get(key)), num(tot.get(key))) else {
                    continue;
                };
                if t > 0.0 {
                    usage.windows.push(LimitWindow {
                        kind: WindowKind::MonthlyCredits,
                        label: title_case(key),
                        used_fraction: sane_fraction((t - r) / t),
                        resets_at: parse_time(v.get("limited_user_reset_date")).or(global_reset),
                        used: Some((t - r).max(0.0)),
                        limit: Some(t),
                        detail: None,
                    });
                }
            }
        }
        return usage;
    };

    let has_ai_credit_snapshot = snaps.keys().any(|k| {
        let k = k.to_ascii_lowercase();
        k.contains("credit") || k == "aic" || k.starts_with("ai_")
    });
    let mut names: Vec<&String> = snaps.keys().collect();
    // Credits first, then alphabetical.
    names.sort_by_key(|k| (!is_credit_quota(k), k.to_string()));
    for name in names {
        let s = &snaps[name.as_str()];
        if !s.is_object() {
            continue;
        }
        if boolean(s.get("unlimited")) == Some(true) {
            continue;
        }
        let entitlement = num(s.get("entitlement")).filter(|e| *e > 0.0);
        let remaining = num(field(s, &["remaining", "quota_remaining"]));
        let used = num(s.get("credits_used"))
            .or_else(|| Some(entitlement? - remaining?))
            .map(|u| u.max(0.0));
        let fraction = num(s.get("percent_remaining"))
            .map(|p| 1.0 - p / 100.0)
            .or_else(|| match (used, entitlement) {
                (Some(u), Some(e)) => Some(u / e),
                _ => None,
            });
        let Some(fraction) = fraction else {
            continue;
        };
        let overage = num(s.get("overage_count")).unwrap_or(0.0);
        let mut details = Vec::new();
        if overage > 0.0 {
            details.push(format!("{overage:.0} over quota"));
        }
        if boolean(s.get("overage_permitted")) == Some(true) {
            details.push("overage enabled".to_owned());
        }
        let lower = name.to_ascii_lowercase();
        let label = if is_credit_quota(name) && !(lower.contains("premium") && has_ai_credit_snapshot) {
            "AIC".to_owned()
        } else if lower.contains("premium") {
            "Premium requests".to_owned()
        } else {
            title_case(name)
        };
        let mut fraction = fraction;
        if overage > 0.0 && let Some(e) = entitlement {
            fraction = fraction.max((e + overage) / e);
        }
        let resets_at = num(s.get("quota_reset_at"))
            .filter(|t| *t > 0.0)
            .and_then(crate::util::from_unix)
            .or(global_reset);
        usage.windows.push(LimitWindow {
            kind: WindowKind::MonthlyCredits,
            label,
            used_fraction: sane_fraction(fraction),
            resets_at,
            used: used.map(|u| u + overage),
            limit: entitlement,
            detail: (!details.is_empty()).then(|| details.join(", ")),
        });
    }
    usage
}

enum Attempt {
    Done(ProviderUsage),
    /// Token was rejected / has no Copilot; try the next candidate.
    Next(String),
}

async fn try_token(client: &reqwest::Client, c: &Candidate) -> Attempt {
    let resp = client
        .get(USER_URL)
        .header(reqwest::header::AUTHORIZATION, format!("token {}", c.token))
        .header(reqwest::header::ACCEPT, "application/json")
        .header("X-GitHub-Api-Version", "2025-04-01")
        .header(reqwest::header::USER_AGENT, crate::USER_AGENT)
        .header("Editor-Version", crate::USER_AGENT)
        .send()
        .await;
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            let mut u = ProviderUsage::new(Provider::GitHubCopilot, SOURCE);
            u.error = Some(format!("Couldn't reach GitHub: {}", crate::describe_reqwest(e)));
            return Attempt::Done(u);
        }
    };
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if status.is_success() {
        return match serde_json::from_str::<Value>(&body) {
            Ok(v) => Attempt::Done(parse_user(&v)),
            Err(e) => {
                let mut u = ProviderUsage::new(Provider::GitHubCopilot, SOURCE);
                u.error = Some(format!("Unexpected Copilot response: {e}"));
                Attempt::Done(u)
            }
        };
    }
    let msg = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| string(v.get("message")).map(|m| one_line(m, 140)))
        .unwrap_or_else(|| status.canonical_reason().unwrap_or("error").to_owned());
    if matches!(status.as_u16(), 401 | 403 | 404) {
        Attempt::Next(format!("token from {} rejected (HTTP {}: {msg})", c.origin, status.as_u16()))
    } else {
        let mut u = ProviderUsage::new(Provider::GitHubCopilot, SOURCE);
        u.error = Some(format!("Copilot usage API returned HTTP {}: {msg}", status.as_u16()));
        Attempt::Done(u)
    }
}

pub(crate) async fn fetch(client: &reqwest::Client) -> Option<ProviderUsage> {
    let cands = candidates().await;
    if cands.is_empty() {
        return None;
    }
    let mut errors = Vec::new();
    for c in &cands {
        match try_token(client, c).await {
            Attempt::Done(u) => return Some(u),
            Attempt::Next(e) => {
                tracing::debug!("copilot: {e}");
                errors.push(e);
            }
        }
    }
    // Every token was refused. Only surface this when a Copilot-specific
    // credential was present; a plain `gh` login without Copilot isn't "configured".
    let copilot_specific = cands
        .iter()
        .any(|c| c.origin.contains("github-copilot") || c.origin == "$COPILOT_GITHUB_TOKEN");
    if !copilot_specific {
        return None;
    }
    let mut u = ProviderUsage::new(Provider::GitHubCopilot, SOURCE);
    u.error = Some(format!("Copilot usage unavailable: {}", errors.join("; ")));
    Some(u)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "login": "someone",
            "access_type_sku": "copilot_enterprise_seat_quota",
            "copilot_plan": "enterprise",
            "chat_enabled": true,
            "quota_reset_date": "2026-12-01",
            "quota_snapshots": {
                "chat": {"overage_count": 0, "overage_permitted": false, "percent_remaining": 100.0,
                         "quota_id": "chat", "quota_remaining": 0.0, "unlimited": true,
                         "remaining": 0, "entitlement": 0},
                "completions": {"percent_remaining": 100.0, "unlimited": true, "remaining": 0, "entitlement": 0},
                "premium_interactions": {"overage_count": 0, "overage_permitted": true,
                         "percent_remaining": 65.9, "quota_id": "premium_interactions",
                         "quota_remaining": 19775.0, "unlimited": false, "quota_reset_at": 0,
                         "credits_used": 10225, "remaining": 19775, "entitlement": 30000}
            },
            "quota_reset_date_utc": "2026-12-01T00:00:00.000Z"
        })
    }

    #[test]
    fn enterprise_premium_interactions() {
        let u = parse_user(&sample());
        assert_eq!(u.plan.as_deref(), Some("Copilot Enterprise"));
        assert_eq!(u.source, SOURCE);
        assert_eq!(u.windows.len(), 1, "unlimited chat/completions are skipped");
        let w = &u.windows[0];
        assert_eq!(w.kind, WindowKind::MonthlyCredits);
        assert_eq!(w.label, "AIC");
        assert!((w.used_fraction - 0.341).abs() < 1e-4);
        assert_eq!(w.used, Some(10225.0));
        assert_eq!(w.limit, Some(30000.0));
        assert_eq!(w.resets_at.unwrap().to_rfc3339(), "2026-12-01T00:00:00+00:00");
        assert_eq!(w.detail.as_deref(), Some("overage enabled"));
    }

    #[test]
    fn free_plan_and_overage_and_missing_fields() {
        let v = json!({
            "copilot_plan": "free",
            "quota_reset_date": "2026-12-01",
            "quota_snapshots": {
                "chat": {"entitlement": 50, "remaining": 10, "unlimited": false},
                "completions": {"entitlement": 2000, "remaining": 2000, "percent_remaining": 100.0},
                "premium_interactions": {"entitlement": 50, "remaining": 0, "percent_remaining": 0.0,
                                         "overage_count": 5, "overage_permitted": false},
                "broken": null,
                "nothing": {"unlimited": false}
            }
        });
        let u = parse_user(&v);
        assert_eq!(u.plan.as_deref(), Some("Copilot Free"));
        let labels: Vec<_> = u.windows.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, ["AIC", "Chat", "Completions"]);
        let aic = &u.windows[0];
        assert!((aic.used_fraction - 1.1).abs() < 1e-6);
        assert_eq!(aic.used, Some(55.0));
        assert_eq!(aic.detail.as_deref(), Some("5 over quota"));
        assert!((u.windows[1].used_fraction - 0.8).abs() < 1e-6);
        assert_eq!(u.windows[2].used_fraction, 0.0);
    }

    #[test]
    fn ai_credits_snapshot_takes_aic_label() {
        let v = json!({"copilot_plan": "individual_pro", "quota_reset_date_utc": null,
            "quota_snapshots": {
                "ai_credits": {"entitlement": 1500, "remaining": 1200, "percent_remaining": 80.0, "quota_reset_at": 1796083200},
                "premium_interactions": {"entitlement": 1500, "remaining": 1500, "percent_remaining": 100.0}
            }});
        let u = parse_user(&v);
        assert_eq!(u.plan.as_deref(), Some("Copilot Pro+"));
        assert_eq!(u.windows[0].label, "AIC");
        assert!((u.windows[0].used_fraction - 0.2).abs() < 1e-6);
        assert_eq!(u.windows[0].resets_at.unwrap().timestamp(), 1796083200);
        assert_eq!(u.windows[1].label, "Premium requests");
        assert!(u.windows[1].resets_at.is_none());
    }

    #[test]
    fn legacy_limited_quotas_and_empty() {
        let v = json!({"copilot_plan": "free", "limited_user_quotas": {"chat": 40, "completions": 1500},
                       "monthly_quotas": {"chat": 50, "completions": 2000},
                       "limited_user_reset_date": "2026-12-05"});
        let u = parse_user(&v);
        assert_eq!(u.windows.len(), 2);
        assert!((u.windows[0].used_fraction - 0.2).abs() < 1e-6);
        assert_eq!(u.windows[0].resets_at.unwrap().to_rfc3339(), "2026-12-05T00:00:00+00:00");

        let u = parse_user(&json!({}));
        assert!(u.windows.is_empty() && u.plan.is_none() && u.error.is_none());
        let u = parse_user(&json!(null));
        assert!(u.windows.is_empty());
    }

    #[test]
    fn config_tokens() {
        let apps = json!({
            "ghe.example.com:Iv1.x": {"user": "a", "oauth_token": "ghu_enterprise"},
            "github.com:Iv1.b507a08c87ecfe98": {"user": "b", "oauth_token": "ghu_public", "githubAppId": "Iv1"},
            "github.com": {"user": "c"}
        });
        assert_eq!(tokens_from_copilot_config(&apps), ["ghu_public", "ghu_enterprise"]);
        assert!(tokens_from_copilot_config(&json!([])).is_empty());
    }
}
