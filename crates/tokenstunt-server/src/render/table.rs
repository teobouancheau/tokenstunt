#[allow(dead_code)]
pub struct Row {
    pub columns: Vec<String>,
}

#[allow(dead_code)]
pub fn render_rows(rows: &[Row], padding: usize) -> String {
    if rows.is_empty() {
        return String::new();
    }

    let col_count = rows.iter().map(|r| r.columns.len()).max().unwrap_or(0);
    let mut widths = vec![0usize; col_count];

    for row in rows {
        for (i, col) in row.columns.iter().enumerate() {
            widths[i] = widths[i].max(col.chars().count());
        }
    }

    let mut out = String::new();
    for row in rows {
        out.push_str(&" ".repeat(padding));
        for (i, col) in row.columns.iter().enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            let w = widths.get(i).copied().unwrap_or(0);
            let pad = w.saturating_sub(col.chars().count());
            out.push_str(col);
            out.push_str(&" ".repeat(pad));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        assert_eq!(render_rows(&[], 0), "");
    }

    #[test]
    fn test_aligned_columns() {
        let rows = vec![
            Row {
                columns: vec!["Function".to_string(), "greet".to_string(), "src/main.ts".to_string()],
            },
            Row {
                columns: vec!["Class".to_string(), "UserProfile".to_string(), "src/auth.ts".to_string()],
            },
        ];
        let out = render_rows(&rows, 2);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("  Function"));
        assert!(lines[1].starts_with("  Class"));
    }

    #[test]
    fn test_padding() {
        let rows = vec![Row {
            columns: vec!["a".to_string()],
        }];
        let out = render_rows(&rows, 4);
        assert!(out.starts_with("    a"));
    }
}
