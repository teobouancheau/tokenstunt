pub fn bar(ratio: f64, width: usize) -> String {
    let filled = (ratio * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    let pct = (ratio * 100.0) as u32;
    format!(
        "{}{} {pct}%",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(empty),
    )
}

pub fn bar_with_label(current: u64, total: u64, width: usize) -> String {
    if total == 0 {
        let empty = "\u{2591}".repeat(width);
        return format!("{current}/{total}  {empty} 0%");
    }
    let ratio = current as f64 / total as f64;
    format!("{current}/{total}  {}", bar(ratio, width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bar_full() {
        let b = bar(1.0, 10);
        assert!(b.contains('\u{2588}'));
        assert!(!b.contains('\u{2591}'));
        assert!(b.contains("100%"));
    }

    #[test]
    fn test_bar_empty() {
        let b = bar(0.0, 10);
        assert!(!b.contains('\u{2588}'));
        assert!(b.contains("0%"));
    }

    #[test]
    fn test_bar_half() {
        let b = bar(0.5, 10);
        assert!(b.contains("50%"));
    }

    #[test]
    fn test_bar_with_label() {
        let b = bar_with_label(312, 400, 20);
        assert!(b.contains("312/400"));
        assert!(b.contains("78%"));
    }

    #[test]
    fn test_bar_with_label_zero_total() {
        let b = bar_with_label(0, 0, 10);
        assert!(b.contains("0%"));
    }
}
