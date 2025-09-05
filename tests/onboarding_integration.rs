mod common;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nrc::{AppState, OnboardingMode};
use std::time::Duration;

#[tokio::test]
async fn test_full_onboarding_flow_via_events() -> Result<()> {
    // Test new user onboarding entirely through events
    let client = TestClientUninitialized::new("test_user").await?;

    // Should start in onboarding state
    assert!(matches!(
        client.get_state().await,
        AppState::Onboarding {
            mode: OnboardingMode::Choose,
            ..
        }
    ));

    // Choose option 1 (new user)
    client
        .emit_key_event(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE))
        .await?;
    client
        .wait_for_condition(
            || async {
                matches!(
                    client.get_state().await,
                    AppState::Onboarding {
                        mode: OnboardingMode::EnterDisplayName,
                        ..
                    }
                )
            },
            Duration::from_secs(2),
        )
        .await?;

    // Enter display name
    for c in "TestUser".chars() {
        client
            .emit_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .await?;
    }

    // Submit
    client
        .emit_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .await?;

    // Should initialize and reach Ready state
    client
        .wait_for_condition(
            || async { matches!(client.get_state().await, AppState::Ready { .. }) },
            Duration::from_secs(10), // Initialization can be slow
        )
        .await?;

    Ok(())
}

#[tokio::test]
async fn test_import_existing_flow_via_events() -> Result<()> {
    // Test import existing user flow entirely through events
    let client = TestClientUninitialized::new("test_user").await?;

    // Should start in onboarding state
    assert!(matches!(
        client.get_state().await,
        AppState::Onboarding {
            mode: OnboardingMode::Choose,
            ..
        }
    ));

    // Choose option 2 (import existing)
    client
        .emit_key_event(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
        .await?;
    client
        .wait_for_condition(
            || async {
                matches!(
                    client.get_state().await,
                    AppState::Onboarding {
                        mode: OnboardingMode::ImportExisting,
                        ..
                    }
                )
            },
            Duration::from_secs(2),
        )
        .await?;

    // Enter a valid test nsec (we'll use a dummy one for testing)
    let test_nsec = "nsec1234567890abcdefghijklmnopqrstuvwxyz1234567890abcdefghijklmn"; // Invalid but length-appropriate
    for c in test_nsec.chars() {
        client
            .emit_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .await?;
    }

    // Submit - this will likely fail with the dummy nsec, but we're testing the event flow
    client
        .emit_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .await?;

    // Should try to initialize (might fail and return to choose mode due to invalid nsec)
    // The important thing is that the event flow worked
    client
        .wait_for_condition(
            || async {
                matches!(
                    client.get_state().await,
                    AppState::Ready { .. }
                        | AppState::Onboarding {
                            mode: OnboardingMode::Choose,
                            ..
                        }
                )
            },
            Duration::from_secs(5),
        )
        .await?;

    Ok(())
}

// Helper struct for uninitialized test client
use nrc::{AppEvent, Nrc};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

pub struct TestClientUninitialized {
    pub nrc: Arc<Mutex<Nrc>>,
    pub temp_dir: PathBuf,
    #[allow(dead_code)]
    event_tx: mpsc::UnboundedSender<AppEvent>,
    #[allow(dead_code)]
    event_rx: Arc<Mutex<mpsc::UnboundedReceiver<AppEvent>>>,
}

impl TestClientUninitialized {
    pub async fn new(name: &str) -> Result<Self> {
        // Create a unique temp directory for this client with random suffix
        let temp_dir =
            std::env::temp_dir().join(format!("nrc_test_{}_{}", name, rand::random::<u32>()));
        // Clean up if it exists and create fresh directory
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir)?;

        // Create Nrc instance - but DON'T initialize it (this is the key difference)
        let mut nrc = Nrc::new(&temp_dir).await?;

        // Create event channel for testing
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        nrc.event_tx = Some(event_tx.clone());

        // DON'T call initialize_with_display_name - leave it in onboarding state

        Ok(Self {
            nrc: Arc::new(Mutex::new(nrc)),
            temp_dir,
            event_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
        })
    }

    pub async fn get_state(&self) -> AppState {
        let nrc = self.nrc.lock().await;
        nrc.state.clone()
    }

    pub async fn emit_key_event(&self, key: KeyEvent) -> Result<()> {
        // First emit through event bus
        {
            let nrc = self.nrc.lock().await;
            let _ = nrc
                .event_bus()
                .emit(nrc::event_bus::UnifiedEvent::KeyPress(key));
        }

        // Process internal events to handle the key press
        self.process_internal_events().await?;

        Ok(())
    }

    pub async fn process_internal_events(&self) -> Result<()> {
        let mut nrc = self.nrc.lock().await;
        nrc.process_internal_events().await?;
        Ok(())
    }

    pub async fn wait_for_condition<F, Fut>(&self, condition: F, timeout: Duration) -> Result<()>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let start = tokio::time::Instant::now();

        while !condition().await && start.elapsed() < timeout {
            // Process internal events from event bus
            self.process_internal_events().await?;

            // Small delay between checks
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        if !condition().await {
            return Err(anyhow::anyhow!(
                "Condition not met within timeout of {:?}",
                timeout
            ));
        }

        Ok(())
    }
}

impl Drop for TestClientUninitialized {
    fn drop(&mut self) {
        // Clean up temp directory
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}
