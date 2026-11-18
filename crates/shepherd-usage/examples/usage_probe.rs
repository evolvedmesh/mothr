//! Print account limits for every configured provider (no secrets).
//!
//! `cargo run -p shepherd-usage --example usage_probe`

use shepherd_core::format_reset;
use shepherd_usage::UsageFetcher;

#[tokio::main]
async fn main() {
    let fetcher = UsageFetcher::new();
    let all = fetcher.fetch_all().await;
    if all.is_empty() {
        println!("No providers configured on this machine.");
    }
    for u in all {
        println!(
            "{} — plan: {} — source: {} — updated {}",
            u.provider.display_name(),
            u.plan.as_deref().unwrap_or("?"),
            u.source,
            u.updated_at.to_rfc3339(),
        );
        if let Some(e) = &u.error {
            println!("    error: {e}");
        }
        for w in &u.windows {
            let abs = match (w.used, w.limit) {
                (Some(used), Some(limit)) => format!(" ({used:.0}/{limit:.0})"),
                _ => String::new(),
            };
            let reset = w
                .resets_at
                .map(|r| format!(", resets {} ({})", format_reset(r), r.to_rfc3339()))
                .unwrap_or_default();
            let detail = w.detail.as_deref().map(|d| format!(" [{d}]")).unwrap_or_default();
            println!(
                "    {:<14} {:?} {:>5.1}%{abs}{reset}{detail}  level={:?}",
                w.label,
                w.kind,
                w.used_fraction * 100.0,
                w.level()
            );
        }
    }
}
