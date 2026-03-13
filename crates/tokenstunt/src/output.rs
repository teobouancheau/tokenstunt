use std::sync::Mutex;

use console::Style;
use indicatif::{ProgressBar, ProgressStyle};
use tokenstunt_index::IndexProgress;

const ORANGE_256: u8 = 173;
const LABEL_WIDTH: usize = 12;

fn accent() -> Style {
    Style::new().color256(ORANGE_256)
}

fn dim() -> Style {
    Style::new().dim()
}

fn bold() -> Style {
    Style::new().white().bold()
}

pub struct IndicatifProgress {
    bar: Mutex<Option<ProgressBar>>,
}

impl IndicatifProgress {
    pub fn new() -> Self {
        Self {
            bar: Mutex::new(None),
        }
    }
}

impl IndexProgress for IndicatifProgress {
    fn on_discover(&self, total_files: usize) {
        let pb = ProgressBar::new(total_files as u64);
        pb.set_style(
            ProgressStyle::with_template(&format!(
                " {}  [{{bar:24.{}}}]  {{pos}}/{{len}} files   {{msg}}",
                accent().apply_to("Indexing"),
                ORANGE_256,
            ))
            .unwrap()
            .progress_chars("##-"),
        );
        if let Ok(mut lock) = self.bar.lock() {
            *lock = Some(pb);
        }
    }

    fn on_file_indexed(&self, path: &str) {
        if let Ok(lock) = self.bar.lock()
            && let Some(pb) = lock.as_ref()
        {
            pb.set_message(truncate_path(path, 40));
            pb.inc(1);
        }
    }

    fn on_file_skipped(&self, _path: &str) {
        if let Ok(lock) = self.bar.lock()
            && let Some(pb) = lock.as_ref()
        {
            pb.inc(1);
        }
    }

    fn on_file_error(&self, _path: &str, _error: &str) {
        if let Ok(lock) = self.bar.lock()
            && let Some(pb) = lock.as_ref()
        {
            pb.inc(1);
        }
    }

    fn on_complete(&self, _files: u64, _blocks: u64, _skipped: u64, _errors: u64) {
        if let Ok(lock) = self.bar.lock()
            && let Some(pb) = lock.as_ref()
        {
            pb.finish_and_clear();
        }
    }
}

pub fn print_index_summary(files: u64, blocks: u64, skipped: u64, deleted_files: u64, errors: u64) {
    let a = accent();
    let b = bold();

    eprintln!(
        "  {:>LABEL_WIDTH$}  {} files, {} code blocks",
        a.apply_to("Indexed"),
        b.apply_to(format_number(files)),
        b.apply_to(format_number(blocks)),
    );

    if skipped > 0 {
        eprintln!(
            "  {:>LABEL_WIDTH$}  {} unchanged",
            dim().apply_to("Skipped"),
            dim().apply_to(format_number(skipped)),
        );
    }

    if deleted_files > 0 {
        let warn = Style::new().yellow();
        eprintln!(
            "  {:>LABEL_WIDTH$}  {} stale files removed",
            warn.apply_to("Cleaned"),
            warn.apply_to(format_number(deleted_files)),
        );
    }

    if errors > 0 {
        let err = Style::new().red();
        eprintln!(
            "  {:>LABEL_WIDTH$}  {} files failed",
            err.apply_to("Errors"),
            err.apply_to(format_number(errors)),
        );
    }
}

pub fn print_embed_summary(emb_count: u64, block_count: u64) {
    let a = accent();
    let b = bold();
    let pct = if block_count > 0 {
        (emb_count as f64 / block_count as f64 * 100.0) as u32
    } else {
        0
    };
    eprintln!(
        "  {:>LABEL_WIDTH$}  {}/{} vectors ({}%)",
        a.apply_to("Embeddings"),
        b.apply_to(format_number(emb_count)),
        format_number(block_count),
        pct,
    );
}

pub fn print_status(db_path: &std::path::Path, files: u64, blocks: u64) {
    let a = accent();
    let b = bold();
    let d = dim();

    let project_name = db_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    eprintln!(
        "  {:>LABEL_WIDTH$}  {}",
        a.apply_to("Index"),
        b.apply_to(project_name),
    );
    eprintln!(
        "  {:>LABEL_WIDTH$}  {}",
        d.apply_to("Path"),
        d.apply_to(db_path.display()),
    );
    eprintln!(
        "  {:>LABEL_WIDTH$}  {} indexed",
        a.apply_to("Files"),
        b.apply_to(format_number(files)),
    );
    eprintln!(
        "  {:>LABEL_WIDTH$}  {}",
        a.apply_to("Code Blocks"),
        b.apply_to(format_number(blocks)),
    );
}

pub fn print_serve_banner(root: &std::path::Path, files: u64, blocks: u64, watcher_active: bool) {
    let a = accent();
    let b = bold();
    let d = dim();
    let g = Style::new().green();

    eprintln!(
        "  {} v{}",
        a.apply_to("Token Stunt").bold(),
        env!("CARGO_PKG_VERSION"),
    );
    eprintln!(
        "  {:>LABEL_WIDTH$}  {}",
        a.apply_to("Root"),
        d.apply_to(root.display()),
    );
    eprintln!(
        "  {:>LABEL_WIDTH$}  {} files, {} code blocks",
        a.apply_to("Index"),
        b.apply_to(format_number(files)),
        b.apply_to(format_number(blocks)),
    );
    if watcher_active {
        eprintln!(
            "  {:>LABEL_WIDTH$}  {}",
            a.apply_to("Watcher"),
            g.apply_to("active"),
        );
    }
    eprintln!(
        "  {:>LABEL_WIDTH$}  {}",
        a.apply_to("MCP"),
        g.apply_to("Ready on stdio"),
    );
    eprintln!();
}

fn truncate_path(path: &str, max_len: usize) -> String {
    let char_count = path.chars().count();
    if char_count <= max_len {
        return path.to_string();
    }
    let keep = max_len.saturating_sub(3);
    let skip = char_count - keep;
    let suffix: String = path.chars().skip(skip).collect();
    format!("...{suffix}")
}

fn format_number(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(42), "42");
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1_000), "1,000");
        assert_eq!(format_number(1_234_567), "1,234,567");
    }
}
