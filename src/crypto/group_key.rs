use anyhow::Result;
use rand::{rngs::OsRng, RngCore};
use std::path::Path;
use zeroize::Zeroize;

/// A single epoch key — 256-bit AES key.
#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct EpochKey {
    pub epoch: u32,
    pub key: [u8; 32],
}

/// The full key chain for a chat — all epochs in order.
pub struct KeyChain {
    keys: Vec<EpochKey>,
}

impl KeyChain {
    /// Create a new key chain with a random epoch-0 key (called by group creator).
    pub fn generate_new() -> Self {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        KeyChain {
            keys: vec![EpochKey { epoch: 0, key }],
        }
    }

    /// Create a key chain from a received set of epoch keys.
    pub fn from_epochs(mut epochs: Vec<EpochKey>) -> Self {
        epochs.sort_by_key(|e| e.epoch);
        KeyChain { keys: epochs }
    }

    pub fn current_epoch(&self) -> u32 {
        self.keys.last().map(|k| k.epoch).unwrap_or(0)
    }

    /// Get the key for a specific epoch (for decrypting old messages).
    pub fn get(&self, epoch: u32) -> Option<&EpochKey> {
        self.keys.iter().find(|k| k.epoch == epoch)
    }

    pub fn current_key(&self) -> Option<&EpochKey> {
        self.keys.last()
    }

    /// Append a new epoch with a random key (called on member removal re-key).
    pub fn rotate(&mut self) -> &EpochKey {
        let new_epoch = self.current_epoch() + 1;
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        self.keys.push(EpochKey {
            epoch: new_epoch,
            key,
        });
        self.keys.last().unwrap()
    }

    /// Append a received epoch key (from key:distribute).
    pub fn add_epoch(&mut self, epoch_key: EpochKey) {
        if !self.keys.iter().any(|k| k.epoch == epoch_key.epoch) {
            self.keys.push(epoch_key);
            self.keys.sort_by_key(|k| k.epoch);
        }
    }

    pub fn all_epochs(&self) -> &[EpochKey] {
        &self.keys
    }

    /// Persist key chain to disk. Format: [epoch: u32 LE][key: 32 bytes] repeated.
    pub fn save(&self, chat_dir: &Path) -> Result<()> {
        let path = chat_dir.join("keychain.bin");
        let mut data = Vec::with_capacity(self.keys.len() * 36);
        for ek in &self.keys {
            data.extend_from_slice(&ek.epoch.to_le_bytes());
            data.extend_from_slice(&ek.key);
        }
        std::fs::write(&path, &data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn load(chat_dir: &Path) -> Result<Option<Self>> {
        let path = chat_dir.join("keychain.bin");
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read(&path)?;
        if data.len() % 36 != 0 {
            anyhow::bail!(
                "keychain.bin corrupted: length {} not a multiple of 36",
                data.len()
            );
        }
        let mut keys = Vec::new();
        for chunk in data.chunks_exact(36) {
            let epoch = u32::from_le_bytes(chunk[..4].try_into().unwrap());
            let mut key = [0u8; 32];
            key.copy_from_slice(&chunk[4..36]);
            keys.push(EpochKey { epoch, key });
        }
        Ok(Some(KeyChain { keys }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_generate_new_has_epoch_zero() {
        let kc = KeyChain::generate_new();
        assert_eq!(kc.current_epoch(), 0);
        assert!(kc.current_key().is_some());
    }

    #[test]
    fn test_get_existing_epoch() {
        let kc = KeyChain::generate_new();
        let ek = kc.get(0);
        assert!(ek.is_some());
        assert_eq!(ek.unwrap().epoch, 0);
    }

    #[test]
    fn test_get_missing_epoch_returns_none() {
        let kc = KeyChain::generate_new();
        assert!(kc.get(99).is_none());
    }

    #[test]
    fn test_rotate_increments_epoch() {
        let mut kc = KeyChain::generate_new();
        kc.rotate();
        assert_eq!(kc.current_epoch(), 1);
        kc.rotate();
        assert_eq!(kc.current_epoch(), 2);
    }

    #[test]
    fn test_rotate_changes_key_material() {
        let mut kc = KeyChain::generate_new();
        let key0 = kc.current_key().unwrap().key;
        kc.rotate();
        let key1 = kc.current_key().unwrap().key;
        assert_ne!(
            key0, key1,
            "rotated key should differ (same RNG output would indicate broken RNG)"
        );
    }

    #[test]
    fn test_old_epoch_still_accessible_after_rotate() {
        let mut kc = KeyChain::generate_new();
        let key0 = kc.current_key().unwrap().key;
        kc.rotate();
        assert_eq!(kc.get(0).unwrap().key, key0);
    }

    #[test]
    fn test_add_epoch_deduplicates() {
        let mut kc = KeyChain::generate_new();
        // epoch 0 is already present; adding it again must not duplicate
        kc.add_epoch(EpochKey {
            epoch: 0,
            key: [1u8; 32],
        });
        assert_eq!(kc.all_epochs().len(), 1);
    }

    #[test]
    fn test_add_epoch_maintains_sort_order() {
        let mut kc = KeyChain::from_epochs(vec![]);
        kc.add_epoch(EpochKey {
            epoch: 2,
            key: [2u8; 32],
        });
        kc.add_epoch(EpochKey {
            epoch: 0,
            key: [0u8; 32],
        });
        kc.add_epoch(EpochKey {
            epoch: 1,
            key: [1u8; 32],
        });
        let epochs: Vec<u32> = kc.all_epochs().iter().map(|k| k.epoch).collect();
        assert_eq!(epochs, vec![0, 1, 2]);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sqwok_kc_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut kc = KeyChain::generate_new();
        kc.rotate();
        kc.save(&dir).unwrap();

        let loaded = KeyChain::load(&dir)
            .unwrap()
            .expect("keychain must be present");
        assert_eq!(loaded.current_epoch(), 1);
        assert_eq!(loaded.get(0).unwrap().key, kc.get(0).unwrap().key);
        assert_eq!(loaded.get(1).unwrap().key, kc.get(1).unwrap().key);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_returns_none_when_no_file() {
        let dir = std::env::temp_dir().join(format!("sqwok_empty_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(KeyChain::load(&dir).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_rejects_corrupted_data() {
        let dir = std::env::temp_dir().join(format!("sqwok_bad_kc_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // 5 bytes is not a multiple of 36
        std::fs::write(dir.join("keychain.bin"), [0u8; 5]).unwrap();
        assert!(KeyChain::load(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_from_epochs_sorts_input() {
        let keys = vec![
            EpochKey {
                epoch: 3,
                key: [3u8; 32],
            },
            EpochKey {
                epoch: 1,
                key: [1u8; 32],
            },
            EpochKey {
                epoch: 2,
                key: [2u8; 32],
            },
        ];
        let kc = KeyChain::from_epochs(keys);
        assert_eq!(kc.current_epoch(), 3);
        assert_eq!(kc.all_epochs()[0].epoch, 1);
    }
}
