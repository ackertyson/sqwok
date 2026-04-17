use anyhow::Result;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use std::path::Path;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use zeroize::{Zeroize, Zeroizing};

/// Holds an Ed25519 signing key and an independently-generated X25519 key.
/// The two keys are kept fully separate: each is derived from its own random seed.
pub struct E2eIdentity {
    signing_key: SigningKey,
    x25519_secret: StaticSecret,
}

impl E2eIdentity {
    /// Load from the identity dir. Requires both `e2e_private.key` (Ed25519, 32 bytes)
    /// and `x25519_private.key` (X25519, 32 bytes).
    pub fn load(identity_dir: &Path) -> Result<Self> {
        let ed25519_bytes = std::fs::read(identity_dir.join("e2e_private.key"))?;
        if ed25519_bytes.len() != 32 {
            anyhow::bail!("e2e_private.key must be exactly 32 bytes");
        }
        let mut seed: [u8; 32] = ed25519_bytes.try_into().unwrap();
        let signing_key = SigningKey::from_bytes(&seed);
        seed.zeroize();

        let x25519_bytes = std::fs::read(identity_dir.join("x25519_private.key"))?;
        if x25519_bytes.len() != 32 {
            anyhow::bail!("x25519_private.key must be exactly 32 bytes");
        }
        let mut x25519_seed: [u8; 32] = x25519_bytes.try_into().unwrap();
        let x25519_secret = StaticSecret::from(x25519_seed);
        x25519_seed.zeroize();

        Ok(E2eIdentity {
            signing_key,
            x25519_secret,
        })
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    pub fn x25519_public(&self) -> X25519PublicKey {
        X25519PublicKey::from(&self.x25519_secret)
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }

    /// X25519 DH with a peer's public key. Returns the 32-byte shared secret,
    /// wrapped in `Zeroizing` so it is wiped from memory when dropped.
    pub fn dh(&self, peer_public: &X25519PublicKey) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.x25519_secret.diffie_hellman(peer_public).to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::OsRng, RngCore};
    use uuid::Uuid;

    fn make_test_identity() -> (E2eIdentity, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("sqwok_id_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut ed25519_seed = [0u8; 32];
        OsRng.fill_bytes(&mut ed25519_seed);
        std::fs::write(dir.join("e2e_private.key"), ed25519_seed).unwrap();

        let mut x25519_seed = [0u8; 32];
        OsRng.fill_bytes(&mut x25519_seed);
        std::fs::write(dir.join("x25519_private.key"), x25519_seed).unwrap();

        let identity = E2eIdentity::load(&dir).unwrap();
        (identity, dir)
    }

    #[test]
    fn test_load_produces_consistent_keys() {
        let (id1, dir) = make_test_identity();
        let id2 = E2eIdentity::load(&dir).unwrap();
        assert_eq!(
            id1.verifying_key().to_bytes(),
            id2.verifying_key().to_bytes()
        );
        assert_eq!(
            id1.x25519_public().to_bytes(),
            id2.x25519_public().to_bytes()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_wrong_ed25519_size_fails() {
        let dir = std::env::temp_dir().join(format!("sqwok_id_bad_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("e2e_private.key"), [0u8; 16]).unwrap();
        std::fs::write(dir.join("x25519_private.key"), [0u8; 32]).unwrap();
        assert!(E2eIdentity::load(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_wrong_x25519_size_fails() {
        let dir = std::env::temp_dir().join(format!("sqwok_id_bad2_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("e2e_private.key"), [0u8; 32]).unwrap();
        std::fs::write(dir.join("x25519_private.key"), [0u8; 16]).unwrap();
        assert!(E2eIdentity::load(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_missing_x25519_fails() {
        let dir = std::env::temp_dir().join(format!("sqwok_id_missing_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("e2e_private.key"), [0u8; 32]).unwrap();
        // x25519_private.key intentionally absent
        assert!(E2eIdentity::load(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_dh_is_symmetric() {
        let (alice, alice_dir) = make_test_identity();
        let (bob, bob_dir) = make_test_identity();
        let alice_secret = alice.dh(&bob.x25519_public());
        let bob_secret = bob.dh(&alice.x25519_public());
        assert_eq!(alice_secret, bob_secret, "X25519 DH must be symmetric");
        let _ = std::fs::remove_dir_all(&alice_dir);
        let _ = std::fs::remove_dir_all(&bob_dir);
    }

    #[test]
    fn test_different_identities_have_different_dh_secrets() {
        let (alice, alice_dir) = make_test_identity();
        let (bob, bob_dir) = make_test_identity();
        let (charlie, charlie_dir) = make_test_identity();
        let alice_bob = alice.dh(&bob.x25519_public());
        let alice_charlie = alice.dh(&charlie.x25519_public());
        assert_ne!(alice_bob, alice_charlie);
        let _ = std::fs::remove_dir_all(&alice_dir);
        let _ = std::fs::remove_dir_all(&bob_dir);
        let _ = std::fs::remove_dir_all(&charlie_dir);
    }

    #[test]
    fn test_sign_and_verify() {
        let (id, dir) = make_test_identity();
        let message = b"sqwok test message";
        let sig = id.sign(message);
        id.verifying_key()
            .verify_strict(message, &sig)
            .expect("signature must verify with the corresponding verifying key");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sign_wrong_message_fails() {
        let (id, dir) = make_test_identity();
        let sig = id.sign(b"original");
        let result = id.verifying_key().verify_strict(b"tampered", &sig);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ed25519_and_x25519_are_independent() {
        // Verify the two keys have no deterministic relationship.
        // With the old design, x25519_public was fully determined by the ed25519 seed.
        // Now they are independent: same ed25519 seed, different x25519 seed → different x25519 public key.
        let dir = std::env::temp_dir().join(format!("sqwok_id_indep_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut ed_seed = [0u8; 32];
        OsRng.fill_bytes(&mut ed_seed);
        std::fs::write(dir.join("e2e_private.key"), ed_seed).unwrap();

        let mut x_seed_a = [0u8; 32];
        OsRng.fill_bytes(&mut x_seed_a);
        std::fs::write(dir.join("x25519_private.key"), x_seed_a).unwrap();
        let id_a = E2eIdentity::load(&dir).unwrap();

        let mut x_seed_b = [0u8; 32];
        OsRng.fill_bytes(&mut x_seed_b);
        std::fs::write(dir.join("x25519_private.key"), x_seed_b).unwrap();
        let id_b = E2eIdentity::load(&dir).unwrap();

        // Same Ed25519 key
        assert_eq!(
            id_a.verifying_key().to_bytes(),
            id_b.verifying_key().to_bytes()
        );
        // Different X25519 public key (with overwhelming probability)
        assert_ne!(
            id_a.x25519_public().to_bytes(),
            id_b.x25519_public().to_bytes()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
