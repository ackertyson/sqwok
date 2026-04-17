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

/// Load existing public keys from disk. Reads the Ed25519 public key directly
/// and derives the X25519 public key from the stored private key.
pub fn load_public_keys(dir: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    let ed25519_public = std::fs::read(dir.join("e2e_public.key"))?;
    let x25519_private_bytes = std::fs::read(dir.join("x25519_private.key"))?;
    let x25519_arr: [u8; 32] = x25519_private_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("x25519_private.key has wrong length"))?;
    let x25519_secret = StaticSecret::from(x25519_arr);
    let x25519_public = X25519PublicKey::from(&x25519_secret).to_bytes().to_vec();
    Ok((ed25519_public, x25519_public))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sqwok_e2e_keys_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn generate_and_store_writes_all_key_files() {
        let dir = temp_dir();
        generate_and_store(&dir).unwrap();
        assert!(dir.join("e2e_private.key").exists());
        assert!(dir.join("e2e_public.key").exists());
        assert!(dir.join("x25519_private.key").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generate_and_store_returns_32_byte_keys() {
        let dir = temp_dir();
        let (ed, x) = generate_and_store(&dir).unwrap();
        assert_eq!(ed.len(), 32, "Ed25519 public key must be 32 bytes");
        assert_eq!(x.len(), 32, "X25519 public key must be 32 bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_public_keys_roundtrips_with_generate_and_store() {
        let dir = temp_dir();
        let (ed_orig, x_orig) = generate_and_store(&dir).unwrap();
        let (ed_loaded, x_loaded) = load_public_keys(&dir).unwrap();
        assert_eq!(
            ed_orig, ed_loaded,
            "Ed25519 public key must survive generate→load roundtrip"
        );
        assert_eq!(
            x_orig, x_loaded,
            "X25519 public key must survive generate→load roundtrip"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_public_keys_is_idempotent() {
        let dir = temp_dir();
        generate_and_store(&dir).unwrap();
        let (ed_a, x_a) = load_public_keys(&dir).unwrap();
        let (ed_b, x_b) = load_public_keys(&dir).unwrap();
        assert_eq!(ed_a, ed_b);
        assert_eq!(x_a, x_b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_public_keys_fails_when_ed25519_public_key_missing() {
        let dir = temp_dir();
        generate_and_store(&dir).unwrap();
        std::fs::remove_file(dir.join("e2e_public.key")).unwrap();
        assert!(
            load_public_keys(&dir).is_err(),
            "must fail when e2e_public.key is absent"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_public_keys_fails_when_x25519_private_key_missing() {
        let dir = temp_dir();
        generate_and_store(&dir).unwrap();
        std::fs::remove_file(dir.join("x25519_private.key")).unwrap();
        assert!(
            load_public_keys(&dir).is_err(),
            "must fail when x25519_private.key is absent"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_public_keys_fails_when_x25519_private_key_wrong_length() {
        let dir = temp_dir();
        generate_and_store(&dir).unwrap();
        std::fs::write(dir.join("x25519_private.key"), [0u8; 16]).unwrap();
        assert!(
            load_public_keys(&dir).is_err(),
            "must fail when x25519_private.key is not 32 bytes"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_generate_calls_produce_independent_keys() {
        let dir1 = temp_dir();
        let dir2 = temp_dir();
        let (ed1, x1) = generate_and_store(&dir1).unwrap();
        let (ed2, x2) = generate_and_store(&dir2).unwrap();
        assert_ne!(ed1, ed2, "Ed25519 keys must be independently random");
        assert_ne!(x1, x2, "X25519 keys must be independently random");
        let _ = std::fs::remove_dir_all(&dir1);
        let _ = std::fs::remove_dir_all(&dir2);
    }
}
