pub mod credentials;
pub mod e2e_keys;
pub mod registration;

/// Writes `data` to `path` with owner-only permissions (0o600 on Unix),
/// creating the file atomically before any data is written.
#[cfg(unix)]
pub(super) fn write_private(path: &std::path::Path, data: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)?;
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn write_private(path: &std::path::Path, data: &[u8]) -> anyhow::Result<()> {
    std::fs::write(path, data)?;
    Ok(())
}
