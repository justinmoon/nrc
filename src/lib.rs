use crate::key_storage::KeyStorage;
use anyhow::Result;
use crossterm::event::KeyEvent;
use nostr_sdk::prelude::*;
use nrc_mls::{groups::NostrGroupConfigData, messages::MessageProcessingResult, NostrMls};
use nrc_mls_sqlite_storage::NostrMlsSqliteStorage;
use nrc_mls_storage::groups::types as group_types;
use openmls::group::GroupId;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc;

pub mod key_storage;
pub mod notification_handler;
/// Get default relay URLs - uses local relay for tests when TEST_USE_LOCAL_RELAY is set
pub fn get_default_relays() -> &'static [&'static str] {
    #[cfg(test)]
    {
        if std::env::var("TEST_USE_LOCAL_RELAY").is_ok() {
            return &["ws://127.0.0.1:8080"];
        }
    }

    &[
        "wss://relay.damus.io",
        "wss://nos.lol",
        "wss://relay.nostr.band",
        "wss://relay.snort.social",
        "wss://nostr.wine",
    ]
}

/// Default relay URLs used throughout the application
pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.nostr.band",
    "wss://relay.snort.social",
    "wss://nostr.wine",
];

/// Helper function to safely convert PublicKey to bech32 with fallback
fn pubkey_to_bech32_safe(pubkey: &PublicKey) -> String {
    pubkey.to_bech32().unwrap_or_else(|_| "unknown".to_string())
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    // UI Events
    KeyPress(KeyEvent),
    Paste(String),

    // Network Events (from background task)
    MessageReceived { group_id: GroupId, message: Message },
    WelcomeReceived { welcome: UnsignedEvent },
    GroupCreated { group_id: GroupId },
    KeyPackagePublished,
    ProfilePublished,
    NetworkError { error: String },

    // Timer Events
    ProcessPendingOperationsTick,

    // Raw network data to be processed
    RawMessagesReceived { events: Vec<Event> },
    RawWelcomesReceived { events: Vec<Event> },
}

#[derive(Debug, Clone)]
pub enum NetworkCommand {
    SendMessage { group_id: GroupId, content: String },
    JoinGroup { npub: String },
    CreateGroup { name: String },
    PublishKeyPackage,
    PublishProfile { display_name: String },
    FetchMessages,
    FetchWelcomes,
}

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
    EnterDisplayName,
    CreatePassword,
    ImportExisting,
    EnterPassword,
}

/// Temporary data during onboarding
#[derive(Debug, Clone)]
pub struct OnboardingData {
    pub display_name: Option<String>,
    pub nsec: Option<String>,
}

pub struct Nrc {
    storage: Box<NostrMls<NostrMlsSqliteStorage>>,
    pub keys: Keys,
    pub client: Client,
    pub state: AppState,
    messages: HashMap<GroupId, Vec<Message>>,
    pub welcome_rumors: HashMap<PublicKey, UnsignedEvent>,
    pub groups: HashMap<GroupId, group_types::Group>,
    pub input: String,
    pub selected_group_index: Option<usize>,
    pub scroll_offset: u16,
    pub last_error: Option<String>,
    pub flash_message: Option<String>,
    pub show_help: bool,
    pub help_explicitly_requested: bool,
    profiles: HashMap<PublicKey, Metadata>,
    pub event_tx: Option<mpsc::UnboundedSender<AppEvent>>,
    pub command_tx: Option<mpsc::Sender<NetworkCommand>>,
    key_storage: KeyStorage,
    pub onboarding_data: OnboardingData,
}

impl Nrc {
    pub async fn new(datadir: &Path) -> Result<Self> {
        // Create datadir if it doesn't exist
        std::fs::create_dir_all(datadir)?;

        let key_storage = KeyStorage::new(datadir);

        // Check if we have existing keys
        let (keys, initial_state) = if key_storage.keys_exist() {
            // We have stored keys, prompt for password
            let keys = Keys::generate(); // Temporary, will be replaced when password entered
            (
                keys,
                AppState::Onboarding {
                    input: String::new(),
                    mode: OnboardingMode::EnterPassword,
                },
            )
        } else {
            // No stored keys, show regular onboarding
            let keys = Keys::generate();
            (
                keys,
                AppState::Onboarding {
                    input: String::new(),
                    mode: OnboardingMode::Choose,
                },
            )
        };

        let client = Client::builder().signer(keys.clone()).build();

        // Add multiple relays for redundancy
        for &relay in get_default_relays() {
            if let Err(e) = client.add_relay(relay).await {
                log::warn!("Failed to add relay {relay}: {e}");
            }
        }

        client.connect().await;

        // Wait for connections to establish
        tokio::time::sleep(Duration::from_secs(2)).await;

        let db_path = datadir.join("nrc.db");
        log::info!("Using SQLite storage at: {db_path:?}");
        let storage = Box::new(NostrMls::new(NostrMlsSqliteStorage::new(db_path)?));

        Ok(Self {
            storage,
            keys,
            client,
            state: initial_state,
            messages: HashMap::new(),
            welcome_rumors: HashMap::new(),
            groups: HashMap::new(),
            input: String::new(),
            selected_group_index: None,
            scroll_offset: 0,
            last_error: None,
            flash_message: None,
            show_help: false,
            help_explicitly_requested: false,
            profiles: HashMap::new(),
            event_tx: None,
            command_tx: None,
            key_storage,
            onboarding_data: OnboardingData {
                display_name: None,
                nsec: None,
            },
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

        let relays: Result<Vec<RelayUrl>, _> = get_default_relays()
            .iter()
            .map(|&url| RelayUrl::parse(url))
            .collect();
        let relays = relays?;
        let (key_package_content, tags) = self
            .storage
            .create_key_package_for_event(&self.keys.public_key(), relays)?;

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
            let events = self
                .client
                .fetch_events(filter.clone(), Duration::from_secs(5))
                .await?;

            if let Some(event) = events.into_iter().next() {
                log::debug!("Found key package on attempt {attempt}");
                return Ok(event);
            }
            log::debug!("Key package not found on attempt {attempt}");
        }

        Err(anyhow::anyhow!("No key package found for {pubkey}"))
    }

    pub async fn create_group(&mut self, name: String) -> Result<GroupId> {
        let config = NostrGroupConfigData::new(
            name,
            "NRC Chat Group".to_string(),
            None,
            None,
            None,
            vec![RelayUrl::parse(get_default_relays()[0])?],
            vec![self.keys.public_key()],
        );

        let group_result = self
            .storage
            .create_group(&self.keys.public_key(), vec![], config)?;
        let group_id = GroupId::from_slice(group_result.group.mls_group_id.as_slice());

        self.groups
            .insert(group_id.clone(), group_result.group.clone());

        // Subscribe to messages for this group
        let h_tag_value = hex::encode(group_result.group.nostr_group_id);
        let filter = Filter::new()
            .kind(Kind::from(445u16))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value)
            .limit(100);
        self.client.subscribe(filter, None).await?;

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

    pub async fn create_multi_user_group(
        &mut self,
        channel_name: String,
        member_pubkeys: Vec<PublicKey>,
    ) -> Result<()> {
        // Fetch key packages for all members
        let mut key_packages = Vec::new();
        for pubkey in &member_pubkeys {
            match self.fetch_key_package(pubkey).await {
                Ok(kp) => key_packages.push(kp),
                Err(e) => {
                    self.last_error = Some(format!(
                        "Failed to fetch key package for {}: {}",
                        pubkey_to_bech32_safe(pubkey),
                        e
                    ));
                    return Err(e);
                }
            }
        }

        // Parse relay URLs
        let relays: Result<Vec<RelayUrl>, _> = get_default_relays()
            .iter()
            .map(|&url| RelayUrl::parse(url))
            .collect();
        let relays = match relays {
            Ok(r) => r,
            Err(e) => {
                self.last_error = Some(format!("Invalid relay URLs: {e}"));
                return Err(e.into());
            }
        };

        let config = NostrGroupConfigData::new(
            channel_name.clone(),
            format!("NRC channel: {channel_name}"),
            None,
            None,
            None,
            relays,
            vec![self.keys.public_key()], // Creator is admin
        );

        // Create group with initial members
        match self
            .storage
            .create_group(&self.keys.public_key(), key_packages, config)
        {
            Ok(group_result) => {
                let group_id = GroupId::from_slice(group_result.group.mls_group_id.as_slice());

                self.groups
                    .insert(group_id.clone(), group_result.group.clone());

                // Subscribe to messages for this group
                let h_tag_value = hex::encode(group_result.group.nostr_group_id);
                let filter = Filter::new()
                    .kind(Kind::from(445u16))
                    .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value)
                    .limit(100);
                if let Err(e) = self.client.subscribe(filter, None).await {
                    log::warn!("Failed to subscribe to group messages: {e}");
                }

                if let AppState::Ready {
                    key_package_published,
                    mut groups,
                } = self.state.clone()
                {
                    groups.push(group_id.clone());
                    let new_index = groups.len() - 1;
                    self.state = AppState::Ready {
                        key_package_published,
                        groups,
                    };
                    // Set as selected group (most recently created)
                    self.selected_group_index = Some(new_index);
                }

                // Send welcomes to all members
                for (pubkey, welcome_rumor) in member_pubkeys
                    .iter()
                    .zip(group_result.welcome_rumors.iter())
                {
                    if let Err(e) = self
                        .send_gift_wrapped_welcome(pubkey, welcome_rumor.clone())
                        .await
                    {
                        log::warn!(
                            "Failed to send welcome to {}: {e}",
                            pubkey_to_bech32_safe(pubkey)
                        );
                    }
                }

                self.flash_message = Some(format!(
                    "Created #{channel_name} with {} members",
                    member_pubkeys.len()
                ));
                Ok(())
            }
            Err(e) => {
                self.last_error = Some(format!("Failed to create group: {e}"));
                Err(e.into())
            }
        }
    }

    pub async fn invite_to_group(
        &mut self,
        group_id: GroupId,
        new_member: PublicKey,
    ) -> Result<()> {
        // Check if we're admin of this group
        let group = self
            .groups
            .get(&group_id)
            .ok_or_else(|| anyhow::anyhow!("Group not found"))?;

        if !group.admin_pubkeys.contains(&self.keys.public_key()) {
            return Err(anyhow::anyhow!("Only admins can invite members"));
        }

        // Fetch the new member's key package
        let key_package = self.fetch_key_package(&new_member).await?;

        // Add member to the group
        let update_result = self.storage.add_members(&group_id, &[key_package])?;

        // CRITICAL: Merge the pending commit to update our local MLS group state
        // Without this, our group state remains at the old epoch and we can't
        // decrypt messages from the new member
        self.storage.merge_pending_commit(&group_id)?;

        // Send the MLS commit/evolution event
        self.client
            .send_event(&update_result.evolution_event)
            .await?;

        // Send welcome to the new member
        if let Some(welcome_rumors) = update_result.welcome_rumors {
            for welcome_rumor in welcome_rumors {
                self.send_gift_wrapped_welcome(&new_member, welcome_rumor)
                    .await?;
            }
        }

        Ok(())
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

        let group_result = self.storage.create_group(
            &self.keys.public_key(),
            vec![key_package.clone()],
            config,
        )?;

        let group_id = GroupId::from_slice(group_result.group.mls_group_id.as_slice());
        // Note: merge_pending_commit is already called inside create_group

        self.groups
            .insert(group_id.clone(), group_result.group.clone());

        // Subscribe to messages for this group
        let h_tag_value = hex::encode(group_result.group.nostr_group_id);
        let filter = Filter::new()
            .kind(Kind::from(445u16))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value)
            .limit(100);
        self.client.subscribe(filter, None).await?;

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

    /// Create a filter for fetching GiftWrap events for a specific recipient
    /// GiftWrap events use ephemeral pubkeys, so we filter by the p tag
    pub fn giftwrap_filter_for_recipient(recipient_pubkey: &PublicKey) -> Filter {
        Filter::new().kind(Kind::GiftWrap).custom_tag(
            SingleLetterTag::lowercase(Alphabet::P),
            recipient_pubkey.to_hex(),
        )
    }

    pub async fn fetch_and_process_welcomes(&mut self) -> Result<()> {
        let filter = Nrc::giftwrap_filter_for_recipient(&self.keys.public_key()).limit(10);

        tokio::time::sleep(Duration::from_secs(2)).await;

        let events = self
            .client
            .fetch_events(filter, Duration::from_secs(10))
            .await?;

        for gift_wrap in events {
            if let Ok(unwrapped) = self.client.unwrap_gift_wrap(&gift_wrap).await {
                if unwrapped.rumor.kind == Kind::from(444u16) {
                    // Process the welcome to add it to pending welcomes
                    let welcome = self
                        .storage
                        .process_welcome(&gift_wrap.id, &unwrapped.rumor)?;

                    // Accept the welcome to actually join the group
                    self.storage.accept_welcome(&welcome)?;

                    // Get the group info from storage after accepting
                    let group_id = GroupId::from_slice(welcome.mls_group_id.as_slice());
                    if let Ok(Some(group)) = self.storage.get_group(&group_id) {
                        self.groups.insert(group_id.clone(), group.clone());

                        // Subscribe to messages for this group
                        let h_tag_value = hex::encode(group.nostr_group_id);
                        let filter = Filter::new()
                            .kind(Kind::from(445u16))
                            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value)
                            .limit(100);
                        let _ = self.client.subscribe(filter, None).await;

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
                        // Select the newly joined group
                        self.selected_group_index = Some(groups.len() - 1);
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
        let group_id_clone = group_id.clone();
        let content = &content;
        let text_note_rumor = EventBuilder::text_note(content).build(self.keys.public_key());

        let event = self
            .storage
            .create_message(&group_id_clone, text_note_rumor.clone())?;

        // Note: merge_pending_commit is already called inside create_message

        // Send message directly - retry logic can be added later if needed
        log::debug!(
            "Sending message event: id={}, kind={}",
            event.id,
            event.kind
        );
        self.client.send_event(&event).await?;

        // Add our own message to local history immediately
        // Since we're sending to ourselves, it won't come back from the relay
        // TODO: In the future, we could implement proper deduplication to handle
        // cases where our own messages might be received from relays
        let message = Message {
            content: content.clone(),
            sender: self.keys.public_key(),
            timestamp: text_note_rumor.created_at,
        };
        self.add_message(group_id, message);

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
                    log::warn!(
                        "Group not found in storage: {}",
                        hex::encode(group_id.as_slice())
                    );
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
                log::info!(
                    "Processing event: {} from {}",
                    event.id,
                    event
                        .pubkey
                        .to_bech32()
                        .unwrap_or_else(|_| "unknown".to_string())
                );
                match self.storage.process_message(&event) {
                    Ok(MessageProcessingResult::ApplicationMessage(msg)) => {
                        log::info!(
                            "Got ApplicationMessage, kind: {} from {}",
                            msg.kind,
                            msg.pubkey
                                .to_bech32()
                                .unwrap_or_else(|_| "unknown".to_string())
                        );
                        if msg.kind == Kind::TextNote {
                            if let Ok(Some(stored_msg)) = self.storage.get_message(&msg.id) {
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
                                    log::info!(
                                        "Adding message to group: '{}' from {}",
                                        message.content,
                                        pubkey_to_bech32_safe(&message.sender)
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
                    Ok(MessageProcessingResult::Commit) => {
                        log::info!("Processed commit/evolution event - group state updated");
                    }
                    Ok(MessageProcessingResult::Proposal(_)) => {
                        log::debug!("Processed proposal");
                    }
                    Ok(MessageProcessingResult::ExternalJoinProposal) => {
                        log::debug!("Processed external join proposal");
                    }
                    Ok(MessageProcessingResult::Unprocessable) => {
                        log::debug!("Message was unprocessable");
                    }
                    Err(e) => {
                        log::warn!("Failed to process message: {e}");
                    }
                }
            }
        }

        Ok(())
    }

    /// Get the currently selected group ID, ensuring consistency between UI and state
    pub fn get_selected_group(&self) -> Option<GroupId> {
        if let AppState::Ready { ref groups, .. } = &self.state {
            self.selected_group_index
                .and_then(|idx| groups.get(idx))
                .cloned()
        } else {
            None
        }
    }

    pub fn get_messages(&self, group_id: &GroupId) -> Vec<Message> {
        self.messages.get(group_id).cloned().unwrap_or_default()
    }

    pub async fn initialize(&mut self) -> Result<()> {
        self.state = AppState::Initializing;

        self.publish_key_package().await?;

        let groups = self.storage.get_groups()?;
        let group_ids: Vec<GroupId> = groups.iter().map(|g| g.mls_group_id.clone()).collect();

        // Store groups in our HashMap for later use and subscribe to messages
        for group in groups {
            self.groups
                .insert(group.mls_group_id.clone(), group.clone());

            // Subscribe to messages for this group
            let h_tag_value = hex::encode(group.nostr_group_id);
            let filter = Filter::new()
                .kind(Kind::from(445u16))
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value)
                .limit(100);
            let _ = self.client.subscribe(filter, None).await;
        }

        self.state = AppState::Ready {
            key_package_published: true,
            groups: group_ids,
        };
        Ok(())
    }

    pub async fn initialize_with_display_name(&mut self, display_name: String) -> Result<()> {
        self.state = AppState::Initializing;

        // Publish profile with display name
        self.publish_profile(display_name).await?;

        // Then publish key package
        self.publish_key_package().await?;

        let groups = self.storage.get_groups()?;
        let group_ids: Vec<GroupId> = groups.iter().map(|g| g.mls_group_id.clone()).collect();

        // Store groups in our HashMap for later use and subscribe to messages
        for group in groups {
            self.groups
                .insert(group.mls_group_id.clone(), group.clone());

            // Subscribe to messages for this group
            let h_tag_value = hex::encode(group.nostr_group_id);
            let filter = Filter::new()
                .kind(Kind::from(445u16))
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value)
                .limit(100);
            let _ = self.client.subscribe(filter, None).await;
        }

        self.state = AppState::Ready {
            key_package_published: true,
            groups: group_ids,
        };
        Ok(())
    }

    pub async fn publish_profile(&mut self, display_name: String) -> Result<()> {
        // Create metadata for the profile
        let metadata = Metadata::new()
            .display_name(display_name.clone())
            .name(display_name.clone());

        // Store it locally too
        self.profiles
            .insert(self.keys.public_key(), metadata.clone());

        // Publish to Nostr
        let event = EventBuilder::metadata(&metadata).sign(&self.keys).await?;
        self.client.send_event(&event).await?;

        log::info!("Published profile with display name: {display_name}");
        Ok(())
    }

    pub async fn initialize_with_nsec(&mut self, nsec: String) -> Result<()> {
        let keys = Keys::parse(&nsec)?;
        self.keys = keys;
        self.client = Client::builder().signer(self.keys.clone()).build();

        for &relay in get_default_relays() {
            if let Err(e) = self.client.add_relay(relay).await {
                log::warn!("Failed to add relay {relay}: {e}");
            }
        }

        self.client.connect().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        self.initialize().await
    }

    /// Initialize with a display name and password (for new users)
    pub async fn initialize_with_display_name_and_password(
        &mut self,
        display_name: String,
        password: String,
    ) -> Result<()> {
        self.state = AppState::Initializing;

        // Save the keys encrypted with the password (npub is derived from keys)
        self.key_storage.save_encrypted(&self.keys, &password)?;

        // Publish profile with display name
        self.publish_profile(display_name).await?;

        // Then publish key package
        self.publish_key_package().await?;

        // Load groups and set up subscriptions
        self.initialize_groups().await?;

        Ok(())
    }

    /// Initialize with nsec and password (for importing existing keys)
    pub async fn initialize_with_nsec_and_password(
        &mut self,
        nsec: String,
        password: String,
    ) -> Result<()> {
        let keys = Keys::parse(&nsec)?;
        self.keys = keys;

        // Save the imported keys encrypted with the password (npub is derived from keys)
        self.key_storage.save_encrypted(&self.keys, &password)?;

        // Recreate client with new keys
        self.client = Client::builder().signer(self.keys.clone()).build();

        for &relay in get_default_relays() {
            if let Err(e) = self.client.add_relay(relay).await {
                log::warn!("Failed to add relay {relay}: {e}");
            }
        }

        self.client.connect().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        self.initialize().await
    }

    /// Initialize with password (for returning users)
    pub async fn initialize_with_password(&mut self, password: String) -> Result<()> {
        // Load and decrypt the stored keys
        let keys = self.key_storage.load_encrypted(&password)?;
        self.keys = keys;

        let npub = self.keys.public_key().to_bech32()?;
        log::info!("Loaded keys for npub: {npub}");

        // Recreate client with loaded keys
        self.client = Client::builder().signer(self.keys.clone()).build();

        for &relay in get_default_relays() {
            if let Err(e) = self.client.add_relay(relay).await {
                log::warn!("Failed to add relay {relay}: {e}");
            }
        }

        self.client.connect().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        self.initialize().await
    }

    /// Helper to initialize groups after keys are set up
    async fn initialize_groups(&mut self) -> Result<()> {
        let groups = self.storage.get_groups()?;
        let group_ids: Vec<GroupId> = groups.iter().map(|g| g.mls_group_id.clone()).collect();

        // Store groups in our HashMap for later use and subscribe to messages
        for group in groups {
            self.groups
                .insert(group.mls_group_id.clone(), group.clone());

            // Subscribe to messages for this group
            let h_tag_value = hex::encode(group.nostr_group_id);
            let filter = Filter::new()
                .kind(Kind::from(445u16))
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value)
                .limit(100);
            let _ = self.client.subscribe(filter, None).await;
        }

        self.state = AppState::Ready {
            key_package_published: true,
            groups: group_ids,
        };
        Ok(())
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

    pub fn get_member_count(&self, group_id: &GroupId) -> Option<usize> {
        match self.storage.get_members(group_id) {
            Ok(members) => Some(members.len()),
            Err(_) => None,
        }
    }

    pub fn add_group(&mut self, group_id: GroupId) {
        if let AppState::Ready {
            key_package_published,
            mut groups,
        } = self.state.clone()
        {
            if !groups.contains(&group_id) {
                groups.push(group_id);
            }
            self.state = AppState::Ready {
                key_package_published,
                groups,
            };
        }
    }

    pub fn add_message(&mut self, group_id: GroupId, message: Message) {
        self.messages.entry(group_id).or_default().push(message);
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
        if let Some(group_id) = self.get_selected_group() {
            log::info!(
                "Sending message '{}' to group {}",
                input,
                hex::encode(group_id.as_slice())
            );
            if let Err(e) = self.send_message(group_id.clone(), input).await {
                self.last_error = Some(format!("Failed to send: {e}"));
            } else {
                log::info!(
                    "Message sent successfully to group {}",
                    hex::encode(group_id.as_slice())
                );
            }
        } else {
            self.last_error = Some("No chat selected".to_string());
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
            return self.handle_npub_copy().await;
        }

        // Handle help command
        if input == "/help" || input == "/h" {
            self.show_help = true;
            self.help_explicitly_requested = true;
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

        // Handle /group command for multi-user groups (formerly /create)
        if input.starts_with("/group ") || input.starts_with("/g ") {
            return self.handle_group_command(&input).await;
        }

        // Handle /invite command
        if input.starts_with("/invite ") || input.starts_with("/i ") {
            return self.handle_invite_command(&input).await;
        }

        // Handle /members command
        if input == "/members" || input == "/m" {
            return self.handle_members_command().await;
        }

        // Handle /leave command
        if input == "/leave" || input == "/l" {
            return self.handle_leave_command().await;
        }

        // Handle /dm command for direct messages (formerly /join)
        if input.starts_with("/dm ") || input.starts_with("/d ") {
            return self.handle_dm_command(&input).await;
        } else if input.starts_with("/") {
            // Unknown command
            self.last_error = Some(
                "Commands: /group, /dm, /invite, /members, /leave, /npub, /help, /quit".to_string(),
            );
        }
        // If not a command, it's a regular message - don't set an error

        Ok(false)
    }

    async fn handle_npub_copy(&mut self) -> Result<bool> {
        use clipboard::ClipboardContext;
        use clipboard::ClipboardProvider;
        use nostr_sdk::prelude::ToBech32;

        let npub = self
            .keys
            .public_key()
            .to_bech32()
            .unwrap_or_else(|_| "error".to_string());

        match ClipboardContext::new() {
            Ok(mut ctx) => {
                if let Err(e) = ctx.set_contents(npub.clone()) {
                    self.last_error = Some(format!("Failed to copy: {e}"));
                } else {
                    self.flash_message = Some(format!("Copied npub to clipboard: {npub}"));
                }
            }
            Err(e) => {
                self.last_error = Some(format!("Clipboard not available: {e}"));
            }
        }
        Ok(false)
    }

    async fn handle_group_command(&mut self, input: &str) -> Result<bool> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.len() < 3 {
            self.last_error = Some("Usage: /group #channelname <npub1> [npub2] ...".to_string());
            return Ok(false);
        }

        let channel_name = parts[1].trim_start_matches('#');
        if channel_name.is_empty() {
            self.last_error = Some("Channel name cannot be empty".to_string());
            return Ok(false);
        }

        // Parse member public keys
        let mut member_pubkeys = Vec::new();
        for pubkey_str in &parts[2..] {
            match PublicKey::from_str(pubkey_str) {
                Ok(pubkey) => member_pubkeys.push(pubkey),
                Err(e) => {
                    self.last_error = Some(format!("Invalid public key {pubkey_str}: {e}"));
                    return Ok(false);
                }
            }
        }

        match self
            .create_multi_user_group(channel_name.to_string(), member_pubkeys)
            .await
        {
            Ok(()) => {
                // Flash message is set by create_multi_user_group
            }
            Err(e) => {
                self.last_error = Some(format!("Failed to create group: {e}"));
            }
        }
        Ok(false)
    }

    async fn handle_dm_command(&mut self, input: &str) -> Result<bool> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.len() < 2 {
            self.last_error = Some("Usage: /dm <npub>".to_string());
            return Ok(false);
        }

        let pubkey_str = parts[1];
        let pubkey = match PublicKey::from_str(pubkey_str) {
            Ok(pk) => pk,
            Err(e) => {
                self.last_error = Some(format!("Invalid public key: {e}"));
                return Ok(false);
            }
        };

        // Fetch their profile first
        let _ = self.fetch_profile(&pubkey).await;

        // IMPORTANT: First check if they already sent us a welcome
        // This prevents creating duplicate groups
        log::info!("Checking for existing welcomes before creating group with {pubkey_str}");
        // NOTE: In production, welcomes are fetched via timer events (FetchWelcomesTick)
        // We should not fetch them directly here - the timer will handle it

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
            self.flash_message = Some(format!("Already in a group with {pubkey_str}"));
            log::info!("Already in a group with {pubkey_str}, not creating a new one");
            return Ok(false);
        }

        // If not already in a group, fetch their key package and create one
        match self.fetch_key_package(&pubkey).await {
            Ok(key_package) => {
                // Create a group with them
                match self.create_group_with_member(key_package).await {
                    Ok(group_id) => {
                        // Send them the welcome
                        match self.get_welcome_rumor_for(&pubkey) {
                            Ok(welcome_rumor) => {
                                log::info!("Sending welcome to {}", pubkey_to_bech32_safe(&pubkey));
                                if let Err(e) =
                                    self.send_gift_wrapped_welcome(&pubkey, welcome_rumor).await
                                {
                                    log::error!("Failed to send welcome: {e}");
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to get welcome rumor: {e}");
                            }
                        }

                        // Update our state to show the new group
                        if let AppState::Ready {
                            key_package_published,
                            groups,
                        } = &self.state
                        {
                            let mut updated_groups = groups.clone();
                            if !updated_groups.contains(&group_id) {
                                updated_groups.push(group_id.clone());
                            }
                            // Update state first, then set selected index to ensure consistency
                            self.state = AppState::Ready {
                                key_package_published: *key_package_published,
                                groups: updated_groups.clone(),
                            };
                            // Now safely select the newly created group
                            if let Some(idx) = updated_groups.iter().position(|g| g == &group_id) {
                                self.selected_group_index = Some(idx);
                            }
                        }
                    }
                    Err(e) => {
                        self.last_error = Some(format!("Failed to create group: {e}"));
                    }
                }
            }
            Err(e) => {
                self.last_error = Some(format!("Failed to fetch key package: {e}"));
            }
        }
        Ok(false)
    }

    async fn handle_invite_command(&mut self, input: &str) -> Result<bool> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.len() < 2 {
            self.last_error = Some("Usage: /invite <npub>".to_string());
            return Ok(false);
        }

        let pubkey_str = parts[1];
        let pubkey = match PublicKey::from_str(pubkey_str) {
            Ok(pk) => pk,
            Err(e) => {
                self.last_error = Some(format!("Invalid public key: {e}"));
                return Ok(false);
            }
        };

        if let Some(group_id) = self.get_selected_group() {
            match self.invite_to_group(group_id, pubkey).await {
                Ok(_) => {
                    self.flash_message = Some(format!("Invited {pubkey_str} to group"));
                }
                Err(e) => {
                    self.last_error = Some(format!("Failed to invite: {e}"));
                }
            }
        } else {
            self.last_error = Some("No group selected. Select a group first.".to_string());
        }
        Ok(false)
    }

    async fn handle_members_command(&mut self) -> Result<bool> {
        if let Some(group_id) = self.get_selected_group() {
            match self.storage.get_members(&group_id) {
                Ok(members) => {
                    let member_list: Vec<String> = members
                        .iter()
                        .map(|pk| self.get_display_name_for_pubkey(pk))
                        .collect();
                    self.flash_message = Some(format!("Members: {}", member_list.join(", ")));
                }
                Err(e) => {
                    self.last_error = Some(format!("Failed to get members: {e}"));
                }
            }
        } else {
            self.last_error = Some("No group selected".to_string());
        }
        Ok(false)
    }

    async fn handle_leave_command(&mut self) -> Result<bool> {
        if let Some(group_id) = self.get_selected_group() {
            // For now, just remove from local state
            // TODO: Implement proper MLS leave group
            if let AppState::Ready {
                key_package_published,
                mut groups,
            } = self.state.clone()
            {
                groups.retain(|g| g != &group_id);
                self.groups.remove(&group_id);
                self.messages.remove(&group_id);
                self.selected_group_index = None;
                self.state = AppState::Ready {
                    key_package_published,
                    groups,
                };
                self.flash_message = Some("Left the group".to_string());
            }
        } else {
            self.last_error = Some("No group selected".to_string());
        }
        Ok(false)
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
        self.flash_message = None;
    }

    pub fn dismiss_help(&mut self) {
        self.show_help = false;
        self.help_explicitly_requested = false;
    }

    pub fn get_display_name_for_pubkey(&self, pubkey: &PublicKey) -> String {
        if let Some(profile) = self.profiles.get(pubkey) {
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

        // Fall back to npub if no profile
        pubkey
            .to_bech32()
            .map(|npub| {
                // Shorten npub for display: npub1abc...xyz
                if npub.len() > 20 {
                    format!("{}...{}", &npub[..10], &npub[npub.len() - 3..])
                } else {
                    npub
                }
            })
            .unwrap_or_else(|_| "Unknown".to_string())
    }

    pub fn get_chat_display_name(&self, group_id: &GroupId) -> String {
        if let Some(group) = self.groups.get(group_id) {
            // Check if it's a multi-user group by member count
            match self.storage.get_members(group_id) {
                Ok(members) => {
                    if members.len() > 2 {
                        // Multi-user group - show as #channelname
                        return format!("#{}", group.name);
                    } else if members.len() == 2 {
                        // Direct message - show other person's name
                        let our_pubkey = self.keys.public_key();
                        for member in &members {
                            if member != &our_pubkey {
                                return self.get_display_name_for_pubkey(member);
                            }
                        }
                    }
                }
                Err(_) => {
                    // Fallback to old logic if we can't get members
                    let our_pubkey = self.keys.public_key();

                    // Check if group name looks like a channel (not "Test Group")
                    if !group.name.starts_with("Test") && !group.name.is_empty() {
                        // Likely a multi-user group with a proper name
                        return format!("#{}", group.name);
                    }

                    // Otherwise treat as DM - find the other person
                    for admin in &group.admin_pubkeys {
                        if admin != &our_pubkey {
                            return self.get_display_name_for_pubkey(admin);
                        }
                    }

                    // Try to find from messages
                    if let Some(messages) = self.messages.get(group_id) {
                        for msg in messages {
                            if msg.sender != our_pubkey {
                                return self.get_display_name_for_pubkey(&msg.sender);
                            }
                        }
                    }
                }
            }
        }
        "Unknown".to_string()
    }

    pub async fn fetch_profile(&mut self, pubkey: &PublicKey) -> Result<()> {
        let filter = Filter::new().kind(Kind::Metadata).author(*pubkey).limit(1);

        let events = self
            .client
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

    /// Test helper: Check if we have a welcome rumor for a pubkey
    pub fn has_welcome_rumor_for(&self, pubkey: &PublicKey) -> bool {
        self.welcome_rumors.contains_key(pubkey)
    }

    /// Process a single message event that was fetched in the background
    pub async fn process_message_event(&mut self, event: Event) -> Result<()> {
        log::debug!("Processing message event: {}", event.id);

        match self.storage.process_message(&event) {
            Ok(MessageProcessingResult::ApplicationMessage(msg)) => {
                log::debug!("Got ApplicationMessage, kind: {}", msg.kind);
                if msg.kind == Kind::TextNote {
                    if let Ok(Some(stored_msg)) = self.storage.get_message(&msg.id) {
                        // Find which group this belongs to based on the h tag
                        for (group_id, group) in &self.groups {
                            let h_tag_value = hex::encode(group.nostr_group_id);

                            // Check if this message belongs to this group
                            let belongs_to_group = event.tags.iter().any(|tag| {
                                tag.as_slice().len() >= 2
                                    && tag.as_slice()[0] == "h"
                                    && tag.as_slice()[1] == h_tag_value
                            });

                            if belongs_to_group {
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
                                    log::info!(
                                        "Adding message to group: '{}' from {}",
                                        message.content,
                                        pubkey_to_bech32_safe(&message.sender)
                                    );
                                    messages.push(message.clone());

                                    // Fetch profile in background if we don't have it
                                    if !self.profiles.contains_key(&message.sender) {
                                        let _ = self.fetch_profile(&message.sender).await;
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }
            Ok(MessageProcessingResult::Commit) => {
                log::info!(
                    "Processed commit/evolution event via subscription - group state updated"
                );
            }
            Ok(MessageProcessingResult::Proposal(_)) => {
                log::debug!("Processed proposal via subscription");
            }
            Ok(MessageProcessingResult::ExternalJoinProposal) => {
                log::debug!("Processed external join proposal via subscription");
            }
            Ok(MessageProcessingResult::Unprocessable) => {
                log::debug!("Message was unprocessable");
            }
            Err(e) => {
                log::debug!("Failed to process message event: {e}");
            }
        }

        Ok(())
    }

    /// Process a single welcome event that was fetched in the background
    pub async fn process_welcome_event(&mut self, event: Event) -> Result<()> {
        if event.kind != Kind::GiftWrap {
            return Ok(());
        }

        match self.client.unwrap_gift_wrap(&event).await {
            Ok(unwrapped) => {
                // Check if this is a welcome message (kind 444)
                if unwrapped.rumor.kind != Kind::from(444u16) {
                    return Ok(());
                }

                // Process the welcome to add it to pending welcomes
                match self.storage.process_welcome(&event.id, &unwrapped.rumor) {
                    Ok(welcome) => {
                        // Accept the welcome to actually join the group
                        if let Ok(()) = self.storage.accept_welcome(&welcome) {
                            let group_id = GroupId::from_slice(welcome.mls_group_id.as_slice());
                            log::info!(
                                "Auto-joining group via welcome: {}",
                                hex::encode(group_id.as_slice())
                            );

                            // Get the group info from storage after accepting
                            if let Ok(Some(group)) = self.storage.get_group(&group_id) {
                                self.groups.insert(group_id.clone(), group.clone());

                                // Subscribe to messages for this group
                                let h_tag_value = hex::encode(group.nostr_group_id);
                                let filter = Filter::new()
                                    .kind(Kind::from(445u16))
                                    .custom_tag(
                                        SingleLetterTag::lowercase(Alphabet::H),
                                        h_tag_value,
                                    )
                                    .limit(100);
                                let _ = self.client.subscribe(filter, None).await;

                                // Fetch the profile of the person who invited us
                                if let Some(admin) = group.admin_pubkeys.first() {
                                    let _ = self.fetch_profile(admin).await;
                                }
                            }

                            // Update state to include new group
                            if let AppState::Ready {
                                key_package_published,
                                mut groups,
                            } = self.state.clone()
                            {
                                if !groups.contains(&group_id) {
                                    groups.push(group_id.clone());
                                    // Update state first, then set selected index to ensure consistency
                                    self.state = AppState::Ready {
                                        key_package_published,
                                        groups: groups.clone(),
                                    };
                                    // Now safely select the newly joined group
                                    if let Some(idx) = groups.iter().position(|g| g == &group_id) {
                                        self.selected_group_index = Some(idx);
                                    }
                                    self.flash_message =
                                        Some("Joined new group via invitation!".to_string());
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::debug!("Not a welcome or already processed: {e}");
                    }
                }
            }
            Err(e) => {
                log::debug!("Failed to unwrap gift wrap: {e}");
            }
        }

        Ok(())
    }
}
