use anyhow::Result;
use curve25519_dalek::edwards::CompressedEdwardsY;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha512};
use std::path::Path;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use zeroize::{Zeroize, Zeroizing};

/// Holds both the Ed25519 identity key and derived X25519 key.
pub struct E2eIdentity {
    signing_key: SigningKey,
    x25519_secret: StaticSecret,
}

impl E2eIdentity {
    /// Load from the identity dir (e2e_private.key must be 32 bytes).
    pub fn load(identity_dir: &Path) -> Result<Self> {
        let key_path = identity_dir.join("e2e_private.key");
        let key_bytes = std::fs::read(&key_path)?;
        if key_bytes.len() != 32 {
            anyhow::bail!("e2e_private.key must be exactly 32 bytes");
        }
        let mut seed: [u8; 32] = key_bytes.try_into().unwrap();
        let signing_key = SigningKey::from_bytes(&seed);

        // Derive X25519 secret from Ed25519 seed via SHA-512, matching libsodium's
        // crypto_sign_ed25519_sk_to_curve25519.
        let hash_output = Sha512::digest(seed);
        let mut hash_bytes = Zeroizing::new([0u8; 64]);
        hash_bytes.copy_from_slice(&hash_output);
        let mut x25519_bytes: [u8; 32] = hash_bytes[..32].try_into().unwrap();
        x25519_bytes[0] &= 248;
        x25519_bytes[31] &= 127;
        x25519_bytes[31] |= 64;
        let x25519_secret = StaticSecret::from(x25519_bytes);

        seed.zeroize();
        x25519_bytes.zeroize();

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

    /// X25519 DH with a peer's public key. Returns the 32-byte shared secret.
    pub fn dh(&self, peer_public: &X25519PublicKey) -> [u8; 32] {
        self.x25519_secret.diffie_hellman(peer_public).to_bytes()
    }
}

/// Convert an Ed25519 public key to an X25519 public key via the birational map.
pub fn ed25519_to_x25519_public(ed_public: &VerifyingKey) -> Option<X25519PublicKey> {
    let compressed = CompressedEdwardsY::from_slice(ed_public.as_bytes()).ok()?;
    let edwards_point = compressed.decompress()?;
    let montgomery = edwards_point.to_montgomery();
    Some(X25519PublicKey::from(montgomery.to_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::OsRng, RngCore};
    use uuid::Uuid;

    fn make_test_identity() -> (E2eIdentity, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("sqwok_id_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        std::fs::write(dir.join("e2e_private.key"), &seed).unwrap();
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
    fn test_load_wrong_size_fails() {
        let dir = std::env::temp_dir().join(format!("sqwok_id_bad_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("e2e_private.key"), &[0u8; 16]).unwrap();
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
    fn test_ed25519_to_x25519_matches_derived_key() {
        let (id, dir) = make_test_identity();
        let converted = ed25519_to_x25519_public(&id.verifying_key())
            .expect("conversion must succeed for valid Ed25519 key");
        assert_eq!(
            converted.to_bytes(),
            id.x25519_public().to_bytes(),
            "birational map must produce the same X25519 public key as internal derivation"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
