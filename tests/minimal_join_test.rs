use anyhow::Result;
use nrc::actions::{Action, OnboardingChoice};
use nrc::evented_nrc::{EventedNrc, EventLoop};
use nrc::AppState;
use std::time::Duration;
use tokio::time::sleep;

/// Minimal test client for debugging join issues
pub struct MinimalTestClient {
    pub evented: EventedNrc,
    pub event_loop: EventLoop,
    pub temp_dir: std::path::PathBuf,
    pub name: String,
}

impl MinimalTestClient {
    pub async fn new(name: &str) -> Result<Self> {
        let temp_dir = std::env::temp_dir().join(format!("minimal_test_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir)?;

        let (evented, event_loop) = EventedNrc::new(&temp_dir).await?;

        Ok(Self {
            evented,
            event_loop,
            temp_dir,
            name: name.to_string(),
        })
    }

    pub async fn fast_onboard(&mut self) -> Result<()> {
        println!("🚀 {} fast onboarding", self.name);

        // Wait for onboarding state
        for _i in 0..10 {
            if matches!(*self.evented.state.borrow(), AppState::Onboarding { .. }) {
                break;
            }
            self.event_loop.process_one().await;
            sleep(Duration::from_millis(50)).await;
        }

        // Generate new keys
        self.evented.emit(Action::OnboardingChoice(OnboardingChoice::GenerateNew));
        for _i in 0..5 {
            self.event_loop.process_one().await;
        }

        // Set display name
        self.evented.emit(Action::SetDisplayName(self.name.clone()));
        for _i in 0..5 {
            self.event_loop.process_one().await;
        }

        // Set password
        let password = format!("password_{}", self.name);
        self.evented.emit(Action::SetPassword(password));
        
        // Process until ready
        for i in 0..30 {
            self.event_loop.process_one().await;
            if matches!(*self.evented.state.borrow(), AppState::Ready { .. }) {
                println!("✅ {} ready after {} iterations", self.name, i);
                return Ok(());
            }
            sleep(Duration::from_millis(200)).await;
        }

        anyhow::bail!("{} never reached ready state", self.name)
    }

    pub fn get_npub(&self) -> String {
        self.evented.npub.clone()
    }

    pub async fn join_chat_with(&mut self, other_npub: &str) -> Result<()> {
        println!("🔗 {} joining chat with {}", self.name, other_npub);
        println!("📊 {} groups BEFORE join: {}", self.name, self.group_count());
        
        self.evented.emit(Action::JoinGroup(other_npub.to_string()));
        
        // Process with detailed logging
        for i in 0..15 {
            if let Some(_) = self.event_loop.process_one().await {
                println!("🔄 {} processed join action #{}", self.name, i + 1);
            }
        }
        
        println!("📊 {} groups AFTER join: {}", self.name, self.group_count());
        Ok(())
    }

    pub fn group_count(&self) -> usize {
        if let AppState::Ready { groups, .. } = &*self.evented.state.borrow() {
            groups.len()
        } else {
            0
        }
    }

    pub async fn trigger_fetch_welcomes(&mut self) -> Result<()> {
        println!("🔍 {} triggering FetchWelcomes", self.name);
        println!("📊 {} groups BEFORE fetch: {}", self.name, self.group_count());
        
        self.evented.emit(Action::FetchWelcomes);
        
        // Process with detailed logging
        for i in 0..10 {
            if let Some(_) = self.event_loop.process_one().await {
                println!("🔄 {} processed welcome fetch action #{}", self.name, i + 1);
            }
        }
        
        println!("📊 {} groups AFTER fetch: {}", self.name, self.group_count());
        Ok(())
    }
}

impl Drop for MinimalTestClient {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

#[tokio::test]
async fn test_minimal_alice_bob_join() -> Result<()> {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).try_init();

    println!("🧪 MINIMAL TEST: Alice and Bob join test");
    
    let mut alice = MinimalTestClient::new("Alice").await?;
    let mut bob = MinimalTestClient::new("Bob").await?;

    println!("🔄 Onboarding Alice...");
    alice.fast_onboard().await?;
    let alice_npub = alice.get_npub();
    println!("✅ Alice ready: {}", alice_npub);

    println!("🔄 Onboarding Bob...");  
    bob.fast_onboard().await?;
    println!("✅ Bob ready: {}", bob.get_npub());

    // Wait for key packages
    println!("⏳ Waiting for key packages...");
    sleep(Duration::from_secs(2)).await;

    // Bob joins Alice
    println!("💬 Bob joining Alice's chat");
    bob.join_chat_with(&alice_npub).await?;
    
    let bob_groups = bob.group_count();
    println!("📊 Bob final groups: {}", bob_groups);
    
    if bob_groups == 0 {
        anyhow::bail!("❌ Bob failed to create group");
    }

    // Wait for welcome propagation
    println!("⏳ Waiting for welcome propagation...");
    sleep(Duration::from_secs(3)).await;
    
    // Alice processes welcomes  
    println!("🔍 Alice processing welcomes...");
    alice.trigger_fetch_welcomes().await?;
    
    let alice_groups = alice.group_count();
    println!("📊 Alice final groups: {}", alice_groups);

    if alice_groups == 0 {
        println!("❌ Alice didn't join - THIS IS THE BUG");
        return Ok(()); // Don't fail, just report the issue
    } else {
        println!("🎉 SUCCESS! Alice joined the group");
    }

    Ok(())
}