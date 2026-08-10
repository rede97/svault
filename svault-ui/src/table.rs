//! Shared table renderer (rich_rust) used by all human-readable outputs.

use rich_rust::r#box::BoxChars;
use rich_rust::prelude::*;
use rich_rust::renderables::Renderable;

/// Custom box style: only heavy header separator (continuous), no vertical dividers.
const CLEAN_STYLE: BoxChars = BoxChars::new(
    [' ', ' ', ' ', ' '], // No top border
    [' ', ' ', ' ', ' '], // No vertical dividers for body
    [' ', '━', '━', ' '], // Heavy continuous line for header separator
    [' ', ' ', ' ', ' '], // No mid separator
    [' ', ' ', ' ', ' '], // No row separators
    [' ', ' ', ' ', ' '], // No foot row separator
    [' ', ' ', ' ', ' '], // No footer vertical dividers
    [' ', ' ', ' ', ' '], // No bottom border
    false,
);

/// Helper to convert renderable to string.
fn render_to_string<R: Renderable>(renderable: &R) -> String {
    let console = Console::new();
    let options = console.options();
    let segments = renderable.render(&console, &options);

    segments
        .into_iter()
        .map(|seg| seg.text.into_owned())
        .collect::<Vec<_>>()
        .join("")
}

/// Render a titled table. The first column is left-justified, the rest right.
pub fn render_table(title: &str, columns: &[&str], rows: &[Vec<String>]) -> String {
    let mut t = Table::new()
        .title(title)
        .title_justify(JustifyMethod::Left)
        .box_style(&CLEAN_STYLE)
        .min_width(40);
    for (i, col) in columns.iter().enumerate() {
        let c = if i == 0 {
            Column::new(*col)
        } else {
            Column::new(*col).justify(JustifyMethod::Right)
        };
        t = t.with_column(c);
    }
    for row in rows {
        t.add_row_cells(row.iter().map(String::as_str));
    }
    render_to_string(&t)
}
