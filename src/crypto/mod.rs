pub mod group_key;
pub mod identity;
pub mod key_exchange;
pub mod message;

use anyhow::Result;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use ed25519_dalek::VerifyingKey;
use std::path::Path;
use uuid::Uuid;
use x25519_dalek::PublicKey as X25519PublicKey;

use group_key::KeyChain;
use identity::E2eIdentity;
use key_exchange::{decrypt_key_bundle, encrypt_key_bundle, EncryptedKeyBundle};

/// High-level crypto context for a single chat.
pub struct ChatCrypto {
    identity: E2eIdentity,
    keychain: KeyChain,
    chat_dir: std::path::PathBuf,
}

impl ChatCrypto {
    /// Load for an existing chat (keychain from disk). Returns None if no keys yet.
    pub fn load(identity_dir: &Path, chat_dir: &Path) -> Result<Option<Self>> {
        let identity = E2eIdentity::load(identity_dir)?;
        match KeyChain::load(chat_dir)? {
            Some(keychain) => Ok(Some(ChatCrypto {
                identity,
                keychain,
                chat_dir: chat_dir.to_path_buf(),
            })),
            None => Ok(None),
        }
    }

    /// Initialize for a newly-created chat (generates epoch 0).
    pub fn create_new(identity_dir: &Path, chat_dir: &Path) -> Result<Self> {
        let identity = E2eIdentity::load(identity_dir)?;
        let keychain = KeyChain::generate_new();
        keychain.save(chat_dir)?;
        Ok(ChatCrypto {
            identity,
            keychain,
            chat_dir: chat_dir.to_path_buf(),
        })
    }

    /// Initialize with an empty keychain (for joiners awaiting key distribution).
    pub fn from_empty(identity_dir: &Path, chat_dir: &Path) -> Result<Self> {
        let identity = E2eIdentity::load(identity_dir)?;
        let keychain = KeyChain::from_epochs(vec![]);
        Ok(ChatCrypto {
            identity,
            keychain,
            chat_dir: chat_dir.to_path_buf(),
        })
    }

    /// Encrypt a plaintext message. Returns (base64-encoded wire bytes, epoch number).
    pub fn encrypt(&self, sender_uuid: &Uuid, plaintext: &str) -> Result<(String, u32)> {
        let wire = message::encrypt_message(&self.keychain, sender_uuid, plaintext.as_bytes())?;
        let b64 = B64.encode(&wire);
        Ok((b64, self.keychain.current_epoch()))
    }

    /// Decrypt a message from its base64 ciphertext field.
    pub fn decrypt(&self, sender_uuid: &Uuid, ciphertext_b64: &str) -> Result<String> {
        let wire = B64.decode(ciphertext_b64)?;
        let plaintext = message::decrypt_message(&self.keychain, sender_uuid, &wire)?;
        Ok(String::from_utf8(plaintext)?)
    }

    pub fn current_epoch(&self) -> u32 {
        self.keychain.current_epoch()
    }

    /// Prepare an encrypted key bundle for a recipient.
    /// `all_epochs`: true to send the full chain (new member join), false for just the latest.
    pub fn prepare_key_bundle(
        &self,
        recipient_x25519: &X25519PublicKey,
        all_epochs: bool,
    ) -> Result<EncryptedKeyBundle> {
        let epochs = if all_epochs {
            self.keychain.all_epochs()
        } else {
            std::slice::from_ref(self.keychain.current_key().unwrap())
        };
        encrypt_key_bundle(&self.identity, recipient_x25519, epochs)
    }

    /// Receive and decrypt a key bundle from another member. Persists the updated keychain.
    pub fn receive_key_bundle(
        &mut self,
        sender_x25519: &X25519PublicKey,
        sender_ed25519: &VerifyingKey,
        bundle: &EncryptedKeyBundle,
    ) -> Result<()> {
        let epoch_keys = decrypt_key_bundle(&self.identity, sender_x25519, sender_ed25519, bundle)?;
        for ek in epoch_keys {
            self.keychain.add_epoch(ek);
        }
        self.keychain.save(&self.chat_dir)?;
        Ok(())
    }

    /// Rotate key on member removal. Returns the new epoch number.
    pub fn rotate_key(&mut self) -> Result<u32> {
        let new_epoch = self.keychain.rotate().epoch;
        self.keychain.save(&self.chat_dir)?;
        Ok(new_epoch)
    }

    pub fn identity(&self) -> &E2eIdentity {
        &self.identity
    }
}

/// Parse a key:distribute wire payload into an EncryptedKeyBundle.
pub fn parse_key_bundle_from_wire(payload: &serde_json::Value) -> Result<EncryptedKeyBundle> {
    let epochs_arr = payload["epochs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("key:distribute missing epochs array"))?;

    let mut epochs = Vec::with_capacity(epochs_arr.len());
    for entry in epochs_arr {
        let epoch = entry["epoch"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("epoch entry missing epoch number"))?
            as u32;
        let encrypted_b64 = entry["encrypted_key"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("epoch entry missing encrypted_key"))?;
        let encrypted = B64.decode(encrypted_b64)?;
        epochs.push((epoch, encrypted));
    }

    let sig_b64 = payload["signature"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("key:distribute missing signature"))?;
    let signature = B64.decode(sig_b64)?;

    Ok(EncryptedKeyBundle { epochs, signature })
}

/// Serialize an EncryptedKeyBundle to the wire JSON format.
pub fn bundle_to_wire_payload(
    bundle: &EncryptedKeyBundle,
    recipient_id: &str,
) -> serde_json::Value {
    let epochs: Vec<serde_json::Value> = bundle
        .epochs
        .iter()
        .map(|(epoch, encrypted)| {
            serde_json::json!({
                "epoch": epoch,
                "encrypted_key": B64.encode(encrypted)
            })
        })
        .collect();

    serde_json::json!({
        "recipient_id": recipient_id,
        "epochs": epochs,
        "signature": B64.encode(&bundle.signature)
    })
}
