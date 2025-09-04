use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use nostr_sdk::prelude::*;
use nrc::{AppEvent, Nrc};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Test client that wraps Nrc for testing
pub struct TestClient {
    pub nrc: Arc<Mutex<Nrc>>,
    pub temp_dir: PathBuf,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    event_rx: Arc<Mutex<mpsc::UnboundedReceiver<AppEvent>>>,
}

#[allow(dead_code)] // Methods are used across different test files
impl TestClient {
    pub async fn new(name: &str) -> Result<Self> {
        // Create a unique temp directory for this client
        let temp_dir = std::env::temp_dir().join(format!("nrc_test_{name}"));
        std::fs::create_dir_all(&temp_dir)?;

        // Create Nrc instance with memory storage
        let mut nrc = Nrc::new(&temp_dir, true).await?;

        // Create event channel for testing
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        nrc.event_tx = Some(event_tx.clone());

        // Initialize through onboarding flow - but for tests, skip the UI flow
        // and directly call the initialization
        nrc.initialize_with_display_name(name.to_string()).await?;

        Ok(Self {
            nrc: Arc::new(Mutex::new(nrc)),
            temp_dir,
            event_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
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

    /// Send a fetch messages tick event (simulates timer)
    pub fn trigger_fetch_messages(&self) -> Result<()> {
        self.event_tx.send(AppEvent::FetchMessagesTick)?;
        Ok(())
    }

    /// Send a fetch welcomes tick event (simulates timer)
    pub fn trigger_fetch_welcomes(&self) -> Result<()> {
        self.event_tx.send(AppEvent::FetchWelcomesTick)?;
        Ok(())
    }


    /// Navigate to previous group using arrow key
    pub fn send_arrow_up(&self) -> Result<()> {
        self.event_tx
            .send(AppEvent::KeyPress(KeyEvent::from(KeyCode::Up)))?;
        Ok(())
    }

    /// Select first group if multiple groups exist
    pub async fn select_first_group(&self) -> Result<()> {
        let nrc = self.nrc.lock().await;
        let group_count = nrc.get_groups().len();
        drop(nrc);

        // Navigate to first group (index 0) if we have groups
        if group_count > 0 {
            // Reset to first group by sending up arrows to wrap around
            for _ in 0..group_count {
                self.send_arrow_up()?;
            }
        }
        Ok(())
    }


    /// Process events from the event queue (simulates event loop)
    pub async fn process_pending_events(&self) -> Result<()> {
        let mut event_rx = self.event_rx.lock().await;
        let mut nrc = self.nrc.lock().await;

        // Process all pending events
        while let Ok(event) = event_rx.try_recv() {
            match event {
                AppEvent::KeyPress(key) => {
                    // Handle navigation keys
                    match key.code {
                        KeyCode::Up if nrc.input.is_empty() => {
                            nrc.prev_group();
                        }
                        KeyCode::Down if nrc.input.is_empty() => {
                            nrc.next_group();
                        }
                        _ => {}
                    }
                }
                AppEvent::FetchMessagesTick => {
                    // In tests, directly call the fetch since we don't have background tasks
                    nrc.fetch_and_process_messages().await?;
                }
                AppEvent::FetchWelcomesTick => {
                    // In tests, directly call the fetch since we don't have background tasks
                    nrc.fetch_and_process_welcomes().await?;
                }
                AppEvent::RawMessagesReceived { events } => {
                    for event in events {
                        if let Err(e) = nrc.process_message_event(event).await {
                            log::debug!("Failed to process message: {e}");
                        }
                    }
                }
                AppEvent::RawWelcomesReceived { events } => {
                    for event in events {
                        if let Err(e) = nrc.process_welcome_event(event).await {
                            log::debug!("Failed to process welcome: {e}");
                        }
                    }
                }
                AppEvent::MessageReceived { group_id, message } => {
                    nrc.add_message(group_id, message);
                }
                AppEvent::GroupCreated { group_id } => {
                    nrc.add_group(group_id);
                }
                _ => {}
            }
        }

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
