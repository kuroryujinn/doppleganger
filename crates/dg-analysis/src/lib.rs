pub mod diff;
pub mod model;
pub mod normalize;
pub mod size;

pub use diff::{analyze, AnalysisReport};
pub use normalize::{normalize, AnalysisError};

/// Read a regular input file without allocating beyond its format budget.
pub fn read_bounded(path: &std::path::Path, limit: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    if !std::fs::metadata(path)?.is_file() {
        return Err(std::io::Error::other("expected a regular input file"));
    }
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("expected a regular input file"));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(std::io::Error::other(format!(
            "input exceeds {limit} byte limit"
        )));
    }
    Ok(bytes)
}
