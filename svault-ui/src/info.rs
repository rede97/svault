//! `svault info` rendering (human tables and JSON).

use svault_core::db::format_bytes;
use svault_core::ops::info::InfoReport;

use crate::table::render_table;

/// Render an info report as human-readable tables.
pub fn render_human(report: &InfoReport) -> String {
    let mut output = String::new();

    // File section
    let mut rows = vec![
        vec!["Path".to_string(), report.path.display().to_string()],
        vec![
            "On disk".to_string(),
            if report.on_disk { "yes" } else { "no" }.to_string(),
        ],
    ];
    if let Some(db) = &report.db {
        rows.push(vec!["DB path".to_string(), db.path.clone()]);
        rows.push(vec!["Size".to_string(), format_bytes(db.size)]);
        rows.push(vec!["Status".to_string(), db.status.clone()]);
        if let Some(h) = &db.xxh3_128 {
            rows.push(vec!["XXH3-128".to_string(), h.clone()]);
        }
        if let Some(h) = &db.sha256 {
            rows.push(vec!["SHA-256".to_string(), h.clone()]);
        }
    } else {
        rows.push(vec![
            "Tracked".to_string(),
            "no (not in vault DB)".to_string(),
        ]);
    }
    output.push_str(&render_table("📄 File", &["Field", "Value"], &rows));
    output.push('\n');

    // EXIF section
    if !report.exif.is_empty() {
        let rows: Vec<Vec<String>> = report
            .exif
            .iter()
            .map(|(tag, value)| vec![tag.clone(), value.clone()])
            .collect();
        output.push_str(&render_table(
            &format!("📷 EXIF ({} fields)", report.exif.len()),
            &["Tag", "Value"],
            &rows,
        ));
        output.push('\n');
    }

    // Video section (ffprobe)
    if let Some(video) = &report.video {
        let mut rows: Vec<Vec<String>> = Vec::new();
        if let Some(format) = video.get("format") {
            if let Some(Ok(secs)) = format
                .get("duration")
                .and_then(|v| v.as_str())
                .map(str::parse::<f64>)
            {
                rows.push(vec!["Duration".to_string(), format!("{secs:.2} s")]);
            }
            if let Some(Ok(bps)) = format
                .get("bit_rate")
                .and_then(|v| v.as_str())
                .map(str::parse::<i64>)
            {
                rows.push(vec!["Bit rate".to_string(), format_bytes(bps / 8) + "/s"]);
            }
            if let Some(tags) = format.get("tags").and_then(|t| t.as_object()) {
                for (k, v) in tags {
                    rows.push(vec![
                        format!("tag:{k}"),
                        v.as_str().unwrap_or_default().to_string(),
                    ]);
                }
            }
        }
        if let Some(streams) = video.get("streams").and_then(|s| s.as_array()) {
            for stream in streams {
                let kind = stream
                    .get("codec_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let codec = stream
                    .get("codec_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let mut desc = codec.to_string();
                if let (Some(w), Some(h)) = (
                    stream.get("width").and_then(|v| v.as_i64()),
                    stream.get("height").and_then(|v| v.as_i64()),
                ) {
                    desc.push_str(&format!(" {w}x{h}"));
                }
                rows.push(vec![format!("stream:{kind}"), desc]);
            }
        }
        if !rows.is_empty() {
            output.push_str(&render_table(
                "🎬 Video (ffprobe)",
                &["Field", "Value"],
                &rows,
            ));
            output.push('\n');
        }
    } else if !report.ffprobe_available {
        output.push_str("⚠ ffprobe not found — video metadata unavailable\n\n");
    }

    output
}

/// Render an info report as pretty JSON.
pub fn render_json(report: &InfoReport) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(report)?)
}
