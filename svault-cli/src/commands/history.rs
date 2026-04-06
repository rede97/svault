use std::collections::HashMap;
use std::path::PathBuf;

use crate::cli::OutputFormat;
use crate::commands::parse_datetime_to_ms;
use svault_core::context::VaultContext;
use svault_core::db::Db;
use console::style;

#[allow(clippy::too_many_arguments)]
pub fn run(
    output: OutputFormat,
    file: Option<PathBuf>,
    from: Option<String>,
    to: Option<String>,
    events: bool,
    limit: usize,
    verbose: bool,
) -> anyhow::Result<()> {
    let ctx = VaultContext::open_cwd()?;

    if !events {
        // Default: Show session-based history (import/add/reconcile)
        show_session_history(output, ctx.vault_root(), ctx.db(), from, to, limit, verbose)?;
    } else {
        // Original event-based history (all events, use grep for filtering)
        show_event_history(
            output, ctx.vault_root(), file, from, to, limit, ctx.db(),
        )?;
    }
    Ok(())
}

fn show_session_history(
    output: OutputFormat,
    _vault_root: &std::path::Path,
    db: &Db,
    from: Option<String>,
    to: Option<String>,
    limit: usize,
    verbose: bool,
) -> anyhow::Result<()> {

    let from_ms = from.as_ref().and_then(|s| parse_datetime_to_ms(s));
    let to_ms = to.as_ref().and_then(|s| parse_datetime_to_ms(s));

    // Query using get_events and filter for import events
    let all_events = db.get_events(
        limit * 2, // Get more events to filter
        None,      // No type filter - we'll filter manually
        from_ms,
        to_ms,
        None,
    )?;

    let sessions: Vec<(i64, String, String)> = all_events
        .into_iter()
        .filter(|e| e.event_type.starts_with("import.") || e.event_type.starts_with("add."))
        .map(|e| (e.occurred_at, e.event_type, e.payload))
        .collect();

    if sessions.is_empty() {
        eprintln!("No import/add/reconcile history found.");
        eprintln!("Use --events to see all events.");
        return Ok(());
    }

    // Group by session_id
    let mut session_map: HashMap<String, (i64, Option<i64>, String)> = HashMap::new();

    for (occurred_at, event_type, payload) in sessions {
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap_or_default();
        let session_id = parsed["session_id"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        match event_type.as_str() {
            "import.pending" => {
                session_map
                    .entry(session_id.clone())
                    .or_insert((occurred_at, None, payload));
            }
            "import.completed" | "add.completed" => {
                let entry = session_map
                    .entry(session_id.clone())
                    .or_insert((occurred_at, None, payload.clone()));
                entry.1 = Some(occurred_at);
                entry.2 = payload;
            }
            _ => {}
        }
    }

    if matches!(output, OutputFormat::Json) {
        let sessions_json: Vec<_> = session_map
            .iter()
            .map(|(session_id, (started_at, completed_at, payload))| {
                let parsed: serde_json::Value =
                    serde_json::from_str(payload).unwrap_or_default();

                serde_json::json!({
                    "session_id": session_id,
                    "started_at": started_at,
                    "completed_at": completed_at,
                    "source": parsed["source"].as_str(),
                    "total_files": parsed["total_files"].as_i64(),
                    "imported": parsed.get("imported").and_then(|v| v.as_i64()),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "sessions": sessions_json
            }))?
        );
    } else {
        println!("{}", style("History (import/add/reconcile)").bold().underlined());
        println!();

        let mut sessions_vec: Vec<_> = session_map.into_iter().collect();
        sessions_vec.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));

        for (i, (session_id, (started_at, completed_at, payload))) in
            sessions_vec.iter().enumerate()
        {
            let datetime = chrono::DateTime::from_timestamp_millis(*started_at).unwrap_or_default();

            let parsed: serde_json::Value = serde_json::from_str(payload).unwrap_or_default();
            let source = parsed["source"].as_str().unwrap_or("unknown");
            let total_files = parsed["total_files"].as_i64().unwrap_or(0);
            let imported = parsed
                .get("imported")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let status_icon = if completed_at.is_some() {
                style("✓").green()
            } else {
                style("⏳").yellow()
            };

            println!(
                "{} {} {} {}",
                style(format!("[{}]", i + 1)).cyan().bold(),
                status_icon,
                style(datetime.format("%Y-%m-%d %H:%M:%S")).bright(),
                style(&session_id[..session_id.len().min(8)]).dim()
            );

            println!("  Source: {}", style(source).blue());
            if completed_at.is_some() {
                println!(
                    "  Status: {} ({} of {} files)",
                    style("completed").green(),
                    style(imported).yellow(),
                    style(total_files).yellow()
                );
            } else {
                println!(
                    "  Status: {}",
                    style("pending (not confirmed)").yellow()
                );
            }

            if verbose {
                // In verbose mode, could show file list from manifest
            }

            println!();
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn show_event_history(
    output: OutputFormat,
    _vault_root: &std::path::Path,
    file: Option<PathBuf>,
    from: Option<String>,
    to: Option<String>,
    limit: usize,
    db: &svault_core::db::Db,
) -> anyhow::Result<()> {
    let from_ms = from.as_ref().and_then(|s| parse_datetime_to_ms(s));
    let to_ms = to.as_ref().and_then(|s| parse_datetime_to_ms(s));
    let file_path = file.as_ref().map(|p| p.to_string_lossy().to_string());

    // No event_type filter - use grep for filtering specific event types
    let events = db.get_events(
        limit,
        None, // event_type filter removed - show all events
        from_ms,
        to_ms,
        file_path.as_deref(),
    )?;

    if matches!(output, OutputFormat::Json) {
        let json = serde_json::json!({
            "events": events.iter().map(|e| serde_json::json!({
                "seq": e.seq,
                "occurred_at": e.occurred_at,
                "event_type": e.event_type,
                "entity_type": e.entity_type,
                "entity_id": e.entity_id,
                "payload": e.payload,
                "prev_hash": e.prev_hash,
                "self_hash": e.self_hash,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        if events.is_empty() {
            eprintln!("No events found.");
            return Ok(());
        }
        println!("{:>6}  {:<22}  {:<20}  payload", "seq", "time", "event");
        for e in events {
            let datetime = chrono::DateTime::from_timestamp_millis(e.occurred_at).unwrap_or_default();
            println!(
                "{:>6}  {:<22}  {:<20}  {}",
                e.seq,
                datetime.format("%Y-%m-%d %H:%M:%S"),
                e.event_type,
                e.payload
            );
        }
    }
    Ok(())
}
