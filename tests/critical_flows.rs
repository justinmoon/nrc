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