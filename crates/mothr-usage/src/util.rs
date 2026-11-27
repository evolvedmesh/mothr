//! Small, defensive helpers shared by the provider fetchers.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// Read a number that may be encoded as a JSON number or a numeric string.
pub(crate) fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        _ => None,
    }
}

pub(crate) fn string(v: Option<&Value>) -> Option<&str> {
    v?.as_str().map(str::trim).filter(|s| !s.is_empty())
}

pub(crate) fn boolean(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// First present field among several spellings (`resets_at` / `resetsAt`).
pub(crate) fn field<'a>(obj: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names
        .iter()
        .find_map(|n| obj.get(*n).filter(|v| !v.is_null()))
}

/// Unix seconds (or milliseconds, auto-detected) to a UTC timestamp.
pub(crate) fn from_unix(n: f64) -> Option<DateTime<Utc>> {
    if !n.is_finite() || n <= 0.0 {
        return None;
    }
    // Anything past ~year 33658 in seconds is really milliseconds.
    let secs = if n > 1.0e12 { n / 1000.0 } else { n };
    Utc.timestamp_opt(secs.trunc() as i64, ((secs.fract()) * 1e9) as u32)
        .single()
}

/// Parse a timestamp given as unix secs/ms, RFC 3339 string or a bare `YYYY-MM-DD` date.
pub(crate) fn parse_time(v: Option<&Value>) -> Option<DateTime<Utc>> {
    match v? {
        Value::Number(n) => from_unix(n.as_f64()?),
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(dt.with_timezone(&Utc));
            }
            if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                return d.and_hms_opt(0, 0, 0).map(|n| n.and_utc());
            }
            s.parse::<f64>().ok().and_then(from_unix)
        }
        _ => None,
    }
}

/// Clamp a fraction into a sane range (allowing overage up to 10x).
pub(crate) fn sane_fraction(f: f64) -> f32 {
    if f.is_finite() { f.clamp(0.0, 10.0) as f32 } else { 0.0 }
}

pub(crate) fn home() -> Option<PathBuf> {
    dirs::home_dir()
}

/// Upper-case the first letter of each `_`/`-`/space separated word.
pub(crate) fn title_case(s: &str) -> String {
    s.split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Output of a finished subprocess.
pub(crate) struct CmdOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run a program non-interactively with a timeout. `None` if it could not be
/// spawned or timed out (the child is killed on drop).
pub(crate) async fn run_command(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Option<CmdOutput> {
    let mut cmd = Command::new(program);
    mothr_core::process::no_window(cmd.as_std_mut());
    let child = cmd
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => Some(CmdOutput {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }),
        Ok(Err(e)) => {
            tracing::debug!("{program} failed: {e}");
            None
        }
        Err(_) => {
            tracing::debug!("{program} timed out after {timeout:?}");
            None
        }
    }
}

/// Look a program up on `PATH` (plus a few extra candidate locations).
pub(crate) fn find_program(name: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    let path_dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    path_dirs
        .iter()
        .flat_map(|d| exts.iter().map(move |e| d.join(format!("{name}{e}"))))
        .chain(extra.iter().cloned())
        .find(|p| p.is_file())
}

/// Shorten a (possibly multi-line) message for display.
pub(crate) fn one_line(s: &str, max: usize) -> String {
    let line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > max {
        let mut t: String = line.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn times() {
        let a = parse_time(Some(&json!(1794970200))).unwrap();
        let b = parse_time(Some(&json!(1794970200000u64))).unwrap();
        assert_eq!(a, b);
        let c = parse_time(Some(&json!("2026-11-18T02:49:59.963320+00:00"))).unwrap();
        assert_eq!(c.timestamp(), 1794970199);
        let d = parse_time(Some(&json!("2026-12-01"))).unwrap();
        assert_eq!(d.to_rfc3339(), "2026-12-01T00:00:00+00:00");
        assert!(parse_time(Some(&json!(null))).is_none());
        assert!(parse_time(Some(&json!("garbage"))).is_none());
        assert!(parse_time(Some(&json!(0))).is_none());
        assert!(parse_time(None).is_none());
    }

    #[test]
    fn numbers_and_text() {
        assert_eq!(num(Some(&json!("12.5"))), Some(12.5));
        assert_eq!(num(Some(&json!(null))), None);
        assert_eq!(title_case("seven_day_opus"), "Seven Day Opus");
        assert_eq!(one_line("a\n  b", 10), "a b");
        assert_eq!(one_line("abcdefghijkl", 5), "abcd…");
    }
}
