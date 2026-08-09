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
        AlbumCommand::List => {
            let tree = album::list(db)?;
            if matches!(output, OutputFormat::Json) {
                println!("{}", serde_json::to_string_pretty(&tree)?);
            } else if tree.is_empty() {
                println!("No albums yet. Create one with: svault album create <path>");
            } else {
                print_tree(&tree, 0);
            }
        }
        AlbumCommand::Show { path } => {
            let detail = album::show(db, &path)?;
            if matches!(output, OutputFormat::Json) {
                println!("{}", serde_json::to_string_pretty(&detail)?);
            } else {
                println!(
                    "Album: {} ({} member(s))",
                    detail.path,
                    detail.members.len()
                );
                for m in &detail.members {
                    let rating = m
                        .rating
                        .map(|r| format!("{}★", r))
                        .unwrap_or_else(|| "-".to_string());
                    println!("  {:>3}  {}", rating, m.path);
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

fn print_tree(nodes: &[AlbumNode], depth: usize) {
    for node in nodes {
        println!(
            "{}{} ({})",
            "  ".repeat(depth),
            node.name,
            node.member_count
        );
        print_tree(&node.children, depth + 1);
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
