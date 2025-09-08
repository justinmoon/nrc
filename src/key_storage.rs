use anyhow::{anyhow, Result};
use nostr_sdk::prelude::*;
use std::fs;
use std::path::Path;

/// Manages encrypted key storage using NIP-49
pub struct KeyStorage {
    keys_path: std::path::PathBuf,
}

impl KeyStorage {
    pub fn new(datadir: &Path) -> Self {
        Self {
            keys_path: datadir.join("keys.ncryptsec"),
        }
    }

    /// Check if encrypted keys exist
    pub fn keys_exist(&self) -> bool {
        self.keys_path.exists()
    }

    /// Save keys encrypted with password using NIP-49
    pub fn save_encrypted(&self, keys: &Keys, password: &str) -> Result<()> {
        // Use the encrypt method to create an EncryptedSecretKey
        let encrypted = keys.secret_key().encrypt(password)?;
        // Convert to bech32 string for storage
        let encrypted_str = encrypted.to_bech32()?;
        fs::write(&self.keys_path, encrypted_str)?;
        log::info!("Keys saved to {:?}", self.keys_path);
        Ok(())
    }

    /// Load and decrypt keys with password
    pub fn load_encrypted(&self, password: &str) -> Result<Keys> {
        let encrypted_str = fs::read_to_string(&self.keys_path)?;
        // Parse the encrypted key from bech32
        let encrypted = EncryptedSecretKey::from_bech32(&encrypted_str)?;
        // Decrypt to get the secret key
        let secret_key = encrypted
            .decrypt(password)
            .map_err(|e| anyhow!("Failed to decrypt keys: {}", e))?;
        Ok(Keys::new(secret_key))
    }

    /// Delete stored keys
    pub fn delete(&self) -> Result<()> {
        if self.keys_path.exists() {
            fs::remove_file(&self.keys_path)?;
            log::info!("Deleted keys at {:?}", self.keys_path);
        }
        Ok(())
    }
}
