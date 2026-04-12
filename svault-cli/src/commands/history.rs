//! History command — query import sessions and items.

use crate::cli::{HistorySubcommand, OutputFormat};
use crate::reporting::{JsonHistoryItemsReporter, JsonHistorySessionsReporter, TerminalReporterBuilder};
use svault_core::context::VaultContext;
use svault_core::history::{query_items, query_sessions};
use svault_core::reporting::{HistoryItemsQuery, HistorySessionsQuery, ReporterBuilder};
use chrono::TimeZone;

/// Parse RFC 3339 or YYYY-MM-DD string to milliseconds timestamp.
fn parse_datetime_to_ms(s: &str) -> Option<i64> {
    // Try RFC 3339 first
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    // Try YYYY-MM-DD (treat as start of day in local timezone)
    if let Ok(naive) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let _local = chrono::Local::now();
        let datetime = naive.and_hms_opt(0, 0, 0)?;
        let local_dt = chrono::Local.from_local_datetime(&datetime).single()?;
        return Some(local_dt.timestamp_millis());
    }
    None
}

/// Run the history command.
pub fn run(
    output: OutputFormat,
    subcommand: Option<HistorySubcommand>,
) -> anyhow::Result<()> {
    let ctx = VaultContext::open_cwd()?;

    match subcommand {
        Some(HistorySubcommand::Sessions { source, from, to, limit, offset }) => {
            run_sessions(output, ctx, source, from, to, limit, offset)
        }
        Some(HistorySubcommand::Items { session, status, limit, offset }) => {
            run_items(output, ctx, &session, status, limit, offset)
        }
        None => {
            // Default: show sessions with default parameters
            run_sessions(output, ctx, None, None, None, 50, 0)
        }
    }
}

fn run_sessions(
    output: OutputFormat,
    ctx: VaultContext,
    source: Option<String>,
    from: Option<String>,
    to: Option<String>,
    limit: usize,
    offset: usize,
) -> anyhow::Result<()> {
    let query = HistorySessionsQuery {
        limit,
        offset,
        source,
        from_ms: from.as_ref().and_then(|s| parse_datetime_to_ms(s)),
        to_ms: to.as_ref().and_then(|s| parse_datetime_to_ms(s)),
    };

    match output {
        OutputFormat::Human => {
            let builder = TerminalReporterBuilder::new();
            let reporter = builder.history_sessions_reporter(&query);
            query_sessions(ctx.db(), &query, &reporter)?;
        }
        OutputFormat::Json => {
            let reporter = JsonHistorySessionsReporter;
            query_sessions(ctx.db(), &query, &reporter)?;
        }
    }

    Ok(())
}

fn run_items(
    output: OutputFormat,
    ctx: VaultContext,
    session: &str,
    status: Option<String>,
    limit: usize,
    offset: usize,
) -> anyhow::Result<()> {
    let query = HistoryItemsQuery {
        limit,
        offset,
        status,
    };

    match output {
        OutputFormat::Human => {
            let builder = TerminalReporterBuilder::new();
            let reporter = builder.history_items_reporter(session, &query);
            query_items(ctx.db(), ctx.vault_root(), session, &query, &reporter)?;
        }
        OutputFormat::Json => {
            let reporter = JsonHistoryItemsReporter;
            query_items(ctx.db(), ctx.vault_root(), session, &query, &reporter)?;
        }
    }

    Ok(())
}
