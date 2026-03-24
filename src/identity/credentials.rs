use anyhow::Result;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub fn save_credentials(
    dir: &Path,
    key_pem: &str,
    cert_pem: &str,
    ca_pem: &str,
    user_uuid: &str,
) -> Result<()> {
    std::fs::create_dir_all(dir)?;

    let key_path = dir.join("private_key.pem");
    std::fs::write(&key_path, key_pem)?;

    #[cfg(unix)]
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;

    std::fs::write(dir.join("cert.pem"), cert_pem)?;
    std::fs::write(dir.join("ca.pem"), ca_pem)?;
    std::fs::write(dir.join("user_uuid"), user_uuid)?;

    Ok(())
}

pub fn is_registered(dir: &Path) -> bool {
    dir.join("cert.pem").exists() && dir.join("private_key.pem").exists()
}
