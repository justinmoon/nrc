use crate::event_bus::{EventBus, UnifiedEvent};
use anyhow::Result;
use crossterm::event::KeyEvent;
use nostr_mls::{groups::NostrGroupConfigData, messages::MessageProcessingResult, NostrMls};
use nostr_mls_sqlite_storage::NostrMlsSqliteStorage;
use nostr_mls_storage::groups::types as group_types;
use nostr_sdk::prelude::*;
use openmls::group::GroupId;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc;

pub mod event_bus;
pub mod handlers;
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
    ImportExisting,
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

    // Event bus - now mandatory
    event_bus: EventBus,
    internal_event_rx: Option<mpsc::UnboundedReceiver<UnifiedEvent>>,
}

impl Nrc {
    pub async fn new(datadir: &Path) -> Result<Self> {
        let keys = Keys::generate();
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

        // Create datadir if it doesn't exist
        std::fs::create_dir_all(datadir)?;
        let db_path = datadir.join("nrc.db");
        log::info!("Using SQLite storage at: {db_path:?}");
        let storage = Box::new(NostrMls::new(NostrMlsSqliteStorage::new(db_path)?));

        // Initialize event bus
        let mut event_bus = EventBus::new();
        let internal_event_rx = event_bus.take_receiver();

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
            help_explicitly_requested: false,
            profiles: HashMap::new(),
            event_tx: None,
            command_tx: None,
            event_bus,
            internal_event_rx,
        })
    }

    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    pub async fn process_internal_events(&mut self) -> Result<()> {
        // Collect events first to avoid borrowing issues
        let mut events = Vec::new();
        if let Some(rx) = &mut self.internal_event_rx {
            // Use try_recv to avoid blocking
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
        }

        // Now process them
        for event in events {
            self.handle_internal_event(event).await?;
        }

        Ok(())
    }

    async fn handle_internal_event(&mut self, event: UnifiedEvent) -> Result<()> {
        use uuid::Uuid;

        match event {
            UnifiedEvent::KeyPress(key) => {
                // Process key presses for onboarding states
                if let AppState::Onboarding { input, mode } = &self.state {
                    match mode {
                        OnboardingMode::Choose => match key.code {
                            crossterm::event::KeyCode::Char('1') => {
                                let _ = self
                                    .event_bus
                                    .emit(UnifiedEvent::OnboardingChooseOption { option: 1 });
                            }
                            crossterm::event::KeyCode::Char('2') => {
                                let _ = self
                                    .event_bus
                                    .emit(UnifiedEvent::OnboardingChooseOption { option: 2 });
                            }
                            _ => {}
                        },
                        OnboardingMode::EnterDisplayName | OnboardingMode::ImportExisting => {
                            match key.code {
                                crossterm::event::KeyCode::Char(c) => {
                                    let _ = self
                                        .event_bus
                                        .emit(UnifiedEvent::OnboardingInputChar { c });
                                }
                                crossterm::event::KeyCode::Backspace => {
                                    let _ = self.event_bus.emit(UnifiedEvent::OnboardingBackspace);
                                }
                                crossterm::event::KeyCode::Enter if !input.is_empty() => {
                                    let _ = self.event_bus.emit(UnifiedEvent::OnboardingSubmit {
                                        input: input.clone(),
                                    });
                                }
                                crossterm::event::KeyCode::Esc => {
                                    let _ = self.event_bus.emit(UnifiedEvent::OnboardingEscape);
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
            UnifiedEvent::Command(cmd) if cmd.starts_with("/profile") || cmd.starts_with("/p ") => {
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                if let Some(npub_str) = parts.get(1) {
                    if let Ok(pubkey) = PublicKey::from_bech32(npub_str) {
                        // Emit fetch request
                        let _ = self.event_bus.emit(UnifiedEvent::NostrFetch {
                            filter: Filter::new().kind(Kind::Metadata).author(pubkey).limit(1),
                            request_id: Uuid::new_v4(),
                        });
                    }
                }
            }
            UnifiedEvent::NostrFetch { filter, request_id } => {
                // Do the actual fetch
                let events = self
                    .client
                    .fetch_events(filter, Duration::from_secs(5))
                    .await?;
                let _ = self.event_bus.emit(UnifiedEvent::NostrEventFetchComplete {
                    request_id,
                    events: events.into_iter().collect(),
                });
            }
            UnifiedEvent::NostrEventFetchComplete { events, .. } => {
                // Process the profile
                if let Some(event) = events.first() {
                    if event.kind == Kind::Metadata {
                        if let Ok(metadata) = Metadata::from_json(&event.content) {
                            self.profiles.insert(event.pubkey, metadata);
                            self.flash_message = Some("Profile fetched via event bus".to_string());
                        }
                    }
                }
            }
            UnifiedEvent::Quit => {
                // Log the quit event
                log::debug!("Event bus received quit command");
                // The actual quitting logic is handled in process_command return value
            }
            UnifiedEvent::ShowHelp => {
                log::debug!("Event bus received show help command");
                self.show_help = true;
                self.help_explicitly_requested = true;
            }
            UnifiedEvent::NextGroup => {
                log::debug!("Event bus received next group command");
                self.next_group();
            }
            UnifiedEvent::PrevGroup => {
                log::debug!("Event bus received prev group command");
                self.prev_group();
            }
            UnifiedEvent::CopyNpub => {
                log::debug!("Event bus received copy npub command");
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
                            self.flash_message =
                                Some(format!("Copied npub to clipboard via event bus: {npub}"));
                        }
                    }
                    Err(e) => {
                        self.last_error = Some(format!("Clipboard not available: {e}"));
                    }
                }
            }
            UnifiedEvent::JoinGroupCommand { npub } => {
                log::debug!("Event bus received join group command for: {npub}");

                // Parse the npub and validate it
                match PublicKey::from_str(&npub) {
                    Ok(pubkey) => {
                        log::debug!("Successfully parsed npub to pubkey: {pubkey}");

                        // Emit profile fetch request
                        log::debug!("Emitting profile fetch for: {pubkey}");
                        let _ = self.event_bus.emit(UnifiedEvent::NostrFetch {
                            filter: Filter::new().kind(Kind::Metadata).author(pubkey).limit(1),
                            request_id: uuid::Uuid::new_v4(),
                        });

                        // Check if already in group (simplified logic for now)
                        let already_in_group =
                            if let AppState::Ready { ref groups, .. } = self.state {
                                groups.iter().any(|group_id| {
                                    if let Some(group) = self.groups.get(group_id) {
                                        group.admin_pubkeys.contains(&pubkey)
                                    } else {
                                        false
                                    }
                                })
                            } else {
                                false
                            };

                        log::debug!("Already in group with {npub}: {already_in_group}");

                        if already_in_group {
                            self.flash_message = Some(format!("Already in a group with {npub}"));
                            return Ok(());
                        }

                        // Emit key package fetch request
                        log::debug!("Emitting fetch key package for: {pubkey}");
                        let _ = self.event_bus.emit(UnifiedEvent::FetchKeyPackage {
                            pubkey,
                            request_id: uuid::Uuid::new_v4(),
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to parse npub '{npub}': {e}");
                        self.last_error = Some(format!("Invalid public key: {e}"));
                    }
                }
            }
            UnifiedEvent::FetchKeyPackage { pubkey, request_id } => {
                log::debug!("Event bus fetching key package for: {pubkey}");

                match self.fetch_key_package(&pubkey).await {
                    Ok(key_package) => {
                        log::debug!("Key package fetched successfully, emitting KeyPackageFetched");
                        let _ = self.event_bus.emit(UnifiedEvent::KeyPackageFetched {
                            pubkey,
                            key_package,
                            request_id,
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to fetch key package: {e}");
                        self.last_error = Some(format!("Failed to fetch key package: {e}"));
                    }
                }
            }
            UnifiedEvent::KeyPackageFetched {
                pubkey,
                key_package,
                ..
            } => {
                log::debug!("Event bus received key package for: {pubkey}");

                // Create group with the member
                match self.create_group_with_member(key_package).await {
                    Ok(group_id) => {
                        // Send welcome
                        match self.get_welcome_rumor_for(&pubkey) {
                            Ok(welcome_rumor) => {
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

                        // Update state
                        if let AppState::Ready {
                            key_package_published,
                            groups,
                        } = &self.state
                        {
                            let mut updated_groups = groups.clone();
                            if !updated_groups.contains(&group_id) {
                                updated_groups.push(group_id.clone());
                            }
                            self.state = AppState::Ready {
                                key_package_published: *key_package_published,
                                groups: updated_groups.clone(),
                            };
                            if let Some(idx) = updated_groups.iter().position(|g| g == &group_id) {
                                self.selected_group_index = Some(idx);
                            }
                        }

                        self.flash_message =
                            Some("Group created and invitation sent via event bus!".to_string());
                    }
                    Err(e) => {
                        self.last_error = Some(format!("Failed to create group: {e}"));
                    }
                }
            }

            // Onboarding Events
            UnifiedEvent::OnboardingChooseOption { option } => {
                if let AppState::Onboarding { .. } = self.state {
                    match option {
                        1 => {
                            self.state = AppState::Onboarding {
                                input: String::new(),
                                mode: OnboardingMode::EnterDisplayName,
                            };
                        }
                        2 => {
                            self.state = AppState::Onboarding {
                                input: String::new(),
                                mode: OnboardingMode::ImportExisting,
                            };
                        }
                        _ => {}
                    }
                }
            }

            UnifiedEvent::OnboardingInputChar { c } => {
                if let AppState::Onboarding { ref mut input, .. } = self.state {
                    input.push(c);
                }
            }

            UnifiedEvent::OnboardingBackspace => {
                if let AppState::Onboarding { ref mut input, .. } = self.state {
                    input.pop();
                }
            }

            UnifiedEvent::OnboardingSubmit { input } => {
                if let AppState::Onboarding { mode, .. } = &self.state {
                    match mode {
                        OnboardingMode::EnterDisplayName => {
                            if !input.is_empty() {
                                // Emit initialization event instead of calling directly
                                let _ = self.event_bus.emit(UnifiedEvent::InitializeApp {
                                    display_name: Some(input),
                                    nsec: None,
                                });
                            }
                        }
                        OnboardingMode::ImportExisting => {
                            if !input.is_empty() {
                                // Emit initialization event instead of calling directly
                                let _ = self.event_bus.emit(UnifiedEvent::InitializeApp {
                                    display_name: None,
                                    nsec: Some(input),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }

            UnifiedEvent::OnboardingEscape => {
                if let AppState::Onboarding { .. } = self.state {
                    self.state = AppState::Onboarding {
                        input: String::new(),
                        mode: OnboardingMode::Choose,
                    };
                }
            }

            UnifiedEvent::InitializeApp { display_name, nsec } => {
                self.state = AppState::Initializing;

                if let Some(name) = display_name {
                    if let Err(e) = self.initialize_with_display_name(name).await {
                        log::error!("Failed to initialize with display name: {e}");
                        self.state = AppState::Onboarding {
                            input: String::new(),
                            mode: OnboardingMode::Choose,
                        };
                    }
                } else if let Some(nsec_str) = nsec {
                    if let Err(e) = self.initialize_with_nsec(nsec_str).await {
                        log::error!("Failed to initialize with nsec: {e}");
                        self.state = AppState::Onboarding {
                            input: String::new(),
                            mode: OnboardingMode::Choose,
                        };
                    }
                }
            }

            // Will be filled in gradually as we migrate commands
            _ => {
                log::debug!("Unhandled internal event: {event:?}");
            }
        }
        Ok(())
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
                log::debug!("Processing event: {}", event.id);
                if let Ok(MessageProcessingResult::ApplicationMessage(msg)) =
                    self.storage.process_message(&event)
                {
                    log::debug!("Got ApplicationMessage, kind: {}", msg.kind);
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
            if let Err(e) = self.send_message(group_id, input).await {
                self.last_error = Some(format!("Failed to send: {e}"));
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
            if let Err(e) = self.event_bus.emit(UnifiedEvent::Quit) {
                log::debug!("Failed to emit quit event: {e}");
            } else {
                // Event bus will handle it, but we still need to return true to quit
                return Ok(true);
            }
            return Ok(true);
        }

        // Handle profile fetch command
        if input.starts_with("/profile ") || input.starts_with("/p ") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            let npub = parts.get(1).map(|s| s.to_string());

            if let Err(e) = self.event_bus.emit(UnifiedEvent::Command(input.clone())) {
                log::debug!("Failed to emit command event: {e}");
            } else {
                // Return early - event bus will handle it
                return Ok(false);
            }

            // Direct handling
            if let Some(npub_str) = npub {
                if let Ok(pubkey) = PublicKey::from_bech32(&npub_str) {
                    self.fetch_profile(&pubkey).await?;
                    self.flash_message = Some("Profile fetched".to_string());
                } else {
                    self.last_error = Some("Invalid npub format".to_string());
                }
            } else {
                self.last_error = Some("Usage: /profile <npub>".to_string());
            }
            return Ok(false);
        }

        // Handle npub copy
        if input == "/npub" || input == "/n" {
            if let Err(e) = self.event_bus.emit(UnifiedEvent::CopyNpub) {
                log::debug!("Failed to emit copy npub event: {e}");
            } else {
                return Ok(false);
            }

            // Direct handling
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
            return Ok(false);
        }

        // Handle help command
        if input == "/help" || input == "/h" {
            if let Err(e) = self.event_bus.emit(UnifiedEvent::ShowHelp) {
                log::debug!("Failed to emit show help event: {e}");
            } else {
                // Return early - event bus will handle it
                return Ok(false);
            }

            // Fallback to direct handling if event bus fails
            self.show_help = true;
            self.help_explicitly_requested = true;
            return Ok(false);
        }

        // Handle navigation commands in Ready state
        if matches!(self.state, AppState::Ready { .. }) {
            if input == "/next" {
                if let Err(e) = self.event_bus.emit(UnifiedEvent::NextGroup) {
                    log::debug!("Failed to emit next group event: {e}");
                } else {
                    return Ok(false);
                }
                // Direct handling
                self.next_group();
                return Ok(false);
            } else if input == "/prev" {
                if let Err(e) = self.event_bus.emit(UnifiedEvent::PrevGroup) {
                    log::debug!("Failed to emit prev group event: {e}");
                } else {
                    return Ok(false);
                }
                // Direct handling
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

            // Try event bus first
            if let Err(e) = self.event_bus.emit(UnifiedEvent::JoinGroupCommand {
                npub: pubkey_str.to_string(),
            }) {
                log::debug!("Failed to emit join group command event: {e}");
            } else {
                // Return early - event bus will handle it
                return Ok(false);
            }
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
                                    log::info!(
                                        "Sending welcome to {}",
                                        pubkey_to_bech32_safe(&pubkey)
                                    );
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
                                if let Some(idx) =
                                    updated_groups.iter().position(|g| g == &group_id)
                                {
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
        } else if input.starts_with("/") {
            // Unknown command
            self.last_error =
                Some("Commands: /join <npub>, /npub, /next, /prev, /help, /quit".to_string());
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
        // Get the other member's display name from their profile
        if let Some(group) = self.groups.get(group_id) {
            // For now, assume it's a 1-on-1 chat
            let our_pubkey = self.keys.public_key();

            // First check if we know the admin (creator) of the group
            for admin in &group.admin_pubkeys {
                if admin != &our_pubkey {
                    return self.get_display_name_for_pubkey(admin);
                }
            }

            // Otherwise, try to find a member from messages
            if let Some(messages) = self.messages.get(group_id) {
                for msg in messages {
                    if msg.sender != our_pubkey {
                        // Found the other person
                        return self.get_display_name_for_pubkey(&msg.sender);
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

        if let Ok(MessageProcessingResult::ApplicationMessage(msg)) =
            self.storage.process_message(&event)
        {
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

// Tests have been moved to integration tests that properly use event-driven approach
// See tests/event_loop_integration.rs
/*
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
        let mut alice = Nrc::new(&temp_dir).await?;
        let mut bob = Nrc::new(&temp_dir).await?;

        // Initialize both clients (like pressing "1" to generate keys in onboarding)
        alice.initialize().await?;
        bob.initialize().await?;

        // Wait for key packages to propagate
        sleep(Duration::from_secs(3)).await;

        // Alice creates a group with Bob (like typing /j npub...)
        let bob_npub = bob.public_key().to_bech32()?;
        alice.process_input(format!("/j {bob_npub}")).await?;

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
        let alice_nostr_group = alice
            .groups
            .get(&alice_group)
            .expect("Alice should have group in storage");
        let bob_nostr_group = bob
            .groups
            .get(&bob_group)
            .expect("Bob should have group in storage");

        println!(
            "Alice's Nostr group ID: {}",
            hex::encode(alice_nostr_group.nostr_group_id)
        );
        println!(
            "Bob's Nostr group ID: {}",
            hex::encode(bob_nostr_group.nostr_group_id)
        );

        // They should now be in the SAME group!
        assert_eq!(
            alice_nostr_group.nostr_group_id, bob_nostr_group.nostr_group_id,
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
        let mut alice = Nrc::new(&temp_dir).await?;
        let mut bob = Nrc::new(&temp_dir).await?;

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

        alice
            .send_message(group_id.clone(), "Hello Bob!".to_string())
            .await?;
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

        bob.send_message(group_id.clone(), "Hi Alice!".to_string())
            .await?;

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
*/
