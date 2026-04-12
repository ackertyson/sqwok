use anyhow::Result;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::path::Path;

pub fn generate_and_store(dir: &Path) -> Result<Vec<u8>> {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let private_bytes = signing_key.to_bytes();
    let public_bytes = verifying_key.to_bytes();

    let private_path = dir.join("e2e_private.key");
    super::write_private(&private_path, &private_bytes)?;

    std::fs::write(dir.join("e2e_public.key"), public_bytes)?;

    Ok(public_bytes.to_vec())
}
