use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nostr_sdk::prelude::*;
use nrc::key_storage::KeyStorage;
use nrc::ui_state::OnboardingMode;
use nrc::{App, AppEvent, Page};
use nrc_mls::NostrMls;
use nrc_mls_sqlite_storage::NostrMlsSqliteStorage;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Minimal test wrapper for the App
struct TestApp {
    app: App,
    temp_dir: PathBuf,
    event_log: Arc<Mutex<Vec<(String, Event)>>>, // (timestamp, event) log of all Nostr events
}

impl TestApp {
    async fn new() -> Result<Self> {
        // Create unique temp directory
        let temp_dir = std::env::temp_dir().join(format!("nrc_test_{}", rand::random::<u32>()));
        std::fs::create_dir_all(&temp_dir)?;

        // Set up storage
        let db_path = temp_dir.join("test.db");
        let storage = Arc::new(NostrMls::new(NostrMlsSqliteStorage::new(db_path)?));

        // Create client
        let client = Client::default();

        // Generate test keys
        let keys = Keys::generate();

        // Create key storage
        let key_storage = KeyStorage::new(&temp_dir);

        // Create app with initial onboarding page
        let initial_page = Page::Onboarding {
            input: String::new(),
            mode: OnboardingMode::Choose,
            error: None,
        };

        // Register event log for this client (test only) - do this before moving keys
        let client_id = keys
            .public_key()
            .to_bech32()
            .unwrap_or_else(|_| keys.public_key().to_hex());
        let event_log =
            nrc::notification_handler::test_helpers::register_test_event_log(client_id).await;

        // Create app first
        let app = App::new(storage, client.clone(), keys, key_storage, initial_page).await?;

        Ok(Self {
            app,
            temp_dir,
            event_log,
        })
    }

    async fn with_existing_user(password: &str) -> Result<Self> {
        // Create unique temp directory
        let temp_dir = std::env::temp_dir().join(format!("nrc_test_{}", rand::random::<u32>()));
        std::fs::create_dir_all(&temp_dir)?;

        // Set up storage
        let db_path = temp_dir.join("test.db");
        let storage = Arc::new(NostrMls::new(NostrMlsSqliteStorage::new(db_path)?));

        // Create client
        let client = Client::default();

        // Generate test keys
        let keys = Keys::generate();

        // Create key storage and save encrypted keys
        let key_storage = KeyStorage::new(&temp_dir);
        key_storage.save_encrypted(&keys, password)?;

        // Create app starting at password entry (existing user flow)
        let initial_page = Page::Onboarding {
            input: String::new(),
            mode: OnboardingMode::EnterPassword,
            error: None,
        };

        // Register event log for this client (test only) - do this before moving keys
        let client_id = keys
            .public_key()
            .to_bech32()
            .unwrap_or_else(|_| keys.public_key().to_hex());
        let event_log =
            nrc::notification_handler::test_helpers::register_test_event_log(client_id).await;

        // Create app first
        let app = App::new(storage, client.clone(), keys, key_storage, initial_page).await?;

        Ok(Self {
            app,
            temp_dir,
            event_log,
        })
    }

    async fn send_key(&mut self, c: char) -> Result<()> {
        let event = AppEvent::KeyPress(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()));
        self.app.handle_event(event).await
    }

    async fn send_keys(&mut self, s: &str) -> Result<()> {
        for c in s.chars() {
            self.send_key(c).await?;
        }
        Ok(())
    }

    async fn send_enter(&mut self) -> Result<()> {
        let event = AppEvent::KeyPress(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
        self.app.handle_event(event).await
    }

    async fn send_backspace(&mut self) -> Result<()> {
        let event = AppEvent::KeyPress(KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()));
        self.app.handle_event(event).await
    }

    async fn send_ctrl_n(&mut self) -> Result<()> {
        let event = AppEvent::KeyPress(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        self.app.handle_event(event).await
    }

    async fn process_incoming_messages(&mut self) -> Result<()> {
        // Process any pending events from the event receiver
        // In a real app, there would be a background task processing these
        // For tests, we manually drain the queue
        if let Some(mut rx) = self.app.event_rx.take() {
            while let Ok(event) = rx.try_recv() {
                self.app.handle_event(event).await?;
            }
            // Put it back
            self.app.event_rx = Some(rx);
        }

        // Give a bit of time for any async processing
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Refresh current page to pick up any new state
        if let Err(e) = self.app.send_event(AppEvent::RefreshCurrentPage) {
            log::warn!("RefreshCurrentPage event failed: {}", e);
        }
        
        // Process the refresh event we just sent
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Some(mut rx) = self.app.event_rx.take() {
            while let Ok(event) = rx.try_recv() {
                self.app.handle_event(event).await?;
            }
            self.app.event_rx = Some(rx);
        }

        Ok(())
    }

    async fn send_message(&mut self, message: &str) -> Result<()> {
        // Send a message in the current chat
        self.send_keys(message).await?;
        self.send_enter().await?;
        Ok(())
    }

    fn has_messages(&self) -> bool {
        match &self.app.current_page {
            Page::Chat { messages, .. } => !messages.is_empty(),
            _ => false,
        }
    }

    fn get_last_message(&self) -> Option<String> {
        match &self.app.current_page {
            Page::Chat { messages, .. } => messages.last().map(|m| m.content.clone()),
            _ => None,
        }
    }

    fn has_group(&self) -> bool {
        match &self.app.current_page {
            Page::Chat { groups, .. } => !groups.is_empty(),
            _ => false,
        }
    }

    fn get_npub(&self) -> String {
        self.app.keys.public_key().to_bech32().unwrap()
    }

    async fn dump_event_log(&self) {
        let log = self.event_log.lock().await;
        println!("\n=== Nostr Event Log ({} events) ===", log.len());
        for (timestamp, event) in log.iter() {
            println!(
                "[{}] Kind: {:?}, From: {}, ID: {}",
                timestamp,
                event.kind,
                event
                    .pubkey
                    .to_bech32()
                    .unwrap_or_else(|_| "unknown".to_string()),
                event
                    .id
                    .to_bech32()
                    .unwrap_or_else(|_| "unknown".to_string())
            );
            if event.kind == Kind::from(443u16) {
                println!("  ^ Key Package");
            } else if event.kind == Kind::GiftWrap {
                println!("  ^ GiftWrap (welcome or DM)");
            } else if event.kind == Kind::from(444u16) {
                println!("  ^ Welcome");
            } else if event.kind == Kind::from(445u16) {
                println!("  ^ MLS Message");
            }
        }
        println!("=== End Event Log ===\n");
    }

    async fn log_event(&self, event: Event) {
        let timestamp = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
        let mut log = self.event_log.lock().await;
        log.push((timestamp, event));
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        // Cleanup temp directory
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

#[tokio::test]
async fn test_new_user_complete_journey() -> Result<()> {
    let mut app = TestApp::new().await?;

    // Should start at onboarding choose mode
    match &app.app.current_page {
        Page::Onboarding {
            mode: OnboardingMode::Choose,
            ..
        } => {}
        _ => panic!(
            "Expected Onboarding Choose mode, got {:?}",
            app.app.current_page
        ),
    }

    // Choose to generate new keys (option 1)
    app.send_key('1').await?;
    app.send_enter().await?;

    // Should transition through GenerateNew briefly, then to EnterDisplayName
    // The GenerateNew mode is just a transitional state
    match &app.app.current_page {
        Page::Onboarding {
            mode: OnboardingMode::GenerateNew,
            ..
        }
        | Page::Onboarding {
            mode: OnboardingMode::EnterDisplayName,
            ..
        } => {}
        _ => panic!(
            "Expected GenerateNew or EnterDisplayName mode, got {:?}",
            app.app.current_page
        ),
    }

    // If we're still in GenerateNew, need to press Enter or wait
    if matches!(
        &app.app.current_page,
        Page::Onboarding {
            mode: OnboardingMode::GenerateNew,
            ..
        }
    ) {
        // GenerateNew is a transitional state, press Enter to continue
        app.send_enter().await?;

        // Now should be in EnterDisplayName
        match &app.app.current_page {
            Page::Onboarding {
                mode: OnboardingMode::EnterDisplayName,
                ..
            } => {}
            _ => panic!(
                "Expected EnterDisplayName after GenerateNew, got {:?}",
                app.app.current_page
            ),
        }
    }

    // Enter display name
    app.send_keys("Alice").await?;

    // Verify input contains the name
    match &app.app.current_page {
        Page::Onboarding {
            input,
            mode: OnboardingMode::EnterDisplayName,
            ..
        } => {
            assert_eq!(input, "Alice");
        }
        _ => panic!(
            "Expected to still be in EnterDisplayName, got {:?}",
            app.app.current_page
        ),
    }

    // Submit display name
    app.send_enter().await?;

    // Should now be at password creation
    match &app.app.current_page {
        Page::Onboarding {
            mode: OnboardingMode::CreatePassword,
            ..
        } => {}
        _ => panic!(
            "Expected CreatePassword mode, got {:?}",
            app.app.current_page
        ),
    }

    // Enter password
    app.send_keys("password123").await?;
    app.send_enter().await?;

    // After initialization, should be at GroupList (may go through Initializing first)
    // Give it a moment to complete initialization
    tokio::time::sleep(Duration::from_millis(100)).await;

    match &app.app.current_page {
        Page::Chat { .. } => {
            // Successfully completed onboarding!
        }
        Page::Initializing { .. } => {
            // Wait a bit more for initialization to complete
            tokio::time::sleep(Duration::from_millis(500)).await;
            match &app.app.current_page {
                Page::Chat { .. } => {
                    // Successfully completed onboarding!
                }
                _ => panic!(
                    "Expected GroupList after initialization, got {:?}",
                    app.app.current_page
                ),
            }
        }
        _ => panic!(
            "Expected GroupList or Initializing, got {:?}",
            app.app.current_page
        ),
    }

    Ok(())
}

#[tokio::test]
async fn test_existing_user_flow() -> Result<()> {
    let password = "testpassword123";
    let mut app = TestApp::with_existing_user(password).await?;

    // Should start at password entry for existing user
    match &app.app.current_page {
        Page::Onboarding {
            mode: OnboardingMode::EnterPassword,
            ..
        } => {}
        _ => panic!(
            "Expected EnterPassword mode for existing user, got {:?}",
            app.app.current_page
        ),
    }

    // Try wrong password first
    app.send_keys("wrongpassword").await?;
    app.send_enter().await?;

    // Should show error
    match &app.app.current_page {
        Page::Onboarding {
            error: Some(err),
            mode: OnboardingMode::EnterPassword,
            ..
        } => {
            // Error message should indicate invalid password
            assert!(
                err.contains("password") || err.contains("Password"),
                "Expected password error, got: {}",
                err
            );
        }
        _ => panic!("Expected password error, got {:?}", app.app.current_page),
    }

    // Clear input and enter correct password
    for _ in 0..13 {
        // length of "wrongpassword"
        app.send_backspace().await?;
    }

    app.send_keys(password).await?;
    app.send_enter().await?;

    // Should proceed to initialization/group list
    tokio::time::sleep(Duration::from_millis(100)).await;

    match &app.app.current_page {
        Page::Chat { .. } => {
            // Successfully logged in!
        }
        Page::Initializing { .. } => {
            // Wait for initialization to complete
            tokio::time::sleep(Duration::from_millis(500)).await;
            match &app.app.current_page {
                Page::Chat { .. } => {
                    // Successfully logged in!
                }
                _ => panic!(
                    "Expected GroupList after login, got {:?}",
                    app.app.current_page
                ),
            }
        }
        _ => panic!(
            "Expected GroupList or Initializing after correct password, got {:?}",
            app.app.current_page
        ),
    }

    Ok(())
}

#[tokio::test]
async fn test_two_users_messaging() -> Result<()> {
    // Initialize logger for debugging
    let _ = env_logger::builder().is_test(true).try_init();

    // Create two new users
    let mut alice = TestApp::new().await?;
    let mut bob = TestApp::new().await?;

    // Alice completes onboarding
    alice.send_key('1').await?; // Generate new
    alice.send_enter().await?;
    alice.send_enter().await?; // Transition through GenerateNew
    alice.send_keys("Alice").await?;
    alice.send_enter().await?;
    alice.send_keys("alicepass123").await?;
    alice.send_enter().await?;

    // Wait for Alice to reach GroupList
    tokio::time::sleep(Duration::from_millis(2500)).await;

    // Bob completes onboarding
    bob.send_key('1').await?; // Generate new
    bob.send_enter().await?;
    bob.send_enter().await?; // Transition through GenerateNew
    bob.send_keys("Bob").await?;
    bob.send_enter().await?;
    bob.send_keys("bobpass123").await?;
    bob.send_enter().await?;

    // Wait for Bob to reach GroupList and publish key package
    tokio::time::sleep(Duration::from_millis(3000)).await;

    // Verify both are at Chat page
    match &alice.app.current_page {
        Page::Chat { .. } => {}
        _ => panic!(
            "Alice should be at Chat page, got {:?}",
            alice.app.current_page
        ),
    }

    match &bob.app.current_page {
        Page::Chat { .. } => {}
        _ => panic!("Bob should be at Chat page, got {:?}", bob.app.current_page),
    }

    // Verify both start with no groups
    assert!(!alice.has_group(), "Alice should have no groups initially");
    assert!(!bob.has_group(), "Bob should have no groups initially");

    // Get Bob's public key (Alice gets this out-of-band)
    let bob_npub = bob.get_npub();
    println!("Bob's npub: {}", bob_npub);

    // IMPORTANT: Give Bob's key package time to propagate to relays
    println!("Waiting for Bob's key package to propagate to relays...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Alice runs /dm command to create group with Bob
    println!("Alice creating DM with Bob...");
    alice.send_keys(&format!("/dm {}", bob_npub)).await?;
    alice.send_enter().await?;

    // Wait for Alice's /dm command to complete
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Check if Alice successfully created the group
    let alice_created_group = alice.has_group();
    println!("Alice created group: {}", alice_created_group);

    if alice_created_group {
        println!("✓ Alice successfully created DM group with Bob");

        // Dump Alice's event log to see what happened
        alice.dump_event_log().await;

        // Bob should process the incoming welcome message
        println!("Bob processing incoming welcome...");
        bob.process_incoming_messages().await?;

        // Dump Bob's event log to see what he received
        bob.dump_event_log().await;

        let bob_joined_group = bob.has_group();
        println!("Bob joined group: {}", bob_joined_group);

        assert!(bob_joined_group, "Bob MUST join the group for test to pass");
        println!("✓ Bob successfully joined the DM group");

        // Bob needs to navigate to the chat to see messages
        // He's currently on Chat page but not viewing the new group
        // Let's get the group ID and navigate to it
        let bob_groups = match &bob.app.current_page {
            Page::Chat { groups, .. } => groups.clone(),
            _ => vec![],
        };
        assert!(!bob_groups.is_empty(), "Bob should have at least one group");
        
        // Navigate Bob to the first (and only) group
        let group_id = bob_groups[0].id.clone();
        bob.app.navigate_to(nrc::ui_state::PageType::Chat(Some(group_id))).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        println!("Bob navigated to the DM chat");

        // Test bidirectional messaging
        println!("\n--- Testing bidirectional messaging ---");

        // Alice sends first message
        println!("Alice sending: 'Hello Bob!'");
        alice.send_message("Hello Bob!").await?;
        
        // Give time to send
        tokio::time::sleep(Duration::from_millis(1000)).await;
        
        // Process any pending events on Alice side (to complete the send)
        alice.process_incoming_messages().await?;

        // Wait for message to propagate
        println!("Waiting for message to propagate to relays...");
        tokio::time::sleep(Duration::from_millis(3000)).await;

        // Bob processes incoming messages
        println!("Bob processing incoming messages...");
        bob.process_incoming_messages().await?;
        tokio::time::sleep(Duration::from_millis(1000)).await;

        // Check if Bob received Alice's message
        let bob_received = bob.get_last_message();
        assert!(bob_received.is_some(), "Bob MUST receive Alice's message");
        assert_eq!(
            bob_received.unwrap(), "Hello Bob!",
            "Bob should have received Alice's exact message"
        );
        println!("✓ Bob received: 'Hello Bob!'");

        // Bob sends reply
        println!("Bob sending: 'Hey Alice!'");
        bob.send_message("Hey Alice!").await?;
        
        // Give time to send
        tokio::time::sleep(Duration::from_millis(1000)).await;
        
        // Process any pending events on Bob side (to complete the send)
        bob.process_incoming_messages().await?;

        // Wait for message to propagate
        println!("Waiting for Bob's message to propagate to relays...");
        tokio::time::sleep(Duration::from_millis(3000)).await;

        // Alice processes incoming messages
        println!("Alice processing incoming messages...");
        alice.process_incoming_messages().await?;
        tokio::time::sleep(Duration::from_millis(1000)).await;

        // Check if Alice received Bob's message
        // Alice's messages should now be ["Hello Bob!", "Hey Alice!"]
        let alice_messages = match &alice.app.current_page {
            Page::Chat { messages, .. } => messages.clone(),
            _ => vec![],
        };
        
        assert!(alice_messages.len() >= 2, "Alice should have at least 2 messages");
        let bob_message = alice_messages.iter().find(|m| m.content == "Hey Alice!");
        assert!(bob_message.is_some(), "Alice MUST receive Bob's message 'Hey Alice!'");
        println!("✓ Alice received: 'Hey Alice!'");

        println!("✓✓✓ Bidirectional messaging test PASSED ✓✓✓");
    } else {
        println!("✗ Alice failed to create group");

        // Dump event logs to debug why group creation failed
        println!("\nDumping event logs for debugging:");
        alice.dump_event_log().await;
        bob.dump_event_log().await;

        panic!("Alice should have been able to create the group!");
    }

    // Summary assertions - both users should be functional
    assert!(
        matches!(alice.app.current_page, Page::Chat { .. }),
        "Alice should be at Chat page"
    );
    assert!(
        matches!(bob.app.current_page, Page::Chat { .. }),
        "Bob should be at Chat page"
    );

    Ok(())
}
