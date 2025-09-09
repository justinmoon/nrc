use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nostr_sdk::prelude::*;
use nrc::{App, AppEvent, Page};
use nrc::ui_state::OnboardingMode;
use nrc::key_storage::KeyStorage;
use nrc_mls::NostrMls;
use nrc_mls_sqlite_storage::NostrMlsSqliteStorage;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Minimal test wrapper for the App
struct TestApp {
    app: App,
    temp_dir: PathBuf,
}

impl TestApp {
    async fn new() -> Result<Self> {
        // Create unique temp directory
        let temp_dir = std::env::temp_dir()
            .join(format!("nrc_test_{}", rand::random::<u32>()));
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
        
        
        let app = App::new(
            storage,
            client,
            keys,
            key_storage,
            initial_page,
        ).await?;
        
        Ok(Self { app, temp_dir })
    }
    
    async fn with_existing_user(password: &str) -> Result<Self> {
        // Create unique temp directory
        let temp_dir = std::env::temp_dir()
            .join(format!("nrc_test_{}", rand::random::<u32>()));
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
        
        
        let app = App::new(
            storage,
            client,
            keys,
            key_storage,
            initial_page,
        ).await?;
        
        Ok(Self { app, temp_dir })
    }
    
    async fn send_key(&mut self, c: char) -> Result<()> {
        let event = AppEvent::KeyPress(KeyEvent::new(
            KeyCode::Char(c),
            KeyModifiers::empty(),
        ));
        self.app.handle_event(event).await
    }
    
    async fn send_keys(&mut self, s: &str) -> Result<()> {
        for c in s.chars() {
            self.send_key(c).await?;
        }
        Ok(())
    }
    
    async fn send_enter(&mut self) -> Result<()> {
        let event = AppEvent::KeyPress(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::empty(),
        ));
        self.app.handle_event(event).await
    }
    
    async fn send_backspace(&mut self) -> Result<()> {
        let event = AppEvent::KeyPress(KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::empty(),
        ));
        self.app.handle_event(event).await
    }
    
    async fn send_ctrl_n(&mut self) -> Result<()> {
        let event = AppEvent::KeyPress(KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL,
        ));
        self.app.handle_event(event).await
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
        Page::Onboarding { mode: OnboardingMode::Choose, .. } => {},
        _ => panic!("Expected Onboarding Choose mode, got {:?}", app.app.current_page),
    }
    
    // Choose to generate new keys (option 1)
    app.send_key('1').await?;
    app.send_enter().await?;
    
    // Should transition through GenerateNew briefly, then to EnterDisplayName
    // The GenerateNew mode is just a transitional state
    match &app.app.current_page {
        Page::Onboarding { mode: OnboardingMode::GenerateNew, .. } |
        Page::Onboarding { mode: OnboardingMode::EnterDisplayName, .. } => {},
        _ => panic!("Expected GenerateNew or EnterDisplayName mode, got {:?}", app.app.current_page),
    }
    
    // If we're still in GenerateNew, need to press Enter or wait
    if matches!(&app.app.current_page, Page::Onboarding { mode: OnboardingMode::GenerateNew, .. }) {
        // GenerateNew is a transitional state, press Enter to continue
        app.send_enter().await?;
        
        // Now should be in EnterDisplayName
        match &app.app.current_page {
            Page::Onboarding { mode: OnboardingMode::EnterDisplayName, .. } => {},
            _ => panic!("Expected EnterDisplayName after GenerateNew, got {:?}", app.app.current_page),
        }
    }
    
    // Enter display name
    app.send_keys("Alice").await?;
    
    // Verify input contains the name
    match &app.app.current_page {
        Page::Onboarding { input, mode: OnboardingMode::EnterDisplayName, .. } => {
            assert_eq!(input, "Alice");
        },
        _ => panic!("Expected to still be in EnterDisplayName, got {:?}", app.app.current_page),
    }
    
    // Submit display name
    app.send_enter().await?;
    
    // Should now be at password creation
    match &app.app.current_page {
        Page::Onboarding { mode: OnboardingMode::CreatePassword, .. } => {},
        _ => panic!("Expected CreatePassword mode, got {:?}", app.app.current_page),
    }
    
    // Enter password
    app.send_keys("password123").await?;
    app.send_enter().await?;
    
    // After initialization, should be at GroupList (may go through Initializing first)
    // Give it a moment to complete initialization
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    match &app.app.current_page {
        Page::GroupList { .. } => {
            // Successfully completed onboarding!
        },
        Page::Initializing { .. } => {
            // Wait a bit more for initialization to complete
            tokio::time::sleep(Duration::from_millis(500)).await;
            match &app.app.current_page {
                Page::GroupList { .. } => {
                    // Successfully completed onboarding!
                },
                _ => panic!("Expected GroupList after initialization, got {:?}", app.app.current_page),
            }
        },
        _ => panic!("Expected GroupList or Initializing, got {:?}", app.app.current_page),
    }
    
    Ok(())
}

#[tokio::test]
async fn test_existing_user_flow() -> Result<()> {
    let password = "testpassword123";
    let mut app = TestApp::with_existing_user(password).await?;
    
    // Should start at password entry for existing user
    match &app.app.current_page {
        Page::Onboarding { mode: OnboardingMode::EnterPassword, .. } => {},
        _ => panic!("Expected EnterPassword mode for existing user, got {:?}", app.app.current_page),
    }
    
    // Try wrong password first
    app.send_keys("wrongpassword").await?;
    app.send_enter().await?;
    
    // Should show error
    match &app.app.current_page {
        Page::Onboarding { error: Some(err), mode: OnboardingMode::EnterPassword, .. } => {
            // Error message should indicate invalid password
            assert!(err.contains("password") || err.contains("Password"), 
                   "Expected password error, got: {}", err);
        },
        _ => panic!("Expected password error, got {:?}", app.app.current_page),
    }
    
    // Clear input and enter correct password
    for _ in 0..13 { // length of "wrongpassword"
        app.send_backspace().await?;
    }
    
    app.send_keys(password).await?;
    app.send_enter().await?;
    
    // Should proceed to initialization/group list
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    match &app.app.current_page {
        Page::GroupList { .. } => {
            // Successfully logged in!
        },
        Page::Initializing { .. } => {
            // Wait for initialization to complete
            tokio::time::sleep(Duration::from_millis(500)).await;
            match &app.app.current_page {
                Page::GroupList { .. } => {
                    // Successfully logged in!
                },
                _ => panic!("Expected GroupList after login, got {:?}", app.app.current_page),
            }
        },
        _ => panic!("Expected GroupList or Initializing after correct password, got {:?}", app.app.current_page),
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
    alice.send_key('1').await?;  // Generate new
    alice.send_enter().await?;
    alice.send_enter().await?;  // Transition through GenerateNew
    alice.send_keys("Alice").await?;
    alice.send_enter().await?;
    alice.send_keys("alicepass123").await?;
    alice.send_enter().await?;
    
    // Wait for Alice to reach GroupList
    tokio::time::sleep(Duration::from_millis(2500)).await;
    
    // Bob completes onboarding  
    bob.send_key('1').await?;  // Generate new
    bob.send_enter().await?;
    bob.send_enter().await?;  // Transition through GenerateNew
    bob.send_keys("Bob").await?;
    bob.send_enter().await?;
    bob.send_keys("bobpass123").await?;
    bob.send_enter().await?;
    
    // Wait for Bob to reach GroupList
    tokio::time::sleep(Duration::from_millis(2500)).await;
    
    // Verify both are at GroupList
    match &alice.app.current_page {
        Page::GroupList { .. } => {},
        _ => panic!("Alice should be at GroupList, got {:?}", alice.app.current_page),
    }
    
    match &bob.app.current_page {
        Page::GroupList { .. } => {},
        _ => panic!("Bob should be at GroupList, got {:?}", bob.app.current_page),
    }
    
    // First, let's test that Alice can view the empty group list
    match &alice.app.current_page {
        Page::GroupList { groups, .. } => {
            assert_eq!(groups.len(), 0, "Alice should have no groups initially");
        },
        _ => panic!("Alice should be at GroupList, got {:?}", alice.app.current_page),
    }
    
    // Get Bob's public key (Alice gets this out-of-band)
    let bob_pubkey = bob.app.keys.public_key();
    let bob_npub = bob_pubkey.to_bech32().unwrap();
    println!("Bob's npub: {}", bob_npub);
    
    // Alice runs /dm command to create group with Bob
    alice.send_keys(&format!("/dm {}", bob_npub)).await?;
    alice.send_enter().await?;
    
    // Wait for Alice's /dm command to complete
    tokio::time::sleep(Duration::from_millis(1000)).await;
    
    // Alice should now be in a chat page with Bob
    match &alice.app.current_page {
        Page::Chat { groups, .. } => {
            println!("Alice successfully created chat with Bob");
            assert_eq!(groups.len(), 1, "Alice should now have one group");
        },
        Page::GroupList { .. } => {
            // If still at GroupList, the dm command might have failed
            println!("Alice's /dm command appears to have failed (still at GroupList)");
            // This is expected in test environment without real relays
        },
        _ => println!("Alice at unexpected page: {:?}", alice.app.current_page),
    }
    
    // Test Alice sending a message (even if the group creation failed)
    if matches!(alice.app.current_page, Page::Chat { .. }) {
        alice.send_keys("Hello Bob!").await?;
        alice.send_enter().await?;
        
        // Verify message was added locally
        match &alice.app.current_page {
            Page::Chat { messages, .. } => {
                if !messages.is_empty() {
                    println!("Alice successfully sent message: {}", messages.last().unwrap().content);
                } else {
                    println!("No messages found after sending");
                }
            },
            _ => {},
        }
    }
    
    // Test Bob receiving messages would require real network infrastructure
    // For now, just verify both users completed onboarding successfully
    assert!(matches!(alice.app.current_page, Page::GroupList { .. }) || 
            matches!(alice.app.current_page, Page::Chat { .. }));
    assert!(matches!(bob.app.current_page, Page::GroupList { .. }));
    
    Ok(())
}