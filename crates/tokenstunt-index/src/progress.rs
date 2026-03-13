pub trait IndexProgress: Send + Sync {
    fn on_discover(&self, total_files: usize);
    fn on_file_indexed(&self, path: &str);
    fn on_file_skipped(&self, path: &str);
    fn on_file_error(&self, path: &str, error: &str);
    fn on_complete(&self, files: u64, blocks: u64, skipped: u64, errors: u64);
}

pub trait EmbeddingProgress: Send + Sync {
    fn on_start(&self, total_blocks: u64);
    fn on_batch_complete(&self, batch_size: u64);
    fn on_complete(&self, total: u64);
}

pub struct NopProgress;

impl IndexProgress for NopProgress {
    fn on_discover(&self, _total_files: usize) {}
    fn on_file_indexed(&self, _path: &str) {}
    fn on_file_skipped(&self, _path: &str) {}
    fn on_file_error(&self, _path: &str, _error: &str) {}
    fn on_complete(&self, _files: u64, _blocks: u64, _skipped: u64, _errors: u64) {}
}

impl EmbeddingProgress for NopProgress {
    fn on_start(&self, _total_blocks: u64) {}
    fn on_batch_complete(&self, _batch_size: u64) {}
    fn on_complete(&self, _total: u64) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nop_index_progress_methods() {
        let p = NopProgress;
        p.on_discover(10);
        p.on_file_indexed("src/main.ts");
        p.on_file_skipped("src/lib.ts");
        p.on_file_error("src/bad.ts", "permission denied");
        IndexProgress::on_complete(&p, 5, 10, 2, 1);
    }

    #[test]
    fn test_nop_embedding_progress_methods() {
        let p = NopProgress;
        EmbeddingProgress::on_start(&p, 100);
        EmbeddingProgress::on_batch_complete(&p, 32);
        EmbeddingProgress::on_complete(&p, 100);
    }
}
