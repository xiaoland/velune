//! Read-only CLI login usage shared by provider and native runtime sources.
use std::path::Path;
use velune_gateway::QuotaWindow;

pub(crate) async fn read(
    binary: &str,
    directory: Option<&Path>,
    ttl_ms: i64,
) -> Result<Vec<QuotaWindow>, ()> {
    let text = crate::claude_cli::usage(binary, directory).await?;
    let observed = crate::local::unix_ms_now();
    if observed <= 0 {
        return Err(());
    }
    let mut windows = Vec::new();
    for line in text.lines().map(str::trim) {
        let (id, period, body) = if let Some(body) = line.strip_prefix("Current session:") {
            ("window:18000", 18_000_000, body)
        } else if let Some(body) = line
            .strip_prefix("Current week (all models):")
            .or_else(|| line.strip_prefix("Current week:"))
        {
            ("window:604800", 604_800_000, body)
        } else {
            continue;
        };
        let (used, tail) = body.trim().split_once("% used").ok_or(())?;
        if !tail.is_empty() && !tail.trim_start().starts_with('·') {
            return Err(());
        }
        let used = used.parse::<f64>().map_err(|_| ())?;
        if !used.is_finite()
            || !(0.0..=100.0).contains(&used)
            || windows.iter().any(|window: &QuotaWindow| window.id == id)
        {
            return Err(());
        }
        // CLI reset prose is not a timestamp contract; expiry requires a new observation.
        windows.push(QuotaWindow {
            id: id.into(),
            model_ids: None,
            used_percent: used,
            reset_at_ms: None,
            window_ms: Some(period),
            observed_at_ms: observed,
            expires_at_ms: observed + ttl_ms,
        });
    }
    if windows.is_empty() {
        return Err(());
    }
    Ok(windows)
}
