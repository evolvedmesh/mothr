//! Anthropic / Claude Code: OAuth usage endpoint + live `rate_limit_event`s.

use crate::util::{boolean, field, home, num, one_line, parse_time, sane_fraction, string, title_case};
use chrono::{DateTime, Utc};
use serde_json::Value;
use mothr_core::{LimitWindow, Provider, ProviderUsage, WindowKind};
use std::path::PathBuf;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
pub(crate) const SOURCE_API: &str = "oauth api";
pub(crate) const SOURCE_STREAM: &str = "session stream";
const EXPIRED: &str = "Claude login expired — run `claude` to refresh";

/// The parts of Claude Code's OAuth credentials we care about.
#[derive(Debug, Clone, Default)]
pub(crate) struct ClaudeCreds {
    pub token: Option<String>,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl ClaudeCreds {
    /// Parse `{"claudeAiOauth": {...}}` (the file / keychain payload).
    pub(crate) fn from_json(v: &Value) -> Option<Self> {
        let o = v.get("claudeAiOauth").filter(|o| o.is_object())?;
        Some(Self {
            token: string(o.get("accessToken")).map(str::to_owned),
            subscription_type: string(o.get("subscriptionType")).map(str::to_owned),
            rate_limit_tier: string(o.get("rateLimitTier")).map(str::to_owned),
            expires_at: parse_time(o.get("expiresAt")),
        })
    }

    pub(crate) fn plan(&self) -> Option<String> {
        plan_name(self.subscription_type.as_deref(), self.rate_limit_tier.as_deref())
    }

    pub(crate) fn expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|t| t <= now)
    }
}

fn credential_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        v.push(PathBuf::from(dir).join(".credentials.json"));
    }
    if let Some(h) = home() {
        v.push(h.join(".claude").join(".credentials.json"));
    }
    v
}

async fn load_creds() -> Option<ClaudeCreds> {
    for path in credential_paths() {
        let Ok(text) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => {
                if let Some(c) = ClaudeCreds::from_json(&v).filter(|c| c.token.is_some()) {
                    return Some(c);
                }
            }
            Err(e) => tracing::warn!("unreadable {}: {e}", path.display()),
        }
    }
    keychain_creds().await
}

#[cfg(target_os = "macos")]
async fn keychain_creds() -> Option<ClaudeCreds> {
    let out = crate::util::run_command(
        "/usr/bin/security",
        &["find-generic-password", "-s", "Claude Code-credentials", "-w"],
        std::time::Duration::from_secs(5),
    )
    .await?;
    if !out.success {
        return None;
    }
    let v: Value = serde_json::from_str(out.stdout.trim()).ok()?;
    ClaudeCreds::from_json(&v).filter(|c| c.token.is_some())
}

#[cfg(not(target_os = "macos"))]
async fn keychain_creds() -> Option<ClaudeCreds> {
    None
}

/// "max" + "default_claude_max_20x" → "Max 20x"; "pro" → "Pro".
pub(crate) fn plan_name(subscription: Option<&str>, tier: Option<&str>) -> Option<String> {
    let base = subscription.map(|s| match s.to_ascii_lowercase().as_str() {
        "max" => "Max".to_owned(),
        "pro" => "Pro".to_owned(),
        "team" => "Team".to_owned(),
        "enterprise" => "Enterprise".to_owned(),
        "free" => "Free".to_owned(),
        other => title_case(other),
    });
    // A trailing "_20x" style multiplier in the tier.
    let multiplier = tier.and_then(|t| {
        let last = t.rsplit(['_', '-']).next()?;
        let digits = last.strip_suffix(['x', 'X'])?;
        (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
            .then(|| format!("{digits}x"))
    });
    match (base, multiplier) {
        (Some(b), Some(m)) => Some(format!("{b} {m}")),
        (Some(b), None) => Some(b),
        (None, Some(m)) if tier.is_some_and(|t| t.contains("max")) => Some(format!("Max {m}")),
        _ => None,
    }
}

/// Kind + label for a window key used by both the API (`seven_day_opus`) and
/// the stream (`unifiedWindows` keys / `rateLimitType`).
pub(crate) fn window_for_key(key: &str) -> (WindowKind, String) {
    match key {
        "five_hour" | "session" => (WindowKind::FiveHour, "5h".into()),
        "seven_day" | "weekly" | "weekly_all" => (WindowKind::Weekly, "Weekly".into()),
        "overage" | "extra_usage" => (WindowKind::Spend, "Extra usage".into()),
        k => match k.strip_prefix("seven_day_") {
            Some("oauth_apps") => (WindowKind::WeeklyModel, "OAuth apps weekly".into()),
            Some(scope) if !scope.is_empty() => {
                (WindowKind::WeeklyModel, format!("{} weekly", title_case(scope)))
            }
            _ => (WindowKind::Other, title_case(k)),
        },
    }
}

fn push_unique(windows: &mut Vec<LimitWindow>, w: LimitWindow) {
    if !windows.iter().any(|e| e.kind == w.kind && e.label == w.label) {
        windows.push(w);
    }
}

/// Parse the `/api/oauth/usage` response. Utilization there is a percent.
pub(crate) fn parse_oauth_usage(v: &Value) -> Vec<LimitWindow> {
    let mut windows = Vec::new();
    let Some(obj) = v.as_object() else {
        return windows;
    };

    // Fixed order first so the UI gets 5h, Weekly, then model windows.
    let mut keys: Vec<&String> = obj
        .keys()
        .filter(|k| *k == "five_hour" || *k == "seven_day" || k.starts_with("seven_day_"))
        .collect();
    keys.sort_by_key(|k| match k.as_str() {
        "five_hour" => (0, String::new()),
        "seven_day" => (1, String::new()),
        other => (2, other.to_owned()),
    });
    for key in keys {
        let w = &obj[key.as_str()];
        if !w.is_object() {
            continue;
        }
        let Some(pct) = num(w.get("utilization")) else {
            continue;
        };
        let (kind, label) = window_for_key(key);
        let detail = string(w.get("locked_reason")).map(|r| format!("locked: {}", r.replace('_', " ")));
        push_unique(
            &mut windows,
            LimitWindow {
                kind,
                label,
                used_fraction: sane_fraction(pct / 100.0),
                resets_at: parse_time(field(w, &["resets_at", "resetsAt"])),
                used: num(w.get("used_dollars")),
                limit: num(w.get("limit_dollars")),
                detail,
            },
        );
    }

    // Newer responses also carry a generic `limits` list, including
    // model-scoped weekly limits that have no `seven_day_*` key.
    if let Some(limits) = v.get("limits").and_then(Value::as_array) {
        for l in limits {
            let Some(kind_str) = string(l.get("kind")) else {
                continue;
            };
            let Some(pct) = num(l.get("percent")) else {
                continue;
            };
            let (kind, label) = match kind_str {
                "weekly_scoped" => {
                    let scope = l.get("scope");
                    let name = scope
                        .and_then(|s| s.get("model"))
                        .and_then(|m| string(m.get("display_name")).or(string(m.get("id"))))
                        .or_else(|| scope.and_then(|s| string(s.get("surface"))));
                    match name {
                        Some(n) => (WindowKind::WeeklyModel, format!("{} weekly", title_case(n))),
                        None => continue,
                    }
                }
                "session" | "weekly_all" => window_for_key(kind_str),
                _ => continue,
            };
            let detail = string(l.get("severity"))
                .filter(|s| *s != "normal")
                .map(str::to_owned);
            push_unique(
                &mut windows,
                LimitWindow {
                    kind,
                    label,
                    used_fraction: sane_fraction(pct / 100.0),
                    resets_at: parse_time(l.get("resets_at")),
                    used: None,
                    limit: None,
                    detail,
                },
            );
        }
    }

    // Paid extra usage ("overage" credits), only when switched on.
    if let Some(extra) = v.get("extra_usage").filter(|e| boolean(e.get("is_enabled")) == Some(true)) {
        let scale = 10f64.powi(num(extra.get("decimal_places")).unwrap_or(2.0).clamp(0.0, 6.0) as i32);
        let used = num(extra.get("used_credits")).map(|c| c / scale);
        let limit = num(extra.get("monthly_limit")).map(|c| c / scale);
        let fraction = num(extra.get("utilization"))
            .map(|p| p / 100.0)
            .or_else(|| match (used, limit) {
                (Some(u), Some(l)) if l > 0.0 => Some(u / l),
                _ => None,
            })
            .unwrap_or(0.0);
        let currency = string(extra.get("currency")).unwrap_or("USD");
        let detail = if limit.is_none() {
            format!("{currency}, no monthly cap")
        } else {
            currency.to_owned()
        };
        windows.push(LimitWindow {
            kind: WindowKind::Spend,
            label: "Extra usage".into(),
            used_fraction: sane_fraction(fraction),
            resets_at: None,
            used,
            limit,
            detail: Some(detail),
        });
    }
    windows
}

pub(crate) async fn fetch(client: &reqwest::Client) -> Option<ProviderUsage> {
    let creds = load_creds().await?;
    let mut usage = ProviderUsage::new(Provider::Anthropic, SOURCE_API);
    usage.plan = creds.plan();
    if creds.expired(Utc::now()) {
        usage.error = Some(EXPIRED.into());
        return Some(usage);
    }
    let token = creds.token.as_deref()?;
    let resp = client
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header(reqwest::header::USER_AGENT, crate::USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await;
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            usage.error = Some(format!("Couldn't reach Claude usage API: {}", crate::describe_reqwest(e)));
            return Some(usage);
        }
    };
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let api_msg = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(|s| one_line(s, 160))
            });
        usage.error = Some(match status.as_u16() {
            401 => EXPIRED.into(),
            429 => "Claude usage API is rate limiting requests; try again shortly".into(),
            code => match api_msg {
                Some(m) => format!("Claude usage API returned HTTP {code}: {m}"),
                None => format!("Claude usage API returned HTTP {code}"),
            },
        });
        return Some(usage);
    }
    match serde_json::from_str::<Value>(&body) {
        Ok(v) => {
            usage.windows = parse_oauth_usage(&v);
            if usage.windows.is_empty() {
                usage.error = Some("Claude usage API returned no limit windows".into());
            }
        }
        Err(e) => usage.error = Some(format!("Unexpected Claude usage response: {e}")),
    }
    Some(usage)
}

/// Parse a Claude Code stream-json `rate_limit_event` line (or its bare
/// `rate_limit_info` object). Utilization here is already a 0–1 fraction and
/// `resetsAt` is unix seconds.
pub fn claude_rate_limit_event(v: &Value) -> Option<ProviderUsage> {
    let info = match v.get("rate_limit_info") {
        Some(i) if i.is_object() => i,
        _ if v.get("rateLimitType").is_some() || v.get("unifiedWindows").is_some() => v,
        _ => return None,
    };

    let top_type = string(info.get("rateLimitType"));
    let using_overage = boolean(info.get("isUsingOverage")) == Some(true);
    let mut windows = Vec::new();

    let window_from = |key: &str, w: &Value| -> Option<LimitWindow> {
        let status = string(w.get("status"));
        let fraction = num(field(w, &["utilization", "surpassedThreshold", "surpassed_threshold"]))
            .or_else(|| (status == Some("rejected")).then_some(1.0))?;
        let (kind, label) = window_for_key(key);
        let mut details = Vec::new();
        if let Some(s) = status.filter(|s| *s != "allowed") {
            details.push(s.replace('_', " "));
        }
        if using_overage && top_type == Some(key) {
            details.push("using extra usage".into());
        }
        Some(LimitWindow {
            kind,
            label,
            used_fraction: sane_fraction(fraction),
            resets_at: parse_time(field(w, &["resetsAt", "resets_at"])),
            used: None,
            limit: None,
            detail: (!details.is_empty()).then(|| details.join(", ")),
        })
    };

    if let Some(unified) = info.get("unifiedWindows").and_then(Value::as_object) {
        let mut keys: Vec<&String> = unified.keys().collect();
        keys.sort_by_key(|k| (window_for_key(k).0 as u8, k.to_string()));
        for k in keys {
            if let Some(w) = window_from(k, &unified[k.as_str()]) {
                push_unique(&mut windows, w);
            }
        }
    }
    if windows.is_empty()
        && let Some(t) = top_type
        && let Some(w) = window_from(t, info)
    {
        windows.push(w);
    }
    if windows.is_empty() {
        return None;
    }
    let mut usage = ProviderUsage::new(Provider::Anthropic, SOURCE_STREAM);
    usage.windows = windows;
    Some(usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const EVENT: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1794970200,"rateLimitType":"five_hour","overageStatus":"rejected","isUsingOverage":false,"unifiedWindows":{"five_hour":{"utilization":0.06,"resetsAt":1794970200,"status":"allowed"},"seven_day":{"utilization":0.18,"resetsAt":1795111200,"status":"allowed"}}}}"#;

    #[test]
    fn rate_limit_event_sample() {
        let v: Value = serde_json::from_str(EVENT).unwrap();
        let u = claude_rate_limit_event(&v).unwrap();
        assert_eq!(u.provider, Provider::Anthropic);
        assert_eq!(u.source, SOURCE_STREAM);
        assert_eq!(u.windows.len(), 2);
        let five = &u.windows[0];
        assert_eq!(five.kind, WindowKind::FiveHour);
        assert_eq!(five.label, "5h");
        assert!((five.used_fraction - 0.06).abs() < 1e-6);
        assert_eq!(five.resets_at.unwrap().timestamp(), 1794970200);
        assert_eq!(five.detail, None);
        let week = &u.windows[1];
        assert_eq!(week.kind, WindowKind::Weekly);
        assert_eq!(week.label, "Weekly");
        assert!((week.used_fraction - 0.18).abs() < 1e-6);
        assert_eq!(week.resets_at.unwrap().timestamp(), 1795111200);

        // The bare rate_limit_info object works too.
        let bare = claude_rate_limit_event(&v["rate_limit_info"]).unwrap();
        assert_eq!(bare.windows, u.windows);
    }

    #[test]
    fn rate_limit_event_fallback_and_model_windows() {
        let v = json!({"type":"rate_limit_event","rate_limit_info":{
            "status":"allowed_warning","resetsAt":1795111200,"rateLimitType":"seven_day_opus",
            "utilization":0.91,"isUsingOverage":true}});
        let u = claude_rate_limit_event(&v).unwrap();
        assert_eq!(u.windows.len(), 1);
        let w = &u.windows[0];
        assert_eq!(w.kind, WindowKind::WeeklyModel);
        assert_eq!(w.label, "Opus weekly");
        assert!((w.used_fraction - 0.91).abs() < 1e-6);
        assert_eq!(w.detail.as_deref(), Some("allowed warning, using extra usage"));

        // Rejected without utilization → exhausted.
        let v = json!({"rate_limit_info":{"status":"rejected","rateLimitType":"five_hour","resetsAt":1794970200}});
        let u = claude_rate_limit_event(&v).unwrap();
        assert_eq!(u.windows[0].used_fraction, 1.0);

        // Unknown unified key + nulls.
        let v = json!({"rate_limit_info":{"unifiedWindows":{
            "seven_day_sonnet":{"utilization":0.5,"resetsAt":null},
            "five_hour":null,
            "weird":{"status":"allowed"}}}});
        let u = claude_rate_limit_event(&v).unwrap();
        assert_eq!(u.windows.len(), 1);
        assert_eq!(u.windows[0].label, "Sonnet weekly");
        assert!(u.windows[0].resets_at.is_none());
    }

    #[test]
    fn rate_limit_event_rejects_garbage() {
        assert!(claude_rate_limit_event(&json!({"type":"assistant"})).is_none());
        assert!(claude_rate_limit_event(&json!({"rate_limit_info":{"status":"allowed"}})).is_none());
        assert!(claude_rate_limit_event(&json!({"rate_limit_info":null})).is_none());
        assert!(claude_rate_limit_event(&json!([1, 2])).is_none());
        assert!(claude_rate_limit_event(&json!("x")).is_none());
    }

    #[test]
    fn oauth_usage_response() {
        let v = json!({
            "five_hour": {"utilization": 8.0, "resets_at": "2026-11-18T02:49:59.963320+00:00",
                          "limit_dollars": null, "used_dollars": null, "locked_reason": null},
            "seven_day": {"utilization": 19.0, "resets_at": "2026-11-19T17:59:59.963337+00:00"},
            "seven_day_oauth_apps": null,
            "seven_day_opus": {"utilization": 42.5, "resets_at": "2026-11-19T18:00:00+00:00"},
            "seven_day_sonnet": null,
            "tangelo": null,
            "extra_usage": {"is_enabled": false, "monthly_limit": null, "used_credits": null},
            "limits": [
                {"kind": "session", "percent": 8, "severity": "normal", "resets_at": "2026-11-18T02:49:59Z"},
                {"kind": "weekly_all", "percent": 19, "severity": "normal"},
                {"kind": "weekly_scoped", "percent": 3, "severity": "warning",
                 "resets_at": "2026-11-19T18:00:00+00:00",
                 "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null}},
                {"kind": "weekly_scoped", "percent": 3, "scope": null},
                {"kind": "mystery", "percent": 50}
            ]
        });
        let w = parse_oauth_usage(&v);
        let labels: Vec<_> = w.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, ["5h", "Weekly", "Opus weekly", "Fable weekly"]);
        assert_eq!(w[0].kind, WindowKind::FiveHour);
        assert!((w[0].used_fraction - 0.08).abs() < 1e-6);
        assert_eq!(w[0].resets_at.unwrap().to_rfc3339(), "2026-11-18T02:49:59.963320+00:00");
        assert_eq!(w[1].kind, WindowKind::Weekly);
        assert!((w[1].used_fraction - 0.19).abs() < 1e-6);
        assert_eq!(w[2].kind, WindowKind::WeeklyModel);
        assert!((w[2].used_fraction - 0.425).abs() < 1e-6);
        assert_eq!(w[3].kind, WindowKind::WeeklyModel);
        assert_eq!(w[3].detail.as_deref(), Some("warning"));
        assert!((w[3].used_fraction - 0.03).abs() < 1e-6);
    }

    #[test]
    fn oauth_usage_extra_and_missing() {
        let v = json!({
            "five_hour": {"utilization": null},
            "seven_day": {"utilization": "55", "resets_at": null},
            "extra_usage": {"is_enabled": true, "monthly_limit": 5000, "used_credits": 1250,
                            "utilization": null, "currency": "USD", "decimal_places": 2}
        });
        let w = parse_oauth_usage(&v);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].label, "Weekly");
        assert!((w[0].used_fraction - 0.55).abs() < 1e-6);
        assert!(w[0].resets_at.is_none());
        assert_eq!(w[1].kind, WindowKind::Spend);
        assert_eq!(w[1].used, Some(12.5));
        assert_eq!(w[1].limit, Some(50.0));
        assert!((w[1].used_fraction - 0.25).abs() < 1e-6);

        assert!(parse_oauth_usage(&json!({})).is_empty());
        assert!(parse_oauth_usage(&json!(null)).is_empty());
        assert!(parse_oauth_usage(&json!({"five_hour": 3})).is_empty());
    }

    #[test]
    fn credentials_and_plan() {
        let v = json!({"claudeAiOauth": {"accessToken": "sk-ant-oat01-x", "refreshToken": "r",
            "expiresAt": 1794981428375u64, "subscriptionType": "max",
            "rateLimitTier": "default_claude_max_20x"}, "mcpOAuth": {}});
        let c = ClaudeCreds::from_json(&v).unwrap();
        assert_eq!(c.plan().as_deref(), Some("Max 20x"));
        assert_eq!(c.expires_at.unwrap().timestamp(), 1794981428);
        assert!(c.expired(c.expires_at.unwrap()));
        assert!(!c.expired(c.expires_at.unwrap() - chrono::Duration::seconds(1)));

        let partial = ClaudeCreds::from_json(&json!({"claudeAiOauth": {"accessToken": null}})).unwrap();
        assert!(partial.token.is_none() && partial.plan().is_none() && !partial.expired(Utc::now()));
        assert!(ClaudeCreds::from_json(&json!({"other": 1})).is_none());

        assert_eq!(plan_name(Some("pro"), Some("default_claude_ai")).as_deref(), Some("Pro"));
        assert_eq!(plan_name(Some("max"), Some("default_claude_max_5x")).as_deref(), Some("Max 5x"));
        assert_eq!(plan_name(None, Some("default_claude_max_20x")).as_deref(), Some("Max 20x"));
        assert_eq!(plan_name(Some("team"), None).as_deref(), Some("Team"));
        assert_eq!(plan_name(None, None), None);
    }
}
