//! Database dump utilities for debugging.

use std::collections::HashMap;

use rusqlite::{Connection, Result, types::Value};

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

/// Formats a SQL value for CSV output.
fn format_value_csv(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => f.to_string(),
        Value::Text(s) => {
            // Escape quotes and wrap in quotes if contains comma, quote, or newline
            let needs_quote = s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r');
            let escaped = s.replace('"', "\"\"");
            if needs_quote || escaped != *s {
                format!("\"{}\"", escaped)
            } else {
                escaped
            }
        }
        Value::Blob(b) => format!("<BLOB:{}>", b.len()),
    }
}

/// Renders dump as CSV (one table per section with header).
pub fn render_csv(dumps: &[TableDump]) -> anyhow::Result<String> {
    let mut output = String::new();
    
    for (i, dump) in dumps.iter().enumerate() {
        if i > 0 {
            output.push('\n');
        }
        
        // Table header comment
        output.push_str(&format!("# Table: {} ({} rows)\n", dump.name, dump.rows.len()));
        
        if dump.rows.is_empty() {
            // Still output column headers for empty tables
            output.push_str(&dump.columns.join(","));
            output.push('\n');
            continue;
        }
        
        // Column headers
        output.push_str(&dump.columns.join(","));
        output.push('\n');
        
        // Data rows
        for row in &dump.rows {
            let row_values: Vec<String> = dump.columns.iter()
                .map(|col| {
                    let val = row.get(col).unwrap_or(&Value::Null);
                    format_value_csv(val)
                })
                .collect();
            output.push_str(&row_values.join(","));
            output.push('\n');
        }
    }
    
    Ok(output)
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
            "row_count": dump.rows.len(),
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
            output.push_str(&format!("-- Table: {} (empty)\n\n", dump.name));
            continue;
        }
        
        output.push_str(&format!("-- Table: {} ({} rows)\n", dump.name, dump.rows.len()));
        
        for row in &dump.rows {
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
                dump.columns.join(", "),
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
    fn test_format_value_csv() {
        assert_eq!(format_value_csv(&Value::Null), "");
        assert_eq!(format_value_csv(&Value::Integer(42)), "42");
        assert_eq!(format_value_csv(&Value::Text("hello".to_string())), "hello");
        assert_eq!(format_value_csv(&Value::Text("with,comma".to_string())), "\"with,comma\"");
        assert_eq!(format_value_csv(&Value::Text("with\"quote".to_string())), "\"with\"\"quote\"");
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

    #[test]
    fn test_render_csv_empty() {
        let dump = TableDump {
            name: "test".to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            rows: vec![],
        };
        let result = render_csv(&[dump]).unwrap();
        assert!(result.contains("# Table: test (0 rows)"));
        assert!(result.contains("id,name"));
    }

    #[test]
    fn test_render_csv_with_data() {
        let mut row = HashMap::new();
        row.insert("id".to_string(), Value::Integer(1));
        row.insert("name".to_string(), Value::Text("test".to_string()));
        
        let dump = TableDump {
            name: "test".to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            rows: vec![row],
        };
        let result = render_csv(&[dump]).unwrap();
        assert!(result.contains("id,name"));
        assert!(result.contains("1,test"));
    }
}
