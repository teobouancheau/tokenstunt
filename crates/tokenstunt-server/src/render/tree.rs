pub struct TreeItem {
    pub label: String,
}

pub fn render_list(items: &[TreeItem]) -> String {
    let mut out = String::new();
    let last = items.len().saturating_sub(1);
    for (i, item) in items.iter().enumerate() {
        let connector = if i == last {
            "\u{2514}\u{2500}"
        } else {
            "\u{251C}\u{2500}"
        };
        out.push_str(&format!("  {connector} {}\n", item.label));
    }
    out
}

pub fn render_tree_with_trunk(title: &str, items: &[TreeItem]) -> String {
    let mut out = format!("  \u{25C7} {title}\n  \u{2502}\n");
    out.push_str(&render_list(items));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_item() {
        let items = vec![TreeItem {
            label: "one".to_string(),
        }];
        let out = render_list(&items);
        assert!(out.contains("\u{2514}\u{2500}"));
        assert!(out.contains("one"));
    }

    #[test]
    fn test_multiple_items() {
        let items = vec![
            TreeItem {
                label: "first".to_string(),
            },
            TreeItem {
                label: "second".to_string(),
            },
            TreeItem {
                label: "third".to_string(),
            },
        ];
        let out = render_list(&items);
        assert!(out.contains("\u{251C}\u{2500} first"));
        assert!(out.contains("\u{251C}\u{2500} second"));
        assert!(out.contains("\u{2514}\u{2500} third"));
    }

    #[test]
    fn test_tree_with_trunk() {
        let items = vec![TreeItem {
            label: "item".to_string(),
        }];
        let out = render_tree_with_trunk("Section", &items);
        assert!(out.contains("\u{25C7} Section"));
        assert!(out.contains("\u{2502}"));
        assert!(out.contains("item"));
    }
}
