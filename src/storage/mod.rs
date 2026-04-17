pub mod contacts;
pub mod messages;

/// Restrict a file to be readable and writable only by the owning user.
///
/// On Unix this sets mode 0o600 via `set_permissions`.
/// On Windows this removes inherited ACEs and grants only the current user
/// full access via `icacls`, which is present on all supported Windows versions.
pub(crate) fn restrict_file_permissions(path: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    #[cfg(windows)]
    {
        let username = std::env::var("USERNAME")
            .map_err(|_| anyhow::anyhow!("could not determine current Windows username"))?;
        let status = std::process::Command::new("icacls")
            .arg(path)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!("{}:F", username))
            .status()
            .map_err(|e| anyhow::anyhow!("icacls failed to run: {}", e))?;
        if !status.success() {
            anyhow::bail!("icacls failed to restrict permissions on {:?}", path);
        }
    }

    Ok(())
}
