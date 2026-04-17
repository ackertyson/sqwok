use anyhow::Result;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::path::Path;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

/// Generate independent Ed25519 and X25519 keypairs, store both to disk,
/// and return (ed25519_public_bytes, x25519_public_bytes).
pub fn generate_and_store(dir: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    let signing_key = SigningKey::generate(&mut OsRng);
    let ed25519_public = signing_key.verifying_key().to_bytes().to_vec();
    super::write_private(&dir.join("e2e_private.key"), &signing_key.to_bytes())?;
    std::fs::write(dir.join("e2e_public.key"), &ed25519_public)?;

    let x25519_secret = StaticSecret::random_from_rng(OsRng);
    let x25519_public = X25519PublicKey::from(&x25519_secret).to_bytes().to_vec();
    super::write_private(&dir.join("x25519_private.key"), x25519_secret.as_bytes())?;

    Ok((ed25519_public, x25519_public))
}
