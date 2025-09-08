use anyhow::Result;
use nrc::actions::{Action, OnboardingChoice};
use nrc::evented_nrc::{EventedNrc, EventLoop};
use nrc::AppState;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_simple_onboarding_fix() -> Result<()> {
    // Enable logging
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).try_init();

    println!("🧪 Creating single test client...");
    
    // Create temp directory
    let temp_dir = std::env::temp_dir().join("simple_onboarding_test");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir)?;

    // Create EventedNrc
    let (evented, mut event_loop) = EventedNrc::new(&temp_dir).await?;
    
    println!("✅ EventedNrc created");
    println!("🔍 Initial state: {:?}", *evented.state.borrow());

    // Wait for onboarding state
    for i in 0..50 {
        if matches!(*evented.state.borrow(), AppState::Onboarding { .. }) {
            println!("✅ Reached onboarding state after {} iterations", i);
            break;
        }
        event_loop.process_one().await;
        sleep(Duration::from_millis(100)).await;
    }

    let current_state = evented.state.borrow().clone();
    println!("🔍 Current state: {:?}", current_state);

    // Test the specific sequence that was failing
    println!("🚀 Emitting OnboardingChoice(GenerateNew)");
    evented.emit(Action::OnboardingChoice(OnboardingChoice::GenerateNew));
    
    // Process all actions until none left
    println!("🔄 Processing actions...");
    while event_loop.process_one().await.is_some() {
        // Keep processing until no more actions
    }

    // Check state transition
    let after_choice = evented.state.borrow().clone();
    println!("🔍 After GenerateNew: {:?}", after_choice);

    // Should be in EnterDisplayName mode
    if let AppState::Onboarding { mode, .. } = &after_choice {
        if matches!(mode, nrc::OnboardingMode::EnterDisplayName) {
            println!("✅ Correctly transitioned to EnterDisplayName mode");
        } else {
            anyhow::bail!("Wrong mode after GenerateNew: {:?}", mode);
        }
    } else {
        anyhow::bail!("Not in onboarding state after GenerateNew: {:?}", after_choice);
    }

    // Test the SetDisplayName action
    println!("🚀 Emitting SetDisplayName(TestUser)");
    evented.emit(Action::SetDisplayName("TestUser".to_string()));

    // Process all actions until none left
    println!("🔄 Processing SetDisplayName action...");
    while event_loop.process_one().await.is_some() {
        // Keep processing until no more actions
    }

    // Check state transition
    let after_name = evented.state.borrow().clone();
    println!("🔍 After SetDisplayName: {:?}", after_name);

    // Should be in CreatePassword mode
    if let AppState::Onboarding { mode, .. } = &after_name {
        if matches!(mode, nrc::OnboardingMode::CreatePassword) {
            println!("✅ Correctly transitioned to CreatePassword mode");
            println!("🎉 Onboarding state transitions are working!");
            
            // Clean up
            let _ = std::fs::remove_dir_all(&temp_dir);
            
            return Ok(());
        } else {
            anyhow::bail!("Wrong mode after SetDisplayName: {:?}", mode);
        }
    } else {
        anyhow::bail!("Not in onboarding state after SetDisplayName: {:?}", after_name);
    }

    // Clean up
    let _ = std::fs::remove_dir_all(&temp_dir);
    
    anyhow::bail!("Onboarding state transitions not working correctly")
}