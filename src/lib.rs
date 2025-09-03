use anyhow::Result;
use nostr_mls::{groups::NostrGroupConfigData, messages::MessageProcessingResult, NostrMls};
use nostr_mls_memory_storage::NostrMlsMemoryStorage;
use nostr_mls_sqlite_storage::NostrMlsSqliteStorage;
use nostr_mls_storage::groups::types as group_types;
use nostr_sdk::prelude::*;
use openmls::group::GroupId;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Message {
    pub content: String,
    pub sender: PublicKey,
    pub timestamp: Timestamp,
}

#[derive(Debug, Clone)]
pub enum AppState {
    Onboarding {
        input: String,
        mode: OnboardingMode,
    },
    Initializing,
    Ready {
        key_package_published: bool,
        groups: Vec<GroupId>,
    },
}

#[derive(Debug, Clone)]
pub enum OnboardingMode {
    Choose,
    GenerateNew,
    ImportExisting,
}

enum Storage {
    Memory(NostrMls<NostrMlsMemoryStorage>),
    Sqlite(NostrMls<NostrMlsSqliteStorage>),
}

macro_rules! with_storage {
    ($self:expr, $method:ident($($args:expr),*)) => {
        match &$self.storage {
            Storage::Memory(mls) => mls.$method($($args),*),
            Storage::Sqlite(mls) => mls.$method($($args),*),
        }
    };
}

macro_rules! with_storage_mut {
    ($self:expr, $method:ident($($args:expr),*)) => {
        match &mut $self.storage {
            Storage::Memory(mls) => mls.$method($($args),*),
            Storage::Sqlite(mls) => mls.$method($($args),*),
        }
    };
}

pub struct Nrc {
    storage: Storage,
    keys: Keys,
    client: Client,
    pub state: AppState,
    messages: HashMap<GroupId, Vec<Message>>,
    welcome_rumors: HashMap<PublicKey, UnsignedEvent>,
    groups: HashMap<GroupId, group_types::Group>,
    pub input: String,
    pub selected_group_index: Option<usize>,
    pub scroll_offset: u16,
    pub last_error: Option<String>,
    pub flash_message: Option<String>,
    pub show_help: bool,
    profiles: HashMap<PublicKey, Metadata>,
}

impl Nrc {
    pub async fn new(datadir: &Path, use_memory: bool) -> Result<Self> {
        let keys = Keys::generate();
        let client = Client::builder().signer(keys.clone()).build();

        // Add multiple relays for redundancy
        let relays = [
            "wss://relay.damus.io",
            "wss://nos.lol",
            "wss://relay.nostr.band",
            "wss://relay.snort.social",
            "wss://nostr.wine",
        ];

        for relay in relays {
            if let Err(e) = client.add_relay(relay).await {
                log::warn!("Failed to add relay {relay}: {e}");
            }
        }

        client.connect().await;

        // Wait for connections to establish
        tokio::time::sleep(Duration::from_secs(2)).await;

        let storage = if use_memory {
            log::info!("Using in-memory storage");
            Storage::Memory(NostrMls::new(NostrMlsMemoryStorage::default()))
        } else {
            // Create datadir if it doesn't exist
            std::fs::create_dir_all(datadir)?;
            let db_path = datadir.join("nrc.db");
            log::info!("Using SQLite storage at: {:?}", db_path);
            Storage::Sqlite(NostrMls::new(NostrMlsSqliteStorage::new(db_path)?))
        };

        Ok(Self {
            storage,
            keys,
            client,
            state: AppState::Onboarding {
                input: String::new(),
                mode: OnboardingMode::Choose,
            },
            messages: HashMap::new(),
            welcome_rumors: HashMap::new(),
            groups: HashMap::new(),
            input: String::new(),
            selected_group_index: None,
            scroll_offset: 0,
            last_error: None,
            flash_message: None,
            show_help: false,
            profiles: HashMap::new(),
        })
    }

    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    pub async fn publish_key_package(&mut self) -> Result<()> {
        // First subscribe to key packages so we can verify our own
        let filter = Filter::new()
            .kind(Kind::from(443u16))
            .author(self.keys.public_key());
        self.client.subscribe(filter, None).await?;

        let relays = vec![
            RelayUrl::parse("wss://relay.damus.io")?,
            RelayUrl::parse("wss://nos.lol")?,
            RelayUrl::parse("wss://relay.nostr.band")?,
            RelayUrl::parse("wss://relay.snort.social")?,
            RelayUrl::parse("wss://nostr.wine")?,
        ];
        let (key_package_content, tags) = 
            with_storage_mut!(self, create_key_package_for_event(&self.keys.public_key(), relays))?;

        let event = EventBuilder::new(Kind::from(443u16), key_package_content)
            .tags(tags)
            .build(self.keys.public_key())
            .sign(&self.keys)
            .await?;

        self.client.send_event(&event).await?;

        // Wait a bit to ensure it's published
        tokio::time::sleep(Duration::from_secs(1)).await;

        let filter = Filter::new()
            .kind(Kind::GiftWrap)
            .pubkey(self.keys.public_key());
        self.client.subscribe(filter, None).await?;

        if let AppState::Ready { groups, .. } = &self.state {
            self.state = AppState::Ready {
                key_package_published: true,
                groups: groups.clone(),
            };
        } else {
            self.state = AppState::Ready {
                key_package_published: true,
                groups: Vec::new(),
            };
        }

        Ok(())
    }

    pub async fn fetch_key_package(&self, pubkey: &PublicKey) -> Result<Event> {
        let filter = Filter::new()
            .kind(Kind::from(443u16))
            .author(*pubkey)
            .limit(1);

        // Subscribe to ensure we can fetch events
        self.client.subscribe(filter.clone(), None).await?;

        // Give time for event to propagate to relay and retry multiple times
        for attempt in 1..=10 {
            tokio::time::sleep(Duration::from_millis(1500)).await;

            // Try to fetch from relay
            if let Ok(events) = self
                .client
                .fetch_events(filter.clone(), Duration::from_secs(5))
                .await
            {
                if !events.is_empty() {
                    log::debug!("Found key package on attempt {attempt}");
                    return Ok(events.into_iter().next().unwrap());
                }
            }

            if attempt % 3 == 0 {
                log::debug!("Attempt {attempt} - key package not found yet for {pubkey}");
            }
        }

        // Last resort: check local database
        let events = self.client.database().query(filter).await?;

        events
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No key package found for {} after 10 attempts", pubkey))
    }

    pub async fn create_group(&mut self, name: String) -> Result<GroupId> {
        let config = NostrGroupConfigData::new(
            name,
            "NRC Chat Group".to_string(),
            None,
            None,
            None,
            vec![RelayUrl::parse("wss://relay.damus.io")?],
            vec![self.keys.public_key()],
        );
        
        let group_result = with_storage_mut!(self, create_group(
            &self.keys.public_key(),
            vec![],
            config
        ))?;
        let group_id = GroupId::from_slice(group_result.group.mls_group_id.as_slice());
        
        self.groups.insert(group_id.clone(), group_result.group);
        
        if let AppState::Ready {
            key_package_published,
            mut groups,
        } = self.state.clone()
        {
            groups.push(group_id.clone());
            self.state = AppState::Ready {
                key_package_published,
                groups,
            };
        }
        
        Ok(group_id)
    }
    
    pub async fn create_group_with_member(&mut self, key_package: Event) -> Result<GroupId> {
        let config = NostrGroupConfigData::new(
            "Test Group".to_string(),
            "Test group for NRC".to_string(),
            None,
            None,
            None,
            vec![],
            vec![self.keys.public_key()],
        );

        let group_result = with_storage_mut!(self, create_group(
            &self.keys.public_key(),
            vec![key_package.clone()],
            config
        ))?;

        let group_id = GroupId::from_slice(group_result.group.mls_group_id.as_slice());
        // Note: merge_pending_commit is already called inside create_group

        self.groups
            .insert(group_id.clone(), group_result.group.clone());

        if let Some(welcome_rumor) = group_result.welcome_rumors.first() {
            let recipient_pubkey = key_package.pubkey;
            self.welcome_rumors
                .insert(recipient_pubkey, welcome_rumor.clone());
        }

        if let AppState::Ready {
            key_package_published,
            mut groups,
        } = self.state.clone()
        {
            groups.push(group_id.clone());
            self.state = AppState::Ready {
                key_package_published,
                groups,
            };
        }

        Ok(group_id)
    }

    pub fn get_welcome_rumor_for(&self, pubkey: &PublicKey) -> Result<UnsignedEvent> {
        self.welcome_rumors
            .get(pubkey)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("No welcome rumor found for {}", pubkey))
    }

    pub async fn send_gift_wrapped_welcome(
        &self,
        recipient: &PublicKey,
        welcome_rumor: UnsignedEvent,
    ) -> Result<()> {
        let gift_wrapped =
            EventBuilder::gift_wrap(&self.keys, recipient, welcome_rumor, None).await?;

        self.client.send_event(&gift_wrapped).await?;
        Ok(())
    }

    pub async fn fetch_and_process_welcomes(&mut self) -> Result<()> {
        let filter = Filter::new()
            .kind(Kind::GiftWrap)
            .pubkey(self.keys.public_key())
            .limit(10);

        tokio::time::sleep(Duration::from_secs(2)).await;

        let events = self
            .client
            .fetch_events(filter, Duration::from_secs(10))
            .await?;

        for gift_wrap in events {
            if let Ok(unwrapped) = self.client.unwrap_gift_wrap(&gift_wrap).await {
                if unwrapped.rumor.kind == Kind::from(444u16) {
                    // Process the welcome to add it to pending welcomes
                    let welcome = with_storage_mut!(self, process_welcome(&gift_wrap.id, &unwrapped.rumor))?;

                    // Accept the welcome to actually join the group
                    with_storage_mut!(self, accept_welcome(&welcome))?;

                    // Get the group info from storage after accepting
                    let group_id = GroupId::from_slice(welcome.mls_group_id.as_slice());
                    if let Ok(Some(group)) = with_storage!(self, get_group(&group_id)) {
                        self.groups.insert(group_id.clone(), group.clone());
                        
                        // Fetch the profile of the person who invited us (first admin)
                        if let Some(admin) = group.admin_pubkeys.first() {
                            let _ = self.fetch_profile(admin).await;
                        }
                    }

                    if let AppState::Ready {
                        key_package_published,
                        mut groups,
                    } = self.state.clone()
                    {
                        groups.push(group_id.clone());
                        self.state = AppState::Ready {
                            key_package_published,
                            groups,
                        };
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn send_message(&mut self, group_id: GroupId, content: String) -> Result<()> {
        let group_id = &group_id;
        let content = &content;
        let text_note_rumor = EventBuilder::text_note(content).build(self.keys.public_key());

        let event = with_storage_mut!(self, create_message(group_id, text_note_rumor))?;

        // Note: merge_pending_commit is already called inside create_message

        log::debug!(
            "Sending message event: id={}, kind={}",
            event.id, event.kind
        );
        self.client.send_event(&event).await?;

        // Don't store locally - we'll fetch it from the relay like other messages
        // This avoids duplicates

        Ok(())
    }

    pub async fn fetch_and_process_messages(&mut self) -> Result<()> {
        let groups = match &self.state {
            AppState::Ready { groups, .. } => groups.clone(),
            _ => return Ok(()),
        };

        log::debug!("Fetching messages for {} groups", groups.len());
        
        for group_id in groups {
            // Get the actual nostr_group_id from storage
            let group = match self.groups.get(&group_id) {
                Some(g) => g,
                None => {
                    log::warn!("Group not found in storage: {}", hex::encode(group_id.as_slice()));
                    continue;
                }
            };
            let h_tag_value = hex::encode(group.nostr_group_id);
            log::debug!(
                "Fetching messages for MLS group: {}, Nostr group: {}",
                hex::encode(group_id.as_slice()),
                h_tag_value
            );
            let filter = Filter::new()
                .kind(Kind::from(445u16))
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value.clone())
                .limit(100);
            log::debug!("Filter: kind=445, h-tag={h_tag_value}");

            tokio::time::sleep(Duration::from_secs(1)).await;

            let events = self
                .client
                .fetch_events(filter, Duration::from_secs(10))
                .await?;
            log::debug!("Fetched {} events from relay", events.len());

            for event in events {
                log::debug!("Processing event: {}", event.id);
                if let Ok(MessageProcessingResult::ApplicationMessage(msg)) =
                    with_storage_mut!(self, process_message(&event))
                {
                    log::debug!("Got ApplicationMessage, kind: {}", msg.kind);
                    if msg.kind == Kind::TextNote {
                        if let Ok(Some(stored_msg)) = with_storage!(self, get_message(&msg.id)) {
                            // Check if we already have this message (by ID) to avoid duplicates
                            let messages = self.messages.entry(group_id.clone()).or_default();
                            let already_exists = messages.iter().any(|m| {
                                m.content == stored_msg.content
                                    && m.sender == stored_msg.pubkey
                                    && m.timestamp == stored_msg.created_at
                            });

                            if !already_exists {
                                let message = Message {
                                    content: stored_msg.content.clone(),
                                    sender: stored_msg.pubkey,
                                    timestamp: stored_msg.created_at,
                                };
                                log::info!("Adding message to group: '{}' from {}", 
                                    message.content, 
                                    message.sender.to_bech32().unwrap_or_else(|_| "unknown".to_string())
                                );
                                messages.push(message.clone());
                                
                                // Fetch profile for this sender if we don't have it
                                if !self.profiles.contains_key(&message.sender) {
                                    let _ = self.fetch_profile(&message.sender).await;
                                }
                            } else {
                                log::debug!("Message already exists, skipping");
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub fn get_messages(&self, group_id: &GroupId) -> Vec<Message> {
        self.messages.get(group_id).cloned().unwrap_or_default()
    }
    
    pub async fn initialize(&mut self) -> Result<()> {
        self.state = AppState::Initializing;
        self.publish_key_package().await?;
        
        let groups = with_storage!(self, get_groups())?;
        let group_ids: Vec<GroupId> = groups.iter().map(|g| g.mls_group_id.clone()).collect();
        
        // Store groups in our HashMap for later use
        for group in groups {
            self.groups.insert(group.mls_group_id.clone(), group);
        }
        
        self.state = AppState::Ready {
            key_package_published: true,
            groups: group_ids,
        };
        Ok(())
    }
    
    pub async fn initialize_with_nsec(&mut self, nsec: String) -> Result<()> {
        let keys = Keys::parse(&nsec)?;
        self.keys = keys;
        self.client = Client::builder().signer(self.keys.clone()).build();
        
        let relays = [
            "wss://relay.damus.io",
            "wss://nos.lol",
            "wss://relay.nostr.band",
            "wss://relay.snort.social",
            "wss://nostr.wine",
        ];
        
        for relay in relays {
            if let Err(e) = self.client.add_relay(relay).await {
                log::warn!("Failed to add relay {relay}: {e}");
            }
        }
        
        self.client.connect().await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        self.initialize().await
    }
    
    pub fn next_group(&mut self) {
        if let AppState::Ready { ref groups, .. } = self.state {
            if !groups.is_empty() {
                self.selected_group_index = Some(
                    self.selected_group_index
                        .map(|i| (i + 1) % groups.len())
                        .unwrap_or(0),
                );
            }
        }
    }
    
    pub fn prev_group(&mut self) {
        if let AppState::Ready { ref groups, .. } = self.state {
            if !groups.is_empty() {
                self.selected_group_index = Some(
                    self.selected_group_index
                        .map(|i| if i == 0 { groups.len() - 1 } else { i - 1 })
                        .unwrap_or(0),
                );
            }
        }
    }
    
    
    
    pub fn get_groups(&self) -> Vec<GroupId> {
        self.groups.keys().cloned().collect()
    }
    
    pub fn get_active_group(&self) -> Option<&GroupId> {
        if let AppState::Ready { ref groups, .. } = self.state {
            if let Some(idx) = self.selected_group_index {
                groups.get(idx)
            } else {
                None
            }
        } else {
            None
        }
    }
    
    pub async fn process_input(&mut self, input: String) -> Result<bool> {
        self.last_error = None;
        self.flash_message = None;
        
        // Check if it's a command
        if input.starts_with("/") {
            return self.process_command(input).await;
        }
        
        // Otherwise it's a message - send to selected chat
        if let AppState::Ready { ref groups, .. } = &self.state {
            if let Some(idx) = self.selected_group_index {
                if let Some(group_id) = groups.get(idx) {
                    if let Err(e) = self.send_message(group_id.clone(), input).await {
                        self.last_error = Some(format!("Failed to send: {}", e));
                    }
                }
            } else {
                self.last_error = Some("No chat selected".to_string());
            }
        }
        
        Ok(false)
    }
    
    pub async fn process_command(&mut self, input: String) -> Result<bool> {
        self.last_error = None;
        self.flash_message = None;
        
        // Handle quit
        if input == "/quit" || input == "/q" {
            return Ok(true); // Signal to quit
        }
        
        // Handle npub copy
        if input == "/npub" || input == "/n" {
            use clipboard::ClipboardProvider;
            use clipboard::ClipboardContext;
            use nostr_sdk::prelude::ToBech32;
            
            let npub = self.keys.public_key().to_bech32().unwrap_or_else(|_| "error".to_string());
            
            match ClipboardContext::new() {
                Ok(mut ctx) => {
                    if let Err(e) = ctx.set_contents(npub.clone()) {
                        self.last_error = Some(format!("Failed to copy: {}", e));
                    } else {
                        self.flash_message = Some(format!("Copied npub to clipboard: {}", npub));
                    }
                }
                Err(e) => {
                    self.last_error = Some(format!("Clipboard not available: {}", e));
                }
            }
            return Ok(false);
        }
        
        // Handle help command
        if input == "/help" || input == "/h" {
            self.show_help = true;
            return Ok(false);
        }
        
        // Handle navigation commands in Ready state
        if matches!(self.state, AppState::Ready { .. }) {
            if input == "/next" {
                self.next_group();
                return Ok(false);
            } else if input == "/prev" {
                self.prev_group();
                return Ok(false);
            }
        }
        
        
        if input.starts_with("/join ") || input.starts_with("/j ") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() < 2 {
                self.last_error = Some("Usage: /join <npub>".to_string());
                return Ok(false);
            }
            
            let pubkey_str = parts[1];
            let pubkey = match PublicKey::from_str(pubkey_str) {
                Ok(pk) => pk,
                Err(e) => {
                    self.last_error = Some(format!("Invalid public key: {}", e));
                    return Ok(false);
                }
            };
            
            // Fetch their profile first
            let _ = self.fetch_profile(&pubkey).await;
            
            // IMPORTANT: First check if they already sent us a welcome
            // This prevents creating duplicate groups
            log::info!("Checking for existing welcomes before creating group with {}", pubkey_str);
            let _ = self.fetch_and_process_welcomes().await;
            
            // Check if we're already in a group with this person
            let already_in_group = if let AppState::Ready { ref groups, .. } = self.state {
                groups.iter().any(|group_id| {
                    if let Some(group) = self.groups.get(group_id) {
                        // Check if this person is an admin (creator) of any of our groups
                        group.admin_pubkeys.contains(&pubkey)
                    } else {
                        false
                    }
                })
            } else {
                false
            };
            
            if already_in_group {
                self.flash_message = Some(format!("Already in a group with {}", pubkey_str));
                log::info!("Already in a group with {}, not creating a new one", pubkey_str);
                return Ok(false);
            }
            
            // If not already in a group, fetch their key package and create one
            match self.fetch_key_package(&pubkey).await {
                Ok(key_package) => {
                    // Create a group with them
                    match self.create_group_with_member(key_package).await {
                        Ok(group_id) => {
                            // Send them the welcome
                            if let Ok(welcome_rumor) = self.get_welcome_rumor_for(&pubkey) {
                                let _ = self.send_gift_wrapped_welcome(&pubkey, welcome_rumor).await;
                            }
                            
                            // Update our state to show the new group
                            if let AppState::Ready { key_package_published, groups } = &self.state {
                                let mut updated_groups = groups.clone();
                                if !updated_groups.contains(&group_id) {
                                    updated_groups.push(group_id);
                                }
                                self.state = AppState::Ready {
                                    key_package_published: *key_package_published,
                                    groups: updated_groups,
                                };
                            }
                        }
                        Err(e) => {
                            self.last_error = Some(format!("Failed to create group: {}", e));
                        }
                    }
                }
                Err(e) => {
                    self.last_error = Some(format!("Failed to fetch key package: {}", e));
                }
            }
        } else if input.starts_with("/") {
            // Unknown command  
            self.last_error = Some("Commands: /join <npub>, /npub, /next, /prev, /help, /quit".to_string());
        }
        // If not a command, it's a regular message - don't set an error
        
        Ok(false)
    }
    
    pub fn clear_error(&mut self) {
        self.last_error = None;
        self.flash_message = None;
    }
    
    pub fn dismiss_help(&mut self) {
        self.show_help = false;
    }
    
    pub fn get_chat_display_name(&self, group_id: &GroupId) -> String {
        // Get the other member's display name from their profile
        if let Some(group) = self.groups.get(group_id) {
            // For now, assume it's a 1-on-1 chat
            let our_pubkey = self.keys.public_key();
            
            // First check if we know the admin (creator) of the group
            for admin in &group.admin_pubkeys {
                if admin != &our_pubkey {
                    if let Some(profile) = self.profiles.get(admin) {
                        if let Some(name) = profile.display_name.as_ref() {
                            if !name.is_empty() {
                                return name.clone();
                            }
                        }
                        if let Some(name) = profile.name.as_ref() {
                            if !name.is_empty() {
                                return name.clone();
                            }
                        }
                    }
                }
            }
            
            // Otherwise, try to find a member from messages
            if let Some(messages) = self.messages.get(group_id) {
                for msg in messages {
                    if msg.sender != our_pubkey {
                        // Found the other person, get their profile
                        if let Some(profile) = self.profiles.get(&msg.sender) {
                            if let Some(name) = profile.display_name.as_ref() {
                                if !name.is_empty() {
                                    return name.clone();
                                }
                            }
                            if let Some(name) = profile.name.as_ref() {
                                if !name.is_empty() {
                                    return name.clone();
                                }
                            }
                        }
                        // No profile found, return "Unknown"
                        return "Unknown".to_string();
                    }
                }
            }
        }
        "Unknown".to_string()
    }
    
    pub async fn fetch_profile(&mut self, pubkey: &PublicKey) -> Result<()> {
        let filter = Filter::new()
            .kind(Kind::Metadata)
            .author(*pubkey)
            .limit(1);
        
        let events = self.client
            .fetch_events(filter, Duration::from_secs(5))
            .await?;
        
        if let Some(event) = events.first() {
            if let Ok(metadata) = Metadata::from_json(&event.content) {
                self.profiles.insert(*pubkey, metadata);
            }
        }
        
        Ok(())
    }
    
    pub fn get_group_members(&self, group_id: &GroupId) -> Result<Vec<PublicKey>> {
        // For now, return the current user as the only member
        // In a real implementation, you'd track members properly  
        if let Some(_group) = self.groups.get(group_id) {
            Ok(vec![self.keys.public_key()])
        } else {
            Ok(vec![])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_scenario_two_clients_join_and_chat() -> Result<()> {
        // This test simulates the exact scenario described:
        // 1. Two clients start up
        // 2. Each does /j with the other's npub
        // 3. They exchange messages
        // 4. Messages should appear on both sides
        
        env_logger::init();
        
        let temp_dir = std::env::temp_dir();
        let mut alice = Nrc::new(&temp_dir, true).await?; // Use memory storage for tests
        let mut bob = Nrc::new(&temp_dir, true).await?; // Use memory storage for tests
        
        // Initialize both clients (like pressing "1" to generate keys in onboarding)
        alice.initialize().await?;
        bob.initialize().await?;
        
        // Wait for key packages to propagate
        sleep(Duration::from_secs(3)).await;
        
        // Alice creates a group with Bob (like typing /j npub...)
        let bob_npub = bob.public_key().to_bech32()?;
        alice.process_input(format!("/j {}", bob_npub)).await?;
        
        // Wait for welcome to propagate
        sleep(Duration::from_secs(3)).await;
        
        // Bob fetches welcomes and automatically joins the group
        bob.fetch_and_process_welcomes().await?;
        
        // Alice should have a group
        assert_eq!(alice.get_groups().len(), 1, "Alice should have 1 group");
        let alice_group = alice.get_groups()[0].clone();
        
        // Bob should have a group
        assert_eq!(bob.get_groups().len(), 1, "Bob should have 1 group");
        let bob_group = bob.get_groups()[0].clone();
        
        // Debug: Check if they're in the same Nostr group
        let alice_nostr_group = alice.groups.get(&alice_group)
            .expect("Alice should have group in storage");
        let bob_nostr_group = bob.groups.get(&bob_group)
            .expect("Bob should have group in storage");
        
        println!("Alice's Nostr group ID: {}", hex::encode(&alice_nostr_group.nostr_group_id));
        println!("Bob's Nostr group ID: {}", hex::encode(&bob_nostr_group.nostr_group_id));
        
        // They should now be in the SAME group!
        assert_eq!(
            alice_nostr_group.nostr_group_id, 
            bob_nostr_group.nostr_group_id,
            "Alice and Bob should be in the same Nostr group"
        );
        
        // Alice sends a message (like typing text and hitting enter)
        alice.selected_group_index = Some(0); // Select the first group
        alice.process_input("Hello Bob!".to_string()).await?;
        
        // Wait for message to propagate
        sleep(Duration::from_secs(3)).await;
        
        // Bob fetches messages
        bob.fetch_and_process_messages().await?;
        
        // Bob should see Alice's message
        let bob_messages = bob.get_messages(&bob_group);
        assert_eq!(bob_messages.len(), 1, "Bob should see 1 message");
        assert_eq!(bob_messages[0].content, "Hello Bob!");
        assert_eq!(bob_messages[0].sender, alice.public_key());
        
        // Bob sends a reply
        bob.selected_group_index = Some(0);
        bob.process_input("Hi Alice!".to_string()).await?;
        
        // Wait for message to propagate
        sleep(Duration::from_secs(3)).await;
        
        // Alice fetches messages
        alice.fetch_and_process_messages().await?;
        
        // Alice should see both messages
        let alice_messages = alice.get_messages(&alice_group);
        assert_eq!(alice_messages.len(), 2, "Alice should see 2 messages");
        
        // Sort messages by timestamp to ensure order
        let mut sorted_messages = alice_messages.clone();
        sorted_messages.sort_by_key(|m| m.timestamp);
        
        assert_eq!(sorted_messages[0].content, "Hello Bob!");
        assert_eq!(sorted_messages[0].sender, alice.public_key());
        assert_eq!(sorted_messages[1].content, "Hi Alice!");
        assert_eq!(sorted_messages[1].sender, bob.public_key());
        
        // Bob should also see both messages when fetching again
        bob.fetch_and_process_messages().await?;
        let bob_messages = bob.get_messages(&bob_group);
        assert_eq!(bob_messages.len(), 2, "Bob should see 2 messages");
        
        Ok(())
    }

    #[tokio::test]
    async fn test_two_nrc_instances_exchange_messages() -> Result<()> {
        let temp_dir = std::env::temp_dir();
        let mut alice = Nrc::new(&temp_dir, true).await?; // Use memory storage for tests
        let mut bob = Nrc::new(&temp_dir, true).await?; // Use memory storage for tests

        // Publish key packages with time for propagation
        alice.publish_key_package().await?;
        log::info!("Alice key package published: {}", alice.public_key());
        sleep(Duration::from_secs(1)).await;

        bob.publish_key_package().await?;
        log::info!("Bob key package published: {}", bob.public_key());
        sleep(Duration::from_secs(1)).await;

        let bob_kp = alice.fetch_key_package(&bob.public_key()).await?;

        let group_id = alice.create_group_with_member(bob_kp).await?;

        let welcome_rumor = alice.get_welcome_rumor_for(&bob.public_key())?;
        alice
            .send_gift_wrapped_welcome(&bob.public_key(), welcome_rumor)
            .await?;

        sleep(Duration::from_secs(3)).await;

        bob.fetch_and_process_welcomes().await?;
        log::info!("Bob processed welcomes, his groups: {:?}", bob.state);

        alice.send_message(group_id.clone(), "Hello Bob!".to_string()).await?;
        log::info!(
            "Alice sent message to group: {:?}",
            hex::encode(group_id.as_slice())
        );

        sleep(Duration::from_secs(3)).await;

        bob.fetch_and_process_messages().await?;
        let messages = bob.get_messages(&group_id);
        log::info!("Bob fetched messages, count: {}", messages.len());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Hello Bob!");

        bob.send_message(group_id.clone(), "Hi Alice!".to_string()).await?;

        sleep(Duration::from_secs(3)).await;

        alice.fetch_and_process_messages().await?;
        let mut messages = alice.get_messages(&group_id);
        assert_eq!(messages.len(), 2);

        // Sort by timestamp since relay order isn't guaranteed
        messages.sort_by_key(|m| m.timestamp);

        // Verify both messages are present (order may vary)
        let contents: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();
        assert!(contents.contains(&"Hello Bob!".to_string()));
        assert!(contents.contains(&"Hi Alice!".to_string()));

        Ok(())
    }
}
