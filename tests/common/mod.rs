use anyhow::Result;
use nostr_sdk::prelude::*;
use nrc::Nrc;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Test client that wraps Nrc for testing
pub struct TestClient {
    pub nrc: Arc<Mutex<Nrc>>,
    pub temp_dir: PathBuf,
}

impl TestClient {
    pub async fn new(name: &str) -> Result<Self> {
        // Create a unique temp directory for this client
        let temp_dir = std::env::temp_dir().join(format!("nrc_test_{name}"));
        std::fs::create_dir_all(&temp_dir)?;

        // Create Nrc instance with memory storage
        let mut nrc = Nrc::new(&temp_dir, true).await?;

        // Initialize with display name
        nrc.initialize_with_display_name(name.to_string()).await?;

        Ok(Self {
            nrc: Arc::new(Mutex::new(nrc)),
            temp_dir,
        })
    }

    /// Get the npub for this client
    pub async fn npub(&self) -> Result<String> {
        let nrc = self.nrc.lock().await;
        Ok(nrc.public_key().to_bech32()?)
    }

    /// Execute a command as if typed by user
    pub async fn execute_command(&self, command: &str) -> Result<()> {
        let mut nrc = self.nrc.lock().await;
        nrc.process_input(command.to_string()).await?;
        Ok(())
    }

    /// Check if welcome rumor exists for a pubkey
    pub async fn has_welcome_rumor_for(&self, pubkey: &PublicKey) -> bool {
        let nrc = self.nrc.lock().await;
        nrc.welcome_rumors.contains_key(pubkey)
    }

    /// Get the number of groups this client is in
    pub async fn group_count(&self) -> usize {
        let nrc = self.nrc.lock().await;
        nrc.get_groups().len()
    }
}

impl Drop for TestClient {
    fn drop(&mut self) {
        // Clean up temp directory
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}
