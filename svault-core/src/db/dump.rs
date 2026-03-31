//! Database dump utilities for debugging.

use rusqlite::{Connection, Result, types::Value};
use std::collections::HashMap;
use tabled::{Table, settings::{Style, Width, Modify, object::Columns, Alignment}};

/// A row of data from a database table.
pub type RowData = HashMap<String, Value>;

/// Dump of a single table.
#[derive(Debug, Clone)]
pub struct TableDump {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: Vec<RowData>,
}

/// Options for dumping database contents.
#[derive(Debug, Clone)]
pub struct DumpOptions {
    /// Specific tables to dump (empty = all tables).
    pub tables: Vec<String>,
    /// Maximum rows per table.
    pub limit: Option<usize>,
}

impl Default for DumpOptions {
    fn default() -> Self {
        Self {
            tables: Vec::new(),
            limit: None,
        }
    }
}

/// Returns a list of all user tables in the database.
pub fn list_tables(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master 
         WHERE type = 'table' 
         AND name NOT LIKE 'sqlite_%'
         ORDER BY name"
    )?;
    
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut tables = Vec::new();
    for table in rows {
        tables.push(table?);
    }
    Ok(tables)
}

/// Dumps a single table's contents.
pub fn dump_table(conn: &Connection, table_name: &str, limit: Option<usize>) -> Result<TableDump> {
    // Get column names
    let pragma_sql = format!("PRAGMA table_info({})", table_name);
    let mut stmt = conn.prepare(&pragma_sql)?;
    let column_rows = stmt.query_map([], |row| {
        Ok(row.get::<_, String>(1)?) // column name is at index 1
    })?;
    
    let mut columns = Vec::new();
    for col in column_rows {
        columns.push(col?);
    }
    
    // Get table data
    let sql = match limit {
        Some(n) => format!("SELECT * FROM {} LIMIT {}", table_name, n),
        None => format!("SELECT * FROM {}", table_name),
    };
    
    let mut stmt = conn.prepare(&sql)?;
    
    let rows = stmt.query_map([], |row| {
        let mut row_data = HashMap::new();
        for (i, col_name) in columns.iter().enumerate() {
            let value = row.get::<_, Value>(i)?;
            row_data.insert(col_name.clone(), value);
        }
        Ok(row_data)
    })?;
    
    let mut row_vec = Vec::new();
    for row in rows {
        row_vec.push(row?);
    }
    
    Ok(TableDump {
        name: table_name.to_string(),
        columns,
        rows: row_vec,
    })
}

/// Dumps all or selected tables from the database.
pub fn dump_database(conn: &Connection, opts: DumpOptions) -> Result<Vec<TableDump>> {
    let tables_to_dump = if opts.tables.is_empty() {
        list_tables(conn)?
    } else {
        opts.tables.clone()
    };
    
    let mut dumps = Vec::new();
    for table_name in tables_to_dump {
        match dump_table(conn, &table_name, opts.limit) {
            Ok(dump) => dumps.push(dump),
            Err(e) => eprintln!("Warning: failed to dump table '{}': {}", table_name, e),
        }
    }
    
    Ok(dumps)
}

/// Formats a SQL value for display.
pub fn format_value(value: &Value, max_len: usize) -> String {
    match value {
        Value::Null => "∅".to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => format!("{:.6}", f),
        Value::Text(s) => {
            if s.len() > max_len {
                format!("{}…", &s[..max_len.saturating_sub(1)])
            } else {
                s.clone()
            }
        }
        Value::Blob(b) => format!("⟨BLOB:{}⟩", b.len()),
    }
}

/// Converts a table dump to a tabled Table.
fn dump_to_tabled(dump: &TableDump, max_col_width: usize) -> Table {
    // Build data: header + rows
    let mut data: Vec<Vec<String>> = Vec::new();
    data.push(dump.columns.clone());
    
    // Build rows
    for row in &dump.rows {
        let row_values: Vec<String> = dump.columns.iter()
            .map(|col| {
                let val = row.get(col).unwrap_or(&Value::Null);
                format_value(val, max_col_width)
            })
            .collect();
        data.push(row_values);
    }
    
    let mut table = Table::from_iter(&data);
    
    // Style: modern rounded borders
    table.with(Style::modern_rounded());
    
    // Left-align all columns
    table.with(Modify::new(Columns::new(..)).with(Alignment::left()));
    
    // Set max width for columns to prevent overflow
    table.with(Modify::new(Columns::new(..)).with(Width::truncate(max_col_width).suffix("…")));
    
    table
}

/// Renders table dump as human-readable text using tabled.
pub fn render_table(dump: &TableDump) -> String {
    if dump.rows.is_empty() {
        return format!("\n📋 {} (0 rows)\n   (empty table)\n", dump.name);
    }
    
    let table = dump_to_tabled(dump, 40);
    
    format!("\n📋 {} ({} rows)\n{}\n", dump.name, dump.rows.len(), table)
}

/// Renders all tables as human-readable text.
pub fn render_tables(dumps: &[TableDump]) -> String {
    let mut output = String::new();
    output.push_str("╔══════════════════════════════════════════════════════════════════════════════╗\n");
    output.push_str("║                      📊 Svault Database Dump                                 ║\n");
    output.push_str("╚══════════════════════════════════════════════════════════════════════════════╝\n");
    
    for dump in dumps {
        output.push_str(&render_table(dump));
    }
    
    output
}

/// Renders dump as JSON.
pub fn render_json(dumps: &[TableDump]) -> anyhow::Result<String> {
    let json_tables: Vec<serde_json::Value> = dumps.iter().map(|dump| {
        let rows: Vec<serde_json::Value> = dump.rows.iter().map(|row| {
            let mut obj = serde_json::Map::new();
            for col in &dump.columns {
                let val = row.get(col).unwrap_or(&Value::Null);
                let json_val = match val {
                    Value::Null => serde_json::Value::Null,
                    Value::Integer(i) => serde_json::Value::Number((*i).into()),
                    Value::Real(f) => serde_json::Value::Number(
                        serde_json::Number::from_f64(*f).unwrap_or(0.into())
                    ),
                    Value::Text(s) => serde_json::Value::String(s.clone()),
                    Value::Blob(b) => serde_json::Value::String(format!("<BLOB:{}>", b.len())),
                };
                obj.insert(col.clone(), json_val);
            }
            serde_json::Value::Object(obj)
        }).collect();
        
        serde_json::json!({
            "name": dump.name,
            "columns": dump.columns,
            "rows": rows,
        })
    }).collect();
    
    Ok(serde_json::to_string_pretty(&json_tables)?)
}

/// Renders dump as SQL INSERT statements.
pub fn render_sql(dumps: &[TableDump]) -> String {
    let mut output = String::new();
    output.push_str("-- Svault Database Dump\n");
    output.push_str("-- Generated automatically\n\n");
    
    for dump in dumps {
        if dump.rows.is_empty() {
            continue;
        }
        
        output.push_str(&format!("-- Table: {}\n", dump.name));
        
        for row in &dump.rows {
            let columns: Vec<String> = dump.columns.clone();
            let values: Vec<String> = dump.columns.iter().map(|col| {
                match row.get(col).unwrap_or(&Value::Null) {
                    Value::Null => "NULL".to_string(),
                    Value::Integer(i) => i.to_string(),
                    Value::Real(f) => f.to_string(),
                    Value::Text(s) => format!("'{}'", s.replace('\'', "''")),
                    Value::Blob(_) => "X'...'".to_string(), // Simplified
                }
            }).collect();
            
            output.push_str(&format!(
                "INSERT INTO {} ({}) VALUES ({});\n",
                dump.name,
                columns.join(", "),
                values.join(", ")
            ));
        }
        output.push('\n');
    }
    
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_value() {
        assert_eq!(format_value(&Value::Null, 20), "∅");
        assert_eq!(format_value(&Value::Integer(42), 20), "42");
        assert_eq!(format_value(&Value::Text("hello".to_string()), 20), "hello");
        assert_eq!(format_value(&Value::Text("a very long string that exceeds limit".to_string()), 10), "a very lon…");
    }

    #[test]
    fn test_list_tables_empty_db() {
        let conn = Connection::open_in_memory().unwrap();
        let tables = list_tables(&conn).unwrap();
        assert!(tables.is_empty());
    }

    #[test]
    fn test_list_tables_with_data() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)", []).unwrap();
        conn.execute("INSERT INTO test VALUES (1, 'hello')", []).unwrap();
        
        let tables = list_tables(&conn).unwrap();
        assert_eq!(tables, vec!["test"]);
    }
}
