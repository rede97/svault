//! `svault album` command wiring: parse args, call core, render output.

use svault_core::context::VaultContext;
use svault_core::ops::album::{self, AlbumChange, AlbumNode};

use crate::cli::{AlbumCommand, OutputFormat};

pub fn run(output: OutputFormat, command: AlbumCommand) -> anyhow::Result<()> {
    let ctx = VaultContext::open_cwd()?;
    let db = ctx.db();
    let vault_root = ctx.vault_root();

    match command {
        AlbumCommand::Create { path } => {
            let result = album::create(db, &path)?;
            if matches!(output, OutputFormat::Json) {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if result.created.is_empty() {
                println!("Album already exists: {}", result.path);
            } else {
                println!("✓ Album created: {}", result.path);
            }
        }
        AlbumCommand::List { pattern } => {
            let tree = album::list(db, pattern.as_deref())?;
            if matches!(output, OutputFormat::Json) {
                println!("{}", serde_json::to_string_pretty(&tree)?);
            } else if tree.is_empty() {
                println!("No albums yet. Create one with: svault album create <path>");
            } else {
                let mut rows = Vec::new();
                flatten_tree(&tree, 0, &mut rows);
                print!(
                    "{}",
                    svault_ui::table::render_table("📚 Albums", &["Album", "Members"], &rows)
                );
            }
        }
        AlbumCommand::Show { path } => {
            let result = album::show(db, &path)?;
            if matches!(output, OutputFormat::Json) {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for detail in &result.matched {
                    let rows: Vec<Vec<String>> = detail
                        .members
                        .iter()
                        .map(|m| {
                            vec![
                                m.path.clone(),
                                m.rating
                                    .map(|r| format!("{}★", r))
                                    .unwrap_or_else(|| "-".to_string()),
                            ]
                        })
                        .collect();
                    print!(
                        "{}",
                        svault_ui::table::render_table(
                            &format!("🖼  {} ({} members)", detail.path, detail.members.len()),
                            &["Path", "Rating"],
                            &rows,
                        )
                    );
                }
            }
        }
        AlbumCommand::Add { album: path, paths } => {
            let change = album::add(db, vault_root, &path, &paths)?;
            print_change("added to", &change, &output);
        }
        AlbumCommand::Remove { album: path, paths } => {
            let change = album::remove(db, vault_root, &path, &paths)?;
            print_change("removed from", &change, &output);
        }
        AlbumCommand::Rate {
            album: path,
            rating,
            paths,
        } => {
            let rating = if rating == 0 { None } else { Some(rating) };
            let change = album::rate(db, vault_root, &path, rating, &paths)?;
            print_change("rated in", &change, &output);
        }
        AlbumCommand::Delete { path } => {
            album::delete(db, &path)?;
            if matches!(output, OutputFormat::Json) {
                println!("{}", serde_json::json!({"deleted": path}));
            } else {
                println!("✓ Album deleted: {path}");
            }
        }
    }
    Ok(())
}

fn flatten_tree(nodes: &[AlbumNode], depth: usize, rows: &mut Vec<Vec<String>>) {
    for node in nodes {
        rows.push(vec![
            format!("{}{}", "  ".repeat(depth), node.name),
            node.member_count.to_string(),
        ]);
        flatten_tree(&node.children, depth + 1, rows);
    }
}

fn print_change(verb: &str, change: &AlbumChange, output: &OutputFormat) {
    if matches!(output, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::to_string_pretty(change).expect("AlbumChange serializes")
        );
        return;
    }
    println!(
        "✓ {} file(s) {verb} album '{}'",
        change.affected.len(),
        change.album
    );
    for skip in &change.skipped {
        println!("  skipped: {} ({})", skip.path, skip.reason);
    }
}
