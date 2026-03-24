use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::Result;
use rand::{rngs::OsRng, RngCore};
use uuid::Uuid;

use crate::crypto::group_key::KeyChain;

/// Wire format: [epoch: 4 bytes LE][nonce: 12 bytes][ciphertext + GCM tag]
/// Total overhead: 32 bytes beyond plaintext.
///
/// AAD = epoch (4 bytes LE) || sender_uuid (16 bytes raw)
fn build_aad(epoch: u32, sender_uuid: &Uuid) -> Vec<u8> {
    let mut aad = Vec::with_capacity(20);
    aad.extend_from_slice(&epoch.to_le_bytes());
    aad.extend_from_slice(sender_uuid.as_bytes());
    aad
}

pub fn encrypt_message(
    keychain: &KeyChain,
    sender_uuid: &Uuid,
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let epoch_key = keychain
        .current_key()
        .ok_or_else(|| anyhow::anyhow!("no keys in keychain"))?;

    let key = Key::<Aes256Gcm>::from_slice(&epoch_key.key);
    let cipher = Aes256Gcm::new(key);

    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let aad = build_aad(epoch_key.epoch, sender_uuid);
    let ciphertext = cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|e| anyhow::anyhow!("message encrypt failed: {}", e))?;

    let mut wire = Vec::with_capacity(4 + 12 + ciphertext.len());
    wire.extend_from_slice(&epoch_key.epoch.to_le_bytes());
    wire.extend_from_slice(&nonce_bytes);
    wire.extend_from_slice(&ciphertext);

    Ok(wire)
}

pub fn decrypt_message(keychain: &KeyChain, sender_uuid: &Uuid, wire: &[u8]) -> Result<Vec<u8>> {
    if wire.len() < 4 + 12 + 16 {
        anyhow::bail!("wire format too short: {} bytes", wire.len());
    }

    let epoch = u32::from_le_bytes(wire[..4].try_into().unwrap());
    let nonce = Nonce::from_slice(&wire[4..16]);
    let ciphertext = &wire[16..];

    let epoch_key = keychain
        .get(epoch)
        .ok_or_else(|| anyhow::anyhow!("no key for epoch {}", epoch))?;

    let key = Key::<Aes256Gcm>::from_slice(&epoch_key.key);
    let cipher = Aes256Gcm::new(key);

    let aad = build_aad(epoch, sender_uuid);
    let plaintext = cipher
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| {
            anyhow::anyhow!(
                "message decrypt failed (epoch {}, sender {})",
                epoch,
                sender_uuid
            )
        })?;

    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::group_key::KeyChain;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let keychain = KeyChain::generate_new();
        let sender = Uuid::new_v4();
        let plaintext = b"hello world";

        let wire = encrypt_message(&keychain, &sender, plaintext).unwrap();
        let decrypted = decrypt_message(&keychain, &sender, &wire).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_wrong_key_fails() {
        let keychain1 = KeyChain::generate_new();
        let keychain2 = KeyChain::generate_new();
        let sender = Uuid::new_v4();

        let wire = encrypt_message(&keychain1, &sender, b"secret").unwrap();
        // keychain2 has epoch 0 but a different key, so epoch lookup succeeds but decrypt fails
        let result = decrypt_message(&keychain2, &sender, &wire);
        assert!(result.is_err());
    }

    #[test]
    fn test_wrong_sender_aad_fails() {
        let keychain = KeyChain::generate_new();
        let real_sender = Uuid::new_v4();
        let fake_sender = Uuid::new_v4();

        let wire = encrypt_message(&keychain, &real_sender, b"secret").unwrap();
        let result = decrypt_message(&keychain, &fake_sender, &wire);
        assert!(result.is_err());
    }

    #[test]
    fn test_wire_too_short_fails() {
        let keychain = KeyChain::generate_new();
        let sender = Uuid::new_v4();
        let result = decrypt_message(&keychain, &sender, &[0u8; 10]);
        assert!(result.is_err());
    }
}
