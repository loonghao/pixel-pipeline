//! Atomic file writes: temp file + rename (PRD §7.9 FR-BATCH-005, §11.2).

use anyhow::{Context, Result};
use std::path::Path;

/// Write bytes to `path` atomically: write to a temp file in the same
/// directory, then rename over the destination.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(".").to_path_buf());
    std::fs::create_dir_all(&parent)
        .with_context(|| format!("creating dir {}", parent.display()))?;

    let mut tmp = tempfile::NamedTempFile::new_in(&parent)
        .with_context(|| format!("creating temp file in {}", parent.display()))?;
    use std::io::Write;
    tmp.write_all(bytes).context("writing temp file")?;
    tmp.flush().context("flushing temp file")?;
    tmp.persist(path)
        .with_context(|| format!("persisting {}", path.display()))?;
    Ok(())
}
