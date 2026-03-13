pub trait IndexProgress: Send + Sync {
    fn on_discover(&self, total_files: usize);
    fn on_file_indexed(&self, path: &str);
    fn on_file_skipped(&self, path: &str);
    fn on_file_error(&self, path: &str, error: &str);
    fn on_complete(&self, files: u64, blocks: u64, skipped: u64, errors: u64);
}

pub struct NopProgress;

impl IndexProgress for NopProgress {
    fn on_discover(&self, _total_files: usize) {}
    fn on_file_indexed(&self, _path: &str) {}
    fn on_file_skipped(&self, _path: &str) {}
    fn on_file_error(&self, _path: &str, _error: &str) {}
    fn on_complete(&self, _files: u64, _blocks: u64, _skipped: u64, _errors: u64) {}
}
