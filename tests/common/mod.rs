use anyhow::Result;
use nostr_sdk::prelude::*;
use nrc::{AppEvent, AppState, NetworkCommand, Nrc};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::sync::Mutex;

/// Test client that wraps Nrc with event channels for testing
pub struct TestClient {
    pub nrc: Arc<Mutex<Nrc>>,
    pub event_tx: mpsc::UnboundedSender<AppEvent>,
    pub event_rx: Arc<Mutex<mpsc::UnboundedReceiver<AppEvent>>>,
    pub command_tx: mpsc::Sender<NetworkCommand>,
    pub name: String,
    pub temp_dir: PathBuf,
}

impl TestClient {
    pub async fn new(name: &str) -> Result<Self> {
        // Create a unique temp directory for this client
        let temp_dir = std::env::temp_dir().join(format!("nrc_test_{}", name));
        std::fs::create_dir_all(&temp_dir)?;

        // Create Nrc instance with memory storage
        let mut nrc = Nrc::new(&temp_dir, true).await?;

        // Create event channels
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (command_tx, _command_rx) = mpsc::channel(100);

        // Set up channels in Nrc
        nrc.event_tx = Some(event_tx.clone());
        nrc.command_tx = Some(command_tx.clone());

        // Initialize with display name
        nrc.initialize_with_display_name(name.to_string()).await?;

        Ok(Self {
            nrc: Arc::new(Mutex::new(nrc)),
            event_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
            command_tx,
            name: name.to_string(),
            temp_dir,
        })
    }

    /// Get the npub for this client
    pub async fn npub(&self) -> Result<String> {
        let nrc = self.nrc.lock().await;
        Ok(nrc.public_key().to_bech32()?)
    }

    /// Send an event to this client
    pub async fn send_event(&self, event: AppEvent) -> Result<()> {
        self.event_tx.send(event)?;
        Ok(())
    }

    /// Process events until a condition is met
    #[allow(dead_code)]
    pub async fn process_events_until<F>(&self, condition: F, timeout: Duration) -> Result<()>
    where
        F: Fn(&AppEvent) -> bool,
    {
        let deadline = Instant::now() + timeout;

        while Instant::now() < deadline {
            let mut rx = self.event_rx.lock().await;

            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Some(event)) => {
                    // Process the event
                    self.process_event(event.clone()).await?;

                    if condition(&event) {
                        return Ok(());
                    }
                }
                Ok(None) => {
                    // Channel closed
                    return Err(anyhow::anyhow!("Event channel closed"));
                }
                Err(_) => {
                    // Timeout, continue loop
                    continue;
                }
            }
        }

        Err(anyhow::anyhow!("Timeout waiting for condition"))
    }

    /// Process a single event like the main loop does
    #[allow(dead_code)]
    async fn process_event(&self, event: AppEvent) -> Result<()> {
        let mut nrc = self.nrc.lock().await;

        match event {
            AppEvent::MessageReceived { group_id, message } => {
                nrc.add_message(group_id, message);
            }
            AppEvent::GroupCreated { group_id } => {
                nrc.add_group(group_id);
            }
            AppEvent::NetworkError { error } => {
                nrc.last_error = Some(error);
            }
            AppEvent::KeyPackagePublished => {
                if let AppState::Ready { groups, .. } = &nrc.state {
                    nrc.state = AppState::Ready {
                        key_package_published: true,
                        groups: groups.clone(),
                    };
                }
            }
            _ => {}
        }

        Ok(())
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
