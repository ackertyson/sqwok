use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::Result;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use hkdf::Hkdf;
use rand::{rngs::OsRng, RngCore};
use sha2::Sha256;
use x25519_dalek::PublicKey as X25519PublicKey;

use crate::crypto::group_key::EpochKey;
use crate::crypto::identity::E2eIdentity;

/// An encrypted key bundle ready to send over the wire.
#[derive(Debug)]
pub struct EncryptedKeyBundle {
    /// One entry per epoch: (epoch, encrypted_key_bytes)
    pub epochs: Vec<(u32, Vec<u8>)>,
    /// Ed25519 signature over the canonical bundle content
    pub signature: Vec<u8>,
}

/// Derive a wrapping key from a DH shared secret using HKDF-SHA256.
fn derive_wrapping_key(shared_secret: &[u8; 32], info: &[u8]) -> Key<Aes256Gcm> {
    let hk = Hkdf::<Sha256>::new(None, shared_secret);
    let mut okm = [0u8; 32];
    hk.expand(info, &mut okm)
        .expect("32 bytes is a valid length for HKDF-SHA256");
    Key::<Aes256Gcm>::from(okm)
}

/// Encrypt epoch keys for a specific recipient.
///
/// Wire format per epoch: [nonce: 12 bytes][ciphertext + tag: 48 bytes]
/// The bundle is signed with the sender's Ed25519 key.
pub fn encrypt_key_bundle(
    identity: &E2eIdentity,
    recipient_x25519: &X25519PublicKey,
    epoch_keys: &[EpochKey],
) -> Result<EncryptedKeyBundle> {
    let shared_secret = identity.dh(recipient_x25519);
    let wrapping_key = derive_wrapping_key(&shared_secret, b"sqwok-key-wrap-v1");
    let cipher = Aes256Gcm::new(&wrapping_key);

    let mut epochs = Vec::with_capacity(epoch_keys.len());
    let mut sign_payload = Vec::new();

    for ek in epoch_keys {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, ek.key.as_ref())
            .map_err(|e| anyhow::anyhow!("AES-GCM encrypt failed: {}", e))?;

        let mut blob = Vec::with_capacity(12 + ciphertext.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ciphertext);

        sign_payload.extend_from_slice(&ek.epoch.to_le_bytes());
        sign_payload.extend_from_slice(&blob);

        epochs.push((ek.epoch, blob));
    }

    let signature = identity.sign(&sign_payload).to_bytes().to_vec();

    Ok(EncryptedKeyBundle { epochs, signature })
}

/// Decrypt a received key bundle from a sender.
/// Verifies the Ed25519 signature, then decrypts each epoch key.
pub fn decrypt_key_bundle(
    identity: &E2eIdentity,
    sender_x25519: &X25519PublicKey,
    sender_ed25519: &VerifyingKey,
    bundle: &EncryptedKeyBundle,
) -> Result<Vec<EpochKey>> {
    // Reconstruct sign payload and verify
    let mut sign_payload = Vec::new();
    for (epoch, blob) in &bundle.epochs {
        sign_payload.extend_from_slice(&epoch.to_le_bytes());
        sign_payload.extend_from_slice(blob);
    }

    let sig_bytes: [u8; 64] = bundle
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid signature length"))?;
    let signature = Signature::from_bytes(&sig_bytes);
    sender_ed25519
        .verify(&sign_payload, &signature)
        .map_err(|_| anyhow::anyhow!("key bundle signature verification failed"))?;

    // Decrypt each epoch
    let shared_secret = identity.dh(sender_x25519);
    let wrapping_key = derive_wrapping_key(&shared_secret, b"sqwok-key-wrap-v1");
    let cipher = Aes256Gcm::new(&wrapping_key);

    let mut epoch_keys = Vec::with_capacity(bundle.epochs.len());
    for (epoch, blob) in &bundle.epochs {
        if blob.len() < 12 {
            anyhow::bail!("encrypted key blob too short for epoch {}", epoch);
        }
        let nonce = Nonce::from_slice(&blob[..12]);
        let plaintext = cipher
            .decrypt(nonce, &blob[12..])
            .map_err(|_| anyhow::anyhow!("AES-GCM decrypt failed for epoch {}", epoch))?;
        if plaintext.len() != 32 {
            anyhow::bail!(
                "decrypted key wrong length for epoch {}: {}",
                epoch,
                plaintext.len()
            );
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&plaintext);
        epoch_keys.push(EpochKey { epoch: *epoch, key });
    }

    Ok(epoch_keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::group_key::KeyChain;
    use crate::crypto::identity::E2eIdentity;
    use rand::{rngs::OsRng, RngCore};
    use uuid::Uuid;

    fn make_test_identity() -> (E2eIdentity, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("sqwok_kx_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        std::fs::write(dir.join("e2e_private.key"), &seed).unwrap();
        (E2eIdentity::load(&dir).unwrap(), dir)
    }

    #[test]
    fn test_single_epoch_roundtrip() {
        let (alice, a_dir) = make_test_identity();
        let (bob, b_dir) = make_test_identity();
        let kc = KeyChain::generate_new();

        let bundle = encrypt_key_bundle(&alice, &bob.x25519_public(), kc.all_epochs()).unwrap();
        let decrypted = decrypt_key_bundle(
            &bob,
            &alice.x25519_public(),
            &alice.verifying_key(),
            &bundle,
        )
        .unwrap();

        assert_eq!(decrypted.len(), 1);
        assert_eq!(decrypted[0].epoch, 0);
        assert_eq!(decrypted[0].key, kc.get(0).unwrap().key);

        let _ = std::fs::remove_dir_all(a_dir);
        let _ = std::fs::remove_dir_all(b_dir);
    }

    #[test]
    fn test_multi_epoch_roundtrip() {
        let (alice, a_dir) = make_test_identity();
        let (bob, b_dir) = make_test_identity();
        let mut kc = KeyChain::generate_new();
        kc.rotate();
        kc.rotate();

        let bundle = encrypt_key_bundle(&alice, &bob.x25519_public(), kc.all_epochs()).unwrap();
        let decrypted = decrypt_key_bundle(
            &bob,
            &alice.x25519_public(),
            &alice.verifying_key(),
            &bundle,
        )
        .unwrap();

        assert_eq!(decrypted.len(), 3);
        for epoch in 0u32..3 {
            assert_eq!(decrypted[epoch as usize].epoch, epoch);
            assert_eq!(decrypted[epoch as usize].key, kc.get(epoch).unwrap().key);
        }

        let _ = std::fs::remove_dir_all(a_dir);
        let _ = std::fs::remove_dir_all(b_dir);
    }

    #[test]
    fn test_wrong_signer_verification_fails() {
        let (alice, a_dir) = make_test_identity();
        let (bob, b_dir) = make_test_identity();
        let (charlie, c_dir) = make_test_identity();
        let kc = KeyChain::generate_new();

        let bundle = encrypt_key_bundle(&alice, &bob.x25519_public(), kc.all_epochs()).unwrap();
        // Use Charlie's verifying key — won't match Alice's signature
        let result = decrypt_key_bundle(
            &bob,
            &alice.x25519_public(),
            &charlie.verifying_key(),
            &bundle,
        );
        assert!(
            result.is_err(),
            "signature verification must fail for wrong signer"
        );

        let _ = std::fs::remove_dir_all(a_dir);
        let _ = std::fs::remove_dir_all(b_dir);
        let _ = std::fs::remove_dir_all(c_dir);
    }

    #[test]
    fn test_wrong_recipient_dh_decryption_fails() {
        let (alice, a_dir) = make_test_identity();
        let (bob, b_dir) = make_test_identity();
        let (charlie, c_dir) = make_test_identity();
        let kc = KeyChain::generate_new();

        // Bundle is encrypted for Bob; Charlie tries to decrypt it
        let bundle = encrypt_key_bundle(&alice, &bob.x25519_public(), kc.all_epochs()).unwrap();
        // Signature check passes (Alice signed it), but AES-GCM fails (wrong DH secret)
        let result = decrypt_key_bundle(
            &charlie,
            &alice.x25519_public(),
            &alice.verifying_key(),
            &bundle,
        );
        assert!(
            result.is_err(),
            "AES-GCM must fail when DH secret is for a different recipient"
        );

        let _ = std::fs::remove_dir_all(a_dir);
        let _ = std::fs::remove_dir_all(b_dir);
        let _ = std::fs::remove_dir_all(c_dir);
    }

    #[test]
    fn test_empty_epoch_list_roundtrip() {
        let (alice, a_dir) = make_test_identity();
        let (bob, b_dir) = make_test_identity();

        let bundle = encrypt_key_bundle(&alice, &bob.x25519_public(), &[]).unwrap();
        let decrypted = decrypt_key_bundle(
            &bob,
            &alice.x25519_public(),
            &alice.verifying_key(),
            &bundle,
        )
        .unwrap();
        assert!(decrypted.is_empty());

        let _ = std::fs::remove_dir_all(a_dir);
        let _ = std::fs::remove_dir_all(b_dir);
    }
}
