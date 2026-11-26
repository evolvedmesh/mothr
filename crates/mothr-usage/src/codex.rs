//! OpenAI Codex: rate limits are only reported inside session events, so we
//! read them back from the newest rollout logs (and parse live events).

use crate::util::{field, home, num, parse_time, sane_fraction, string, title_case};
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use mothr_core::{LimitWindow, Provider, ProviderUsage, WindowKind};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub(crate) const SOURCE_LOGS: &str = "local logs";
pub(crate) const SOURCE_STREAM: &str = "session stream";

/// How many newest day-directories / files to consider, and how much of each
/// file's tail to scan.
const MAX_DAY_DIRS: usize = 14;
const MAX_FILES: usize = 24;
const TAIL_BYTES: u64 = 8 * 1024 * 1024;

/// Locate the rate-limits object inside whatever we were given and the
/// timestamp of the event, if any.
fn locate(v: &Value) -> Option<(&Value, Option<DateTime<Utc>>)> {
    let ts = parse_time(v.get("timestamp"));
    let candidates = [
        v.pointer("/payload/rate_limits"),
        v.pointer("/payload/rateLimits"),
        v.pointer("/msg/rate_limits"),
        v.get("rate_limits"),
        v.get("rateLimits"),
        v.pointer("/params/rateLimits"),
    ];
    if let Some(rl) = candidates.into_iter().flatten().find(|r| r.is_object()) {
        return Some((rl, ts));
    }
    let looks_like_limits = ["primary", "secondary", "primary_used_percent", "secondary_used_percent"]
        .iter()
        .any(|k| v.get(*k).is_some_and(|x| !x.is_null()));
    looks_like_limits.then_some((v, ts))
}

fn window_kind(minutes: Option<f64>, primary: bool) -> (WindowKind, String) {
    match minutes {
        Some(m) if (m - 300.0).abs() < 1.0 => (WindowKind::FiveHour, "5h".into()),
        Some(m) if (m - 10080.0).abs() < 1.0 => (WindowKind::Weekly, "Weekly".into()),
        Some(m) if m > 0.0 && m < 24.0 * 60.0 => {
            let h = m / 60.0;
            let label = if h.fract().abs() < 1e-6 { format!("{h:.0}h") } else { format!("{m:.0}m") };
            (WindowKind::Other, label)
        }
        Some(m) if (6.0 * 24.0 * 60.0..=8.0 * 24.0 * 60.0).contains(&m) => {
            (WindowKind::Weekly, "Weekly".into())
        }
        Some(m) if m >= 24.0 * 60.0 => (WindowKind::Other, format!("{:.0}d", m / 1440.0)),
        _ if primary => (WindowKind::FiveHour, "5h".into()),
        _ => (WindowKind::Weekly, "Weekly".into()),
    }
}

fn parse_window(w: &Value, primary: bool, base: DateTime<Utc>) -> Option<LimitWindow> {
    let pct = num(field(w, &["used_percent", "usedPercent"]))?;
    let minutes = num(field(w, &["window_minutes", "windowDurationMins", "window_mins"]));
    let resets_at = parse_time(field(w, &["resets_at", "resetsAt"])).or_else(|| {
        num(field(w, &["resets_in_seconds", "resetsInSeconds", "resets_after_seconds"]))
            .filter(|s| *s >= 0.0)
            .map(|s| base + Duration::milliseconds((s * 1000.0) as i64))
    });
    let (kind, label) = window_kind(minutes, primary);
    Some(LimitWindow {
        kind,
        label,
        used_fraction: sane_fraction(pct / 100.0),
        resets_at,
        used: None,
        limit: None,
        detail: None,
    })
}

/// Legacy flat snapshot: `primary_used_percent`, `primary_window_minutes`, …
fn flat_window(rl: &Value, prefix: &str, base: DateTime<Utc>) -> Option<LimitWindow> {
    let obj = serde_json::json!({
        "used_percent": rl.get(format!("{prefix}_used_percent")),
        "window_minutes": rl.get(format!("{prefix}_window_minutes")),
        "resets_in_seconds": rl.get(format!("{prefix}_resets_in_seconds")),
        "resets_at": rl.get(format!("{prefix}_resets_at")),
    });
    parse_window(&obj, prefix == "primary", base)
}

/// Parse Codex rate limits. Accepts the `rate_limits` object itself, a whole
/// rollout line (`{"timestamp":…,"type":"event_msg","payload":{"type":"token_count","rate_limits":{…}}}`),
/// a bare `token_count` payload, or an app-server `rateLimits` notification.
pub fn codex_rate_limits(v: &Value) -> Option<ProviderUsage> {
    let (rl, ts) = locate(v)?;
    let base = ts.unwrap_or_else(Utc::now);
    let mut windows = Vec::new();
    for (key, primary) in [("primary", true), ("secondary", false)] {
        let w = match rl.get(key).filter(|w| w.is_object()) {
            Some(w) => parse_window(w, primary, base),
            None => flat_window(rl, key, base),
        };
        if let Some(w) = w {
            windows.push(w);
        }
    }
    if windows.is_empty() {
        return None;
    }
    // Two windows mapped to the same slot (e.g. both missing window_minutes
    // but one explicitly weekly) would collide in `merge`; disambiguate.
    if windows.len() == 2 && windows[0].kind == windows[1].kind && windows[0].label == windows[1].label {
        windows[1].label.push_str(" (2)");
    }
    let mut usage = ProviderUsage::new(Provider::OpenAi, SOURCE_STREAM);
    usage.plan = string(field(rl, &["plan_type", "planType"])).map(plan_name);
    if let Some(ts) = ts {
        usage.updated_at = ts;
    }
    usage.windows = windows;
    Some(usage)
}

fn plan_name(p: &str) -> String {
    match p.to_ascii_lowercase().as_str() {
        "plus" => "Plus".into(),
        "pro" => "Pro".into(),
        "team" => "Team".into(),
        "business" => "Business".into(),
        "enterprise" => "Enterprise".into(),
        "edu" => "Edu".into(),
        "free" => "Free".into(),
        other => title_case(other),
    }
}

/// `$CODEX_HOME` (if set) and `~/.codex`, deduplicated, existing only.
pub(crate) fn codex_homes() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(h) = std::env::var_os("CODEX_HOME").filter(|h| !h.is_empty()) {
        v.push(PathBuf::from(h));
    }
    if let Some(h) = home() {
        v.push(h.join(".codex"));
    }
    v.dedup();
    v.retain(|p| p.is_dir());
    v
}

fn sorted_subdirs_desc(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    v.sort();
    v.reverse();
    v
}

fn rollout_files_in(dir: &Path, out: &mut Vec<(SystemTime, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let is_rollout = p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"));
        if is_rollout {
            let mtime = e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            out.push((mtime, p));
        }
    }
}

/// Newest rollout files across `<home>/sessions/YYYY/MM/DD` and
/// `<home>/archived_sessions`, newest modification first.
pub(crate) fn newest_rollouts(homes: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for h in homes {
        let mut days = Vec::new();
        'outer: for y in sorted_subdirs_desc(&h.join("sessions")) {
            for m in sorted_subdirs_desc(&y) {
                for d in sorted_subdirs_desc(&m) {
                    days.push(d);
                    if days.len() >= MAX_DAY_DIRS {
                        break 'outer;
                    }
                }
            }
        }
        for d in days {
            rollout_files_in(&d, &mut files);
        }
        rollout_files_in(&h.join("archived_sessions"), &mut files);
    }
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    files.into_iter().take(MAX_FILES).map(|(_, p)| p).collect()
}

fn read_tail(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if start > 0 {
        // Drop the partial first line.
        let i = text.find('\n')?;
        text.drain(..=i);
    }
    Some(text)
}

/// Last `token_count` event with rate limits in one rollout file.
pub(crate) fn last_rate_limits_in(path: &Path) -> Option<ProviderUsage> {
    let text = read_tail(path)?;
    text.lines().rev().find_map(|line| {
        if !line.contains("token_count") || !(line.contains("rate_limits") || line.contains("rateLimits")) {
            return None;
        }
        let v: Value = serde_json::from_str(line).ok()?;
        let is_event = string(v.get("type")) == Some("event_msg")
            && string(v.pointer("/payload/type")) == Some("token_count");
        if !is_event {
            return None;
        }
        codex_rate_limits(&v)
    })
}

/// Scan the given Codex homes. `None` if there are none.
pub(crate) fn scan(homes: &[PathBuf], now: DateTime<Utc>) -> Option<ProviderUsage> {
    if homes.is_empty() {
        return None;
    }
    let found = newest_rollouts(homes).iter().find_map(|p| last_rate_limits_in(p));
    let Some(mut usage) = found else {
        let mut u = ProviderUsage::new(Provider::OpenAi, SOURCE_LOGS);
        u.error = Some(
            "No Codex rate-limit data yet — limits appear after the first message of a Codex session"
                .into(),
        );
        return Some(u);
    };
    usage.source = SOURCE_LOGS.into();
    for w in &mut usage.windows {
        if w.resets_at.is_some_and(|r| r <= now) {
            // The window rolled over since that session; it's fresh again.
            w.used_fraction = 0.0;
            w.resets_at = None;
            w.detail = Some("reset since last Codex session".into());
        }
    }
    Some(usage)
}

pub(crate) async fn fetch() -> Option<ProviderUsage> {
    let homes = codex_homes();
    if homes.is_empty() {
        return None;
    }
    tokio::task::spawn_blocking(move || scan(&homes, Utc::now()))
        .await
        .unwrap_or_else(|e| {
            let mut u = ProviderUsage::new(Provider::OpenAi, SOURCE_LOGS);
            u.error = Some(format!("Reading Codex logs failed: {e}"));
            Some(u)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn line(ts: &str, rl: Value) -> String {
        json!({"timestamp": ts, "type": "event_msg",
               "payload": {"type": "token_count",
                           "info": {"total_token_usage": {"input_tokens": 1200, "output_tokens": 300}},
                           "rate_limits": rl}})
        .to_string()
    }

    #[test]
    fn rate_limits_object_with_resets_in() {
        let rl = json!({"primary": {"used_percent": 12.0, "window_minutes": 300, "resets_in_seconds": 1234},
                        "secondary": {"used_percent": 40.5, "window_minutes": 10080, "resets_in_seconds": 86400},
                        "plan_type": "plus"});
        let l: Value = serde_json::from_str(&line("2026-11-18T10:00:00.000Z", rl.clone())).unwrap();
        let u = codex_rate_limits(&l).unwrap();
        assert_eq!(u.provider, Provider::OpenAi);
        assert_eq!(u.plan.as_deref(), Some("Plus"));
        assert_eq!(u.windows.len(), 2);
        let base = parse_time(Some(&json!("2026-11-18T10:00:00Z"))).unwrap();
        assert_eq!(u.updated_at, base);
        assert_eq!(u.windows[0].kind, WindowKind::FiveHour);
        assert_eq!(u.windows[0].label, "5h");
        assert!((u.windows[0].used_fraction - 0.12).abs() < 1e-6);
        assert_eq!(u.windows[0].resets_at, Some(base + Duration::seconds(1234)));
        assert_eq!(u.windows[1].kind, WindowKind::Weekly);
        assert!((u.windows[1].used_fraction - 0.405).abs() < 1e-6);
        assert_eq!(u.windows[1].resets_at, Some(base + Duration::days(1)));

        // Bare object: resets relative to now.
        let u = codex_rate_limits(&rl).unwrap();
        let r = u.windows[0].resets_at.unwrap();
        assert!((r - Utc::now()).num_seconds().abs() <= 1234 + 5);
    }

    #[test]
    fn rate_limits_resets_at_and_camel_case() {
        let rl = json!({"primary": {"used_percent": 99.0, "window_minutes": 300, "resets_at": 1794970200},
                        "secondary": null, "plan_type": null});
        let u = codex_rate_limits(&rl).unwrap();
        assert_eq!(u.windows.len(), 1);
        assert_eq!(u.windows[0].resets_at.unwrap().timestamp(), 1794970200);
        assert!(u.plan.is_none());

        let n = json!({"method": "account/rateLimits/updated", "params": {"rateLimits": {
            "primary": {"usedPercent": 5, "windowDurationMins": 300, "resetsAt": 1794970200},
            "secondary": {"usedPercent": 70, "windowDurationMins": 10080, "resetsAt": 1795111200},
            "planType": "pro"}}});
        let u = codex_rate_limits(&n).unwrap();
        assert_eq!(u.plan.as_deref(), Some("Pro"));
        assert_eq!(u.windows[1].label, "Weekly");
        assert!((u.windows[1].used_fraction - 0.7).abs() < 1e-6);
    }

    #[test]
    fn rate_limits_legacy_flat_and_missing() {
        let rl = json!({"primary_used_percent": 30.0, "secondary_used_percent": 10.0,
                        "primary_to_secondary_ratio_percent": 40.0,
                        "primary_window_minutes": 300, "secondary_window_minutes": 10080});
        let u = codex_rate_limits(&rl).unwrap();
        assert_eq!(u.windows.len(), 2);
        assert!(u.windows.iter().all(|w| w.resets_at.is_none()));

        // Missing window_minutes → positional defaults.
        let u = codex_rate_limits(&json!({"primary": {"used_percent": 1}, "secondary": {"used_percent": 2}})).unwrap();
        assert_eq!(u.windows[0].kind, WindowKind::FiveHour);
        assert_eq!(u.windows[1].kind, WindowKind::Weekly);

        // Unusual windows.
        let u = codex_rate_limits(&json!({"primary": {"used_percent": 1, "window_minutes": 60}})).unwrap();
        assert_eq!(u.windows[0].label, "1h");

        assert!(codex_rate_limits(&json!({"payload": {"type": "token_count", "rate_limits": null}})).is_none());
        assert!(codex_rate_limits(&json!({"primary": {"window_minutes": 300}})).is_none());
        assert!(codex_rate_limits(&json!({"type": "event_msg"})).is_none());
        assert!(codex_rate_limits(&json!(null)).is_none());
    }

    #[test]
    fn scans_newest_rollout() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".codex");
        let old_day = home.join("sessions/2026/11/16");
        let new_day = home.join("sessions/2026/11/18");
        std::fs::create_dir_all(&old_day).unwrap();
        std::fs::create_dir_all(&new_day).unwrap();

        let mut f = File::create(old_day.join("rollout-2026-11-16T09-00-00-aaa.jsonl")).unwrap();
        writeln!(f, "{}", line("2026-11-16T09:00:00Z",
            json!({"primary": {"used_percent": 80.0, "window_minutes": 300, "resets_in_seconds": 60}}))).unwrap();
        drop(f);

        // Newest file: one good token_count followed by noise and a null one.
        let mut f = File::create(new_day.join("rollout-2026-11-18T09-00-00-bbb.jsonl")).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-11-18T09:00:00Z","type":"session_meta","payload":{{"id":"x"}}}}"#).unwrap();
        writeln!(f, "{}", line("2026-11-18T09:30:00Z", json!({
            "primary": {"used_percent": 25.0, "window_minutes": 300, "resets_in_seconds": 3600},
            "secondary": {"used_percent": 60.0, "window_minutes": 10080, "resets_in_seconds": 400000},
            "plan_type": "team"}))).unwrap();
        writeln!(f, "{}", line("2026-11-18T09:31:00Z", Value::Null)).unwrap();
        writeln!(f, "not json at all token_count rate_limits").unwrap();
        writeln!(f, r#"{{"timestamp":"2026-11-18T09:32:00Z","type":"response_item","payload":{{"type":"message"}}}}"#).unwrap();
        drop(f);

        let now = parse_time(Some(&json!("2026-11-18T09:45:00Z"))).unwrap();
        let u = scan(std::slice::from_ref(&home), now).unwrap();
        assert_eq!(u.source, SOURCE_LOGS);
        assert!(u.error.is_none());
        assert_eq!(u.plan.as_deref(), Some("Team"));
        assert!((u.windows[0].used_fraction - 0.25).abs() < 1e-6);
        assert!((u.windows[1].used_fraction - 0.60).abs() < 1e-6);

        // Much later: both windows have rolled over.
        let later = now + Duration::days(10);
        let u = scan(std::slice::from_ref(&home), later).unwrap();
        assert!(u.windows.iter().all(|w| w.used_fraction == 0.0 && w.resets_at.is_none()));

        // Home with no data yet.
        let empty = tmp.path().join("empty-codex");
        std::fs::create_dir_all(&empty).unwrap();
        let u = scan(&[empty], now).unwrap();
        assert!(u.windows.is_empty());
        assert!(u.error.is_some());

        assert!(scan(&[], now).is_none());
    }
}
