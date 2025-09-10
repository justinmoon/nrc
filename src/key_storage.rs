use anyhow::{anyhow, Result};
use nostr_sdk::prelude::*;
use rusqlite::{params, Connection};
use std::path::Path;

/// Manages encrypted key storage using NIP-49 in SQLite
pub struct KeyStorage {
    db_path: std::path::PathBuf,
}

impl KeyStorage {
    pub fn new(datadir: &Path) -> Self {
        Self {
            db_path: datadir.join("nrc.db"),
        }
    }

    pub fn datadir(&self) -> &Path {
        self.db_path.parent().unwrap_or_else(|| Path::new("."))
    }

    /// Initialize the keys table if it doesn't exist
    fn init_table(&self, conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS keys (
                id INTEGER PRIMARY KEY,
                npub TEXT NOT NULL UNIQUE,
                encrypted_nsec TEXT NOT NULL
            )",
            [],
        )?;
        Ok(())
    }

    /// Check if any encrypted keys exist
    pub fn keys_exist(&self) -> bool {
        self.keys_exist_internal().unwrap_or(false)
    }

    fn keys_exist_internal(&self) -> Result<bool> {
        let conn = Connection::open(&self.db_path)?;
        self.init_table(&conn)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM keys", [], |row| row.get(0))?;
        Ok(count > 0)
    }

    /// Get the first available account npub (for loading on startup)
    pub fn get_first_npub(&self) -> Result<String> {
        let conn = Connection::open(&self.db_path)?;
        let npub: String =
            conn.query_row("SELECT npub FROM keys ORDER BY id LIMIT 1", [], |row| {
                row.get(0)
            })?;
        Ok(npub)
    }

    /// Save keys encrypted with password using NIP-49
    pub fn save_encrypted(&self, keys: &Keys, password: &str) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        self.init_table(&conn)?;

        // Get the npub from the keys
        let npub = keys.public_key().to_bech32()?;

        // Use the encrypt method to create an EncryptedSecretKey
        let encrypted = keys.secret_key().encrypt(password)?;
        // Convert to bech32 string for storage
        let encrypted_str = encrypted.to_bech32()?;

        // Insert or replace the account with the npub
        conn.execute(
            "INSERT OR REPLACE INTO keys (npub, encrypted_nsec) VALUES (?1, ?2)",
            params![npub, encrypted_str],
        )?;

        log::info!("Keys saved to database for npub: {npub}");
        Ok(())
    }

    /// Load and decrypt keys with password
    pub fn load_encrypted(&self, password: &str) -> Result<Keys> {
        let conn = Connection::open(&self.db_path)?;

        // Get the first account (for now, until we have account selection)
        let encrypted_str: String = conn.query_row(
            "SELECT encrypted_nsec FROM keys ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )?;

        // Parse the encrypted key from bech32
        let encrypted = EncryptedSecretKey::from_bech32(&encrypted_str)?;
        // Decrypt to get the secret key
        let secret_key = encrypted
            .decrypt(password)
            .map_err(|e| anyhow!("Failed to decrypt keys: {}", e))?;
        Ok(Keys::new(secret_key))
    }

    /// Delete stored keys for a specific npub
    pub fn delete_by_npub(&self, npub: &str) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        conn.execute("DELETE FROM keys WHERE npub = ?1", params![npub])?;
        log::info!("Deleted keys for npub: {npub}");
        Ok(())
    }
}
