use anyhow::Result;
use nostr_sdk::prelude::*;
use nrc::actions::{Action, OnboardingChoice};
use nrc::evented_nrc::EventedNrc;
use nrc::AppState;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;

/// Test client wrapper for EventedNrc - follows the same pattern as the UI
pub struct EventedTestClient {
    pub evented: EventedNrc,
    pub temp_dir: PathBuf,
    pub name: String,
}

impl EventedTestClient {
    pub async fn new(name: &str) -> Result<Self> {
        // Create unique temp directory
        let temp_dir =
            std::env::temp_dir().join(format!("evented_test_{}_{}", name, rand::random::<u32>()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir)?;

        // Create EventedNrc with background processing
        let evented = EventedNrc::new(&temp_dir).await?;

        Ok(Self {
            evented,
            temp_dir,
            name: name.to_string(),
        })
    }

    /// Wait for events to be processed (background thread handles it)
    pub async fn wait_for_processing(&mut self) -> Result<()> {
        println!("🔄 {} waiting for background processing...", self.name);
        // Give the background thread time to process events
        sleep(Duration::from_millis(100)).await;
        println!("✅ {} processing wait complete", self.name);
        Ok(())
    }

    /// Complete onboarding flow by emitting actions like the UI would
    pub async fn complete_onboarding(&mut self) -> Result<()> {
        println!("🔄 Waiting for initial onboarding state...");
        self.wait_for_onboarding_state().await?;

        println!("✅ Initial state reached, choosing GenerateNew...");
        // Choose to generate new keys (option 1)
        self.evented
            .emit(Action::OnboardingChoice(OnboardingChoice::GenerateNew));
        self.wait_for_processing().await?;

        println!("🔄 Waiting for EnterDisplayName mode...");
        self.wait_for_display_name_mode().await?;

        println!("✅ EnterDisplayName mode reached, setting display name...");
        // Enter display name - this should advance to CreatePassword mode
        self.evented.emit(Action::SetDisplayName(self.name.clone()));
        self.wait_for_processing().await?;

        println!("🔄 Waiting for CreatePassword mode...");
        self.wait_for_create_password_mode().await?;

        println!("✅ CreatePassword mode reached, setting password...");
        // Enter password
        let password = format!("password_{}", self.name);
        self.evented.emit(Action::SetPassword(password));
        self.wait_for_processing().await?;

        println!("🔄 Waiting for Ready state...");
        // Wait for initialization to complete
        self.wait_for_ready_state().await?;
        println!("✅ Ready state reached!");

        Ok(())
    }

    async fn wait_for_onboarding_state(&self) -> Result<()> {
        for _ in 0..50 {
            if matches!(
                self.evented.ui_state.borrow().app_state,
                AppState::Onboarding { .. }
            ) {
                return Ok(());
            }
            sleep(Duration::from_millis(100)).await;
        }
        anyhow::bail!("Never reached onboarding state")
    }

    async fn wait_for_display_name_mode(&mut self) -> Result<()> {
        for _ in 0..50 {
            if let AppState::Onboarding { mode, .. } = &self.evented.ui_state.borrow().app_state {
                if matches!(mode, nrc::OnboardingMode::EnterDisplayName) {
                    return Ok(());
                }
            }
            self.wait_for_processing().await?;
            sleep(Duration::from_millis(100)).await;
        }
        anyhow::bail!("Never reached EnterDisplayName mode")
    }

    async fn wait_for_create_password_mode(&mut self) -> Result<()> {
        for _ in 0..50 {
            if let AppState::Onboarding { mode, .. } = &self.evented.ui_state.borrow().app_state {
                if matches!(mode, nrc::OnboardingMode::CreatePassword) {
                    return Ok(());
                }
            }
            self.wait_for_processing().await?;
            sleep(Duration::from_millis(100)).await;
        }
        anyhow::bail!("Never reached CreatePassword mode")
    }

    async fn wait_for_ready_state(&mut self) -> Result<()> {
        for _ in 0..100 {
            // Wait up to 10 seconds
            if matches!(
                self.evented.ui_state.borrow().app_state,
                AppState::Ready { .. }
            ) {
                return Ok(());
            }
            self.wait_for_processing().await?;
            sleep(Duration::from_millis(100)).await;
        }
        anyhow::bail!("Never reached ready state")
    }

    pub fn get_npub(&self) -> String {
        self.evented.npub.clone()
    }

    /// Send a /join command like the UI would
    pub async fn join_chat_with(&mut self, other_npub: &str) -> Result<()> {
        println!("🔗 {} joining chat with {}", self.name, other_npub);
        println!(
            "📊 {} current group count BEFORE join: {}",
            self.name,
            self.group_count()
        );

        self.evented.emit(Action::JoinGroup(other_npub.to_string()));
        self.wait_for_processing().await?;

        println!(
            "📊 {} group count AFTER join: {}",
            self.name,
            self.group_count()
        );
        println!(
            "🔍 {} current state: {:?}",
            self.name,
            self.evented.ui_state.borrow().app_state
        );

        Ok(())
    }

    /// Send a message like the UI would
    pub async fn send_message(&mut self, content: &str) -> Result<()> {
        self.evented.emit(Action::SendMessage(content.to_string()));
        self.wait_for_processing().await?;
        Ok(())
    }

    /// Get current group count
    pub fn group_count(&self) -> usize {
        if let AppState::Ready { groups, .. } = &self.evented.ui_state.borrow().app_state {
            groups.len()
        } else {
            0
        }
    }

    /// Get messages for the first group (if any)
    pub fn get_first_group_messages(&self) -> Vec<nrc::Message> {
        if let Some(group_id) = self.get_first_group_id() {
            self.evented
                .ui_state
                .borrow()
                .messages
                .get(&group_id)
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    fn get_first_group_id(&self) -> Option<openmls::group::GroupId> {
        if let AppState::Ready { groups, .. } = &self.evented.ui_state.borrow().app_state {
            groups.first().cloned()
        } else {
            None
        }
    }

    /// Trigger manual message fetching (simulates periodic fetch)
    pub async fn trigger_fetch_messages(&mut self) -> Result<()> {
        self.evented.emit(Action::FetchMessages);
        self.wait_for_processing().await?;
        Ok(())
    }

    /// Trigger manual welcome fetching
    pub async fn trigger_fetch_welcomes(&mut self) -> Result<()> {
        println!("🔍 {} triggering FetchWelcomes", self.name);
        println!(
            "📊 {} current group count BEFORE: {}",
            self.name,
            self.group_count()
        );

        self.evented.emit(Action::FetchWelcomes);
        self.wait_for_processing().await?;

        println!(
            "📊 {} group count AFTER FetchWelcomes: {}",
            self.name,
            self.group_count()
        );
        println!(
            "🔍 {} current state: {:?}",
            self.name,
            self.evented.ui_state.borrow().app_state
        );

        Ok(())
    }

    /// Select first group (navigate to it)
    pub async fn select_first_group(&mut self) -> Result<()> {
        // Emit navigation to first group
        for _ in 0..self.group_count() {
            self.evented.emit(Action::PrevGroup);
        }
        self.wait_for_processing().await?;
        Ok(())
    }

    /// Get current state for debugging
    pub fn debug_state(&self) -> String {
        format!("{:?}", self.evented.ui_state.borrow().app_state)
    }

    /// Check if there's an error
    pub fn get_last_error(&self) -> Option<String> {
        self.evented.ui_state.borrow().last_error.clone()
    }
}

impl Drop for EventedTestClient {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

#[tokio::test]
async fn test_evented_two_account_dm_flow() -> Result<()> {
    // Enable logging for debugging with higher verbosity
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug"))
        .try_init();

    println!("🧪 Creating Alice and Bob test clients...");

    // Create two test clients
    let mut alice = EventedTestClient::new("Alice").await?;
    let mut bob = EventedTestClient::new("Bob").await?;

    println!("✅ Clients created");
    println!("🔄 Starting Alice's onboarding...");

    // Complete onboarding for both clients
    alice.complete_onboarding().await?;
    println!("✅ Alice completed onboarding");
    println!("📋 Alice NPub: {}", alice.get_npub());

    println!("🔄 Starting Bob's onboarding...");
    bob.complete_onboarding().await?;
    println!("✅ Bob completed onboarding");
    println!("📋 Bob NPub: {}", bob.get_npub());

    // Give key packages time to propagate
    println!("⏳ Waiting for key packages to propagate...");
    sleep(Duration::from_secs(3)).await;

    // Bob joins Alice's chat
    let alice_npub = alice.get_npub();
    println!("💬 Bob joining chat with Alice ({alice_npub})...");

    bob.join_chat_with(&alice_npub).await?;

    // Check that Bob created a group
    assert_eq!(bob.group_count(), 1, "Bob should have created 1 group");
    println!("✅ Bob created group (count: {})", bob.group_count());

    // Wait for Alice to receive the welcome message
    println!("⏳ Waiting for Alice to receive welcome...");
    sleep(Duration::from_secs(4)).await;

    // Trigger welcome processing for Alice
    alice.trigger_fetch_welcomes().await?;
    alice.wait_for_processing().await?;

    // Alice should now have joined the group
    let alice_group_count = alice.group_count();
    println!("📊 Alice group count: {alice_group_count}");

    if alice_group_count == 0 {
        println!("⚠️  Alice hasn't joined yet, checking for errors...");
        if let Some(error) = alice.get_last_error() {
            println!("❌ Alice error: {error}");
        }
        println!("🔍 Alice state: {}", alice.debug_state());
    }

    assert_eq!(alice_group_count, 1, "Alice should have joined the group");

    // Bob selects his group and sends a message
    println!("💬 Bob sending message to Alice...");
    bob.select_first_group().await?;
    bob.send_message("Hello Alice! This is Bob.").await?;

    // Wait for message propagation
    println!("⏳ Waiting for message to propagate...");
    sleep(Duration::from_secs(3)).await;

    // Alice fetches messages
    alice.trigger_fetch_messages().await?;
    alice.wait_for_processing().await?;

    // Check Alice received Bob's message
    let alice_messages = alice.get_first_group_messages();
    println!("📬 Alice received {} messages", alice_messages.len());

    if alice_messages.is_empty() {
        if let Some(error) = alice.get_last_error() {
            println!("❌ Alice error: {error}");
        }
        println!("🔍 Alice state: {}", alice.debug_state());
    }

    assert!(
        !alice_messages.is_empty(),
        "Alice should have received Bob's message"
    );
    assert_eq!(alice_messages[0].content, "Hello Alice! This is Bob.");

    // Alice selects her group and sends a reply
    println!("💬 Alice sending reply to Bob...");
    alice.select_first_group().await?;
    alice.send_message("Hi Bob! Nice to hear from you.").await?;

    // Wait for message propagation
    sleep(Duration::from_secs(3)).await;

    // Bob fetches messages
    bob.trigger_fetch_messages().await?;
    bob.wait_for_processing().await?;

    // Check Bob received Alice's reply
    let bob_messages = bob.get_first_group_messages();
    println!("📬 Bob received {} messages", bob_messages.len());

    // Bob should have at least Alice's message (may also have his own)
    let alice_reply = bob_messages
        .iter()
        .find(|m| m.content.contains("Nice to hear from you"));
    assert!(
        alice_reply.is_some(),
        "Bob should have received Alice's reply"
    );

    println!("🎉 Test completed successfully!");
    println!("✅ Full onboarding -> DM conversation flow working with EventedNrc");

    Ok(())
}
