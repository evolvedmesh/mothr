//! OpenCode and Hermes: multi-provider agents without one quota API of their own.

use crate::util::{field, find_program, home, num, one_line, parse_time, run_command, sane_fraction, string};
use serde_json::Value;
use mothr_core::{LimitWindow, Provider, ProviderUsage, WindowKind};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const SOURCE_SESSION: &str = "session";

fn any_dir(paths: &[PathBuf]) -> bool {
    paths.iter().any(|p| p.is_dir())
}

fn xdg(var: &str, fallback: &[&str], leaf: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home().map(|h| fallback.iter().fold(h, |p, s| p.join(s))))
        .map(|p| p.join(leaf))
}

// ---------------------------------------------------------------- OpenCode

pub(crate) async fn fetch_opencode() -> Option<ProviderUsage> {
    let mut dirs = Vec::new();
    dirs.extend(xdg("XDG_DATA_HOME", &[".local", "share"], "opencode"));
    dirs.extend(xdg("XDG_CONFIG_HOME", &[".config"], "opencode"));
    dirs.extend(home().map(|h| h.join(".local/share/opencode")));
    dirs.extend(home().map(|h| h.join(".config/opencode")));
    if !any_dir(&dirs) {
        return None;
    }
    let mut u = ProviderUsage::new(Provider::OpenCode, SOURCE_SESSION);
    u.error = Some(
        "OpenCode routes to many providers and has no account-limit API; usage is tracked per session"
            .into(),
    );
    Some(u)
}

// ---------------------------------------------------------------- Hermes

fn hermes_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = std::env::var_os("HERMES_HOME").filter(|h| !h.is_empty()) {
        v.push(PathBuf::from(h));
    }
    v.extend(home().map(|h| h.join(".hermes")));
    v.push(PathBuf::from("/opt/hermes-agent"));
    v
}

fn hermes_window(w: &Value) -> Option<LimitWindow> {
    let label = string(w.get("label")).unwrap_or("Limit");
    let pct = num(field(w, &["used_percent", "usedPercent", "utilization"]))
        .map(|p| p / 100.0)
        .or_else(|| num(field(w, &["used_fraction", "usedFraction"])))?;
    let lower = label.to_ascii_lowercase();
    let (kind, label) = if lower.contains("session") || lower.contains("5h") || lower.contains("5-hour") || lower.contains("five") {
        (WindowKind::FiveHour, "5h".to_owned())
    } else if matches!(lower.as_str(), "current week" | "weekly" | "week" | "7d" | "seven day")
        || lower.starts_with("current week (all")
    {
        (WindowKind::Weekly, "Weekly".to_owned())
    } else if lower.contains("week") {
        (WindowKind::WeeklyModel, label.to_owned())
    } else if lower.contains("credit") || lower.contains("spend") || lower.contains('$') {
        (WindowKind::Spend, label.to_owned())
    } else {
        (WindowKind::Other, label.to_owned())
    };
    Some(LimitWindow {
        kind,
        label,
        used_fraction: sane_fraction(pct),
        resets_at: parse_time(field(w, &["resets_at", "resetsAt"])),
        used: num(w.get("used")),
        limit: num(w.get("limit")),
        detail: string(w.get("detail")).map(str::to_owned),
    })
}

/// Parse `hermes usage --json`.
pub(crate) fn parse_hermes_usage(v: &Value) -> Option<ProviderUsage> {
    if !v.is_object() {
        return None;
    }
    let upstream = string(v.get("provider"));
    let source = match upstream {
        Some(p) => format!("hermes usage ({p})"),
        None => "hermes usage".to_owned(),
    };
    let mut u = ProviderUsage::new(Provider::Hermes, source);
    u.plan = string(v.get("plan")).map(str::to_owned);
    if let Some(t) = parse_time(v.get("fetched_at")) {
        u.updated_at = t;
    }
    for w in v.get("windows").and_then(Value::as_array).into_iter().flatten() {
        if let Some(w) = hermes_window(w)
            && !u.windows.iter().any(|e| e.kind == w.kind && e.label == w.label)
        {
            u.windows.push(w);
        }
    }
    if u.windows.is_empty() {
        u.error = Some(match string(v.get("unavailable_reason")) {
            Some(r) => one_line(r, 160),
            None => "Hermes reported no account limits".into(),
        });
    }
    Some(u)
}

/// Turn the human-readable failure text into a short message.
fn hermes_failure_message(text: &str) -> String {
    let provider = text
        .split_once("provider '")
        .and_then(|(_, rest)| rest.split_once('\''))
        .map(|(p, _)| p);
    match provider {
        Some(p) => format!("Hermes' configured provider '{p}' exposes no account limits"),
        None if text.trim().is_empty() => "Hermes has no account-limit data for its configured provider".into(),
        None => one_line(text, 160),
    }
}

fn no_data(msg: String) -> ProviderUsage {
    let mut u = ProviderUsage::new(Provider::Hermes, SOURCE_SESSION);
    u.error = Some(msg);
    u
}

pub(crate) async fn fetch_hermes() -> Option<ProviderUsage> {
    let dirs = hermes_dirs();
    if !any_dir(&dirs) {
        return None;
    }
    let mut extra: Vec<PathBuf> = dirs.iter().map(|d| d.join("bin").join("hermes")).collect();
    extra.extend(dirs.iter().map(|d| d.join("venv/bin/hermes")));
    let Some(bin) = find_program("hermes", &extra) else {
        return Some(no_data("Hermes CLI not found; account limits unavailable".into()));
    };
    Some(hermes_usage(&bin).await)
}

/// `hermes usage --json` — a read-only, non-billable account-limit lookup for
/// Hermes' configured model provider (Anthropic OAuth, Codex, OpenRouter …).
async fn hermes_usage(bin: &Path) -> ProviderUsage {
    let Some(out) = run_command(&bin.to_string_lossy(), &["usage", "--json"], Duration::from_secs(20)).await else {
        return no_data("`hermes usage` failed or timed out".into());
    };
    if let Ok(v) = serde_json::from_str::<Value>(out.stdout.trim())
        && let Some(u) = parse_hermes_usage(&v)
    {
        return u;
    }
    let text = if out.stdout.trim().is_empty() { &out.stderr } else { &out.stdout };
    if !out.success && text.contains("invalid choice") {
        return no_data("This Hermes version has no `usage` command; account limits unavailable".into());
    }
    no_data(hermes_failure_message(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hermes_usage_json() {
        let v = json!({
            "provider": "anthropic", "source": "oauth_usage_api", "title": "Account limits",
            "plan": null, "fetched_at": "2026-11-17T22:50:14.566594+00:00",
            "windows": [
                {"label": "Current session", "used_percent": 8.0, "resets_at": "2026-11-18T02:50:00.402478+00:00", "detail": null},
                {"label": "Current week", "used_percent": 19.0, "resets_at": "2026-11-19T18:00:00.402499+00:00", "detail": null},
                {"label": "Current week (Opus)", "used_percent": 2.0, "resets_at": null},
                {"label": "OpenRouter credits", "used_percent": 50, "used": 5, "limit": 10, "detail": "USD"},
                {"label": "no numbers"},
                null
            ],
            "details": [], "unavailable_reason": null
        });
        let u = parse_hermes_usage(&v).unwrap();
        assert_eq!(u.provider, Provider::Hermes);
        assert_eq!(u.source, "hermes usage (anthropic)");
        assert!(u.plan.is_none() && u.error.is_none());
        let kinds: Vec<_> = u.windows.iter().map(|w| (w.kind, w.label.as_str())).collect();
        assert_eq!(kinds, [
            (WindowKind::FiveHour, "5h"),
            (WindowKind::Weekly, "Weekly"),
            (WindowKind::WeeklyModel, "Current week (Opus)"),
            (WindowKind::Spend, "OpenRouter credits"),
        ]);
        assert!((u.windows[0].used_fraction - 0.08).abs() < 1e-6);
        assert_eq!(u.windows[3].limit, Some(10.0));
        assert_eq!(u.updated_at.to_rfc3339(), "2026-11-17T22:50:14.566594+00:00");
    }

    #[test]
    fn hermes_usage_unavailable() {
        let u = parse_hermes_usage(&json!({"provider": "nous", "windows": [], "unavailable_reason": "no credential"})).unwrap();
        assert!(u.windows.is_empty());
        assert_eq!(u.error.as_deref(), Some("no credential"));
        let u = parse_hermes_usage(&json!({})).unwrap();
        assert!(u.error.is_some());
        assert!(parse_hermes_usage(&json!("text")).is_none());

        let m = hermes_failure_message("No account usage available for provider 'custom': no credential is configured for it, the provider has no usage endpoint, or the fetch failed.\n");
        assert_eq!(m, "Hermes' configured provider 'custom' exposes no account limits");
        assert!(!hermes_failure_message("").is_empty());
    }
}
