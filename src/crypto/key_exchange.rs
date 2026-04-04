use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::Result;
use ed25519_dalek::{Signature, VerifyingKey};
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
/// Salt is the sorted concatenation of both parties' X25519 public keys,
/// ensuring the derived key is unique per sender-recipient pair regardless
/// of who initiates.
fn derive_wrapping_key(
    shared_secret: &[u8; 32],
    our_public: &X25519PublicKey,
    their_public: &X25519PublicKey,
    info: &[u8],
) -> Key<Aes256Gcm> {
    let a = our_public.as_bytes();
    let b = their_public.as_bytes();
    let mut salt = [0u8; 64];
    if a < b {
        salt[..32].copy_from_slice(a);
        salt[32..].copy_from_slice(b);
    } else {
        salt[..32].copy_from_slice(b);
        salt[32..].copy_from_slice(a);
    }
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared_secret);
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
    let our_public = identity.x25519_public();
    let wrapping_key = derive_wrapping_key(
        &shared_secret,
        &our_public,
        recipient_x25519,
        b"sqwok-key-wrap-v1",
    );
    let cipher = Aes256Gcm::new(&wrapping_key);

    let mut epochs = Vec::with_capacity(epoch_keys.len());
    let mut sign_payload = Vec::new();

    for ek in epoch_keys {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Bind epoch number as AAD so ciphertext is self-authenticating
        let aad = ek.epoch.to_le_bytes();
        let ciphertext = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: ek.key.as_ref(),
                    aad: &aad,
                },
            )
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
        .verify_strict(&sign_payload, &signature)
        .map_err(|_| anyhow::anyhow!("key bundle signature verification failed"))?;

    // Decrypt each epoch
    let shared_secret = identity.dh(sender_x25519);
    let our_public = identity.x25519_public();
    let wrapping_key = derive_wrapping_key(
        &shared_secret,
        &our_public,
        sender_x25519,
        b"sqwok-key-wrap-v1",
    );
    let cipher = Aes256Gcm::new(&wrapping_key);

    let mut epoch_keys = Vec::with_capacity(bundle.epochs.len());
    for (epoch, blob) in &bundle.epochs {
        if blob.len() < 12 {
            anyhow::bail!("encrypted key blob too short for epoch {}", epoch);
        }
        let nonce = Nonce::from_slice(&blob[..12]);
        let aad = epoch.to_le_bytes();
        let plaintext = cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &blob[12..],
                    aad: &aad,
                },
            )
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

    #[test]
    fn test_tampered_epoch_blob_fails() {
        let (alice, a_dir) = make_test_identity();
        let (bob, b_dir) = make_test_identity();
        let kc = KeyChain::generate_new();

        let mut bundle = encrypt_key_bundle(&alice, &bob.x25519_public(), kc.all_epochs()).unwrap();
        // Flip a bit in the encrypted key blob
        bundle.epochs[0].1[15] ^= 0x01;

        // Signature is over the original blob, so signature check fails
        let result = decrypt_key_bundle(
            &bob,
            &alice.x25519_public(),
            &alice.verifying_key(),
            &bundle,
        );
        assert!(result.is_err(), "tampered blob must be detected");

        let _ = std::fs::remove_dir_all(a_dir);
        let _ = std::fs::remove_dir_all(b_dir);
    }

    #[test]
    fn test_reordered_epochs_fails_signature() {
        let (alice, a_dir) = make_test_identity();
        let (bob, b_dir) = make_test_identity();
        let mut kc = KeyChain::generate_new();
        kc.rotate();

        let mut bundle = encrypt_key_bundle(&alice, &bob.x25519_public(), kc.all_epochs()).unwrap();
        // Swap the two epoch entries
        bundle.epochs.swap(0, 1);

        let result = decrypt_key_bundle(
            &bob,
            &alice.x25519_public(),
            &alice.verifying_key(),
            &bundle,
        );
        assert!(
            result.is_err(),
            "reordering epochs must invalidate the signature"
        );

        let _ = std::fs::remove_dir_all(a_dir);
        let _ = std::fs::remove_dir_all(b_dir);
    }

    #[test]
    fn test_tampered_signature_fails() {
        let (alice, a_dir) = make_test_identity();
        let (bob, b_dir) = make_test_identity();
        let kc = KeyChain::generate_new();

        let mut bundle = encrypt_key_bundle(&alice, &bob.x25519_public(), kc.all_epochs()).unwrap();
        // Corrupt the signature
        bundle.signature[0] ^= 0xFF;

        let result = decrypt_key_bundle(
            &bob,
            &alice.x25519_public(),
            &alice.verifying_key(),
            &bundle,
        );
        assert!(
            result.is_err(),
            "corrupted signature must fail verification"
        );

        let _ = std::fs::remove_dir_all(a_dir);
        let _ = std::fs::remove_dir_all(b_dir);
    }

    #[test]
    fn test_truncated_signature_fails() {
        let (alice, a_dir) = make_test_identity();
        let (bob, b_dir) = make_test_identity();
        let kc = KeyChain::generate_new();

        let mut bundle = encrypt_key_bundle(&alice, &bob.x25519_public(), kc.all_epochs()).unwrap();
        bundle.signature.truncate(32); // Ed25519 sigs are 64 bytes

        let result = decrypt_key_bundle(
            &bob,
            &alice.x25519_public(),
            &alice.verifying_key(),
            &bundle,
        );
        assert!(result.is_err(), "truncated signature must fail");

        let _ = std::fs::remove_dir_all(a_dir);
        let _ = std::fs::remove_dir_all(b_dir);
    }

    #[test]
    fn test_truncated_epoch_blob_fails() {
        let (alice, a_dir) = make_test_identity();
        let (bob, b_dir) = make_test_identity();
        let kc = KeyChain::generate_new();

        let bundle = encrypt_key_bundle(&alice, &bob.x25519_public(), kc.all_epochs()).unwrap();
        // Create a bundle with a too-short blob but valid signature structure
        let short_bundle = EncryptedKeyBundle {
            epochs: vec![(0, vec![0u8; 5])],
            signature: bundle.signature.clone(),
        };

        let result = decrypt_key_bundle(
            &bob,
            &alice.x25519_public(),
            &alice.verifying_key(),
            &short_bundle,
        );
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(a_dir);
        let _ = std::fs::remove_dir_all(b_dir);
    }

    #[test]
    fn test_bidirectional_key_exchange() {
        // Verify Alice→Bob and Bob→Alice both work (HKDF salt ordering is symmetric)
        let (alice, a_dir) = make_test_identity();
        let (bob, b_dir) = make_test_identity();
        let kc = KeyChain::generate_new();

        // Alice sends to Bob
        let bundle_ab = encrypt_key_bundle(&alice, &bob.x25519_public(), kc.all_epochs()).unwrap();
        let dec_ab = decrypt_key_bundle(
            &bob,
            &alice.x25519_public(),
            &alice.verifying_key(),
            &bundle_ab,
        )
        .unwrap();
        assert_eq!(dec_ab[0].key, kc.get(0).unwrap().key);

        // Bob sends to Alice
        let bundle_ba = encrypt_key_bundle(&bob, &alice.x25519_public(), kc.all_epochs()).unwrap();
        let dec_ba = decrypt_key_bundle(
            &alice,
            &bob.x25519_public(),
            &bob.verifying_key(),
            &bundle_ba,
        )
        .unwrap();
        assert_eq!(dec_ba[0].key, kc.get(0).unwrap().key);

        let _ = std::fs::remove_dir_all(a_dir);
        let _ = std::fs::remove_dir_all(b_dir);
    }

    #[test]
    fn test_epoch_aad_prevents_epoch_swap() {
        // Verify that swapping the epoch number on a blob fails decryption
        // even if the signature is somehow bypassed (tests AAD independently)
        let (alice, a_dir) = make_test_identity();
        let (bob, b_dir) = make_test_identity();
        let mut kc = KeyChain::generate_new();
        kc.rotate();

        let bundle = encrypt_key_bundle(&alice, &bob.x25519_public(), kc.all_epochs()).unwrap();

        // Take epoch 0's blob but label it as epoch 1 — re-sign so signature passes
        let swapped_epochs = vec![(1u32, bundle.epochs[0].1.clone())];
        let mut sign_payload = Vec::new();
        for (epoch, blob) in &swapped_epochs {
            sign_payload.extend_from_slice(&epoch.to_le_bytes());
            sign_payload.extend_from_slice(blob);
        }
        let signature = alice.sign(&sign_payload).to_bytes().to_vec();
        let swapped_bundle = EncryptedKeyBundle {
            epochs: swapped_epochs,
            signature,
        };

        let result = decrypt_key_bundle(
            &bob,
            &alice.x25519_public(),
            &alice.verifying_key(),
            &swapped_bundle,
        );
        assert!(
            result.is_err(),
            "epoch AAD must prevent decrypting blob under wrong epoch number"
        );

        let _ = std::fs::remove_dir_all(a_dir);
        let _ = std::fs::remove_dir_all(b_dir);
    }
}
