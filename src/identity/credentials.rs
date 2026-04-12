use anyhow::Result;
use std::path::Path;

pub fn save_credentials(
    dir: &Path,
    key_pem: &str,
    cert_pem: &str,
    ca_pem: &str,
    user_uuid: &str,
) -> Result<()> {
    std::fs::create_dir_all(dir)?;

    let key_path = dir.join("private_key.pem");
    super::write_private(&key_path, key_pem.as_bytes())?;

    std::fs::write(dir.join("cert.pem"), cert_pem)?;
    std::fs::write(dir.join("ca.pem"), ca_pem)?;
    std::fs::write(dir.join("user_uuid"), user_uuid)?;

    Ok(())
}

pub fn is_registered(dir: &Path) -> bool {
    dir.join("cert.pem").exists() && dir.join("private_key.pem").exists()
}
