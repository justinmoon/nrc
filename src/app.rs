use anyhow::Result;
use clipboard::ClipboardProvider;
use nostr_sdk::nips::nip59;
use nostr_sdk::prelude::*;
use nrc_mls::groups::NostrGroupConfigData;
use nrc_mls::{messages::MessageProcessingResult, NostrMls};
use nrc_mls_sqlite_storage::NostrMlsSqliteStorage;
use openmls::group::GroupId;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch, Mutex};

use crate::config::get_default_relays;
use crate::events::{AppEvent, NetworkCommand};
use crate::key_storage::KeyStorage;
use crate::ui_state::{GroupSummary, Message, Modal, Page, PageType};

pub struct App {
    pub current_page: Page,
    pub previous_page: Option<Page>,

    pub flash: Option<(String, Instant)>,
    pub modal: Option<Modal>,
    pub is_processing: bool,

    pub storage: Arc<NostrMls<NostrMlsSqliteStorage>>,
    pub client: Client,
    pub keys: Keys,
    pub key_storage: KeyStorage,

    state_tx: watch::Sender<Page>,
    state_rx: watch::Receiver<Page>,

    pub event_tx: mpsc::UnboundedSender<AppEvent>,
    pub event_rx: Option<mpsc::UnboundedReceiver<AppEvent>>,
    pub command_tx: mpsc::Sender<NetworkCommand>,

    pub profiles: Arc<Mutex<HashMap<PublicKey, Metadata>>>,
    pub welcome_rumors: Arc<Mutex<HashMap<PublicKey, UnsignedEvent>>>,
}

impl App {
    pub async fn new(
        storage: Arc<NostrMls<NostrMlsSqliteStorage>>,
        client: Client,
        keys: Keys,
        key_storage: KeyStorage,
        initial_page: Page,
    ) -> Result<Self> {
        let (state_tx, state_rx) = watch::channel(initial_page.clone());
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (command_tx, _command_rx) = mpsc::channel(100);

        // Add relays and connect (like master branch does)
        for &relay in get_default_relays() {
            if let Err(e) = client.add_relay(relay).await {
                log::warn!("Failed to add relay {relay}: {e}");
            }
        }

        client.connect().await;

        // Spawn notification handler to receive events from relays
        // This processes incoming welcomes, messages, etc in real-time
        crate::notification_handler::spawn_notification_handler(
            client.clone(),
            event_tx.clone(),
            keys.public_key(),
        );

        Ok(Self {
            current_page: initial_page,
            previous_page: None,
            flash: None,
            modal: None,
            is_processing: false,
            storage,
            client,
            keys,
            key_storage,
            state_tx,
            state_rx,
            event_tx,
            event_rx: Some(event_rx),
            command_tx,
            profiles: Arc::new(Mutex::new(HashMap::new())),
            welcome_rumors: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn navigate_to(&mut self, page_type: PageType) -> Result<()> {
        let new_page = self.load_page_data(page_type).await?;

        self.previous_page = Some(self.current_page.clone());
        self.current_page = new_page.clone();
        let _ = self.state_tx.send(new_page);

        Ok(())
    }

    pub async fn load_page_data(&self, page_type: PageType) -> Result<Page> {
        match page_type {
            PageType::Chat(maybe_group_id) => {
                let groups = self.load_group_summaries().await?;

                // If no group selected, show empty chat
                if let Some(group_id) = maybe_group_id {
                    let selected_group_index =
                        groups.iter().position(|g| g.id == group_id).unwrap_or(0);
                    let group_info = self
                        .storage
                        .get_group(&group_id)?
                        .ok_or_else(|| anyhow::anyhow!("Group not found"))?;
                    let messages = self.load_chat_messages(&group_id, 100).await?;
                    let members = self.load_group_members(&group_id).await?;

                    Ok(Page::Chat {
                        groups,
                        selected_group_index,
                        group_id,
                        group_info: Box::new(group_info),
                        messages,
                        members,
                        input: String::new(),
                        scroll_offset: 0,
                        typing_members: vec![],
                    })
                } else {
                    // No group selected - show empty state
                    Ok(Page::Chat {
                        groups,
                        selected_group_index: 0,
                        group_id: GroupId::from_slice(&[0u8; 32]),
                        group_info: Box::new(nrc_mls_storage::groups::types::Group {
                            mls_group_id: GroupId::from_slice(&[0u8; 32]),
                            nostr_group_id: [0u8; 32],
                            name: "Welcome to NRC".to_string(),
                            description: "Use /dm <npub> to start a conversation".to_string(),
                            admin_pubkeys: std::collections::BTreeSet::new(),
                            last_message_id: None,
                            last_message_at: None,
                            epoch: 0,
                            state: nrc_mls_storage::groups::types::GroupState::Active,
                            image_url: None,
                            image_key: None,
                            image_nonce: None,
                        }),
                        messages: vec![],
                        members: vec![],
                        input: String::new(),
                        scroll_offset: 0,
                        typing_members: vec![],
                    })
                }
            }
            PageType::Help => Ok(Page::Help {
                selected_section: 0,
            }),
            PageType::Onboarding => Ok(Page::Onboarding {
                input: String::new(),
                mode: crate::ui_state::OnboardingMode::Choose,
                error: None,
            }),
            PageType::Initializing => Ok(Page::Initializing {
                message: "Initializing...".to_string(),
                progress: 0.0,
            }),
        }
    }

    pub fn can_navigate_back(&self) -> bool {
        self.previous_page.is_some()
    }

    pub async fn navigate_back(&mut self) -> Result<()> {
        if let Some(prev) = self.previous_page.take() {
            self.current_page = prev.clone();
            let _ = self.state_tx.send(prev);
        }
        Ok(())
    }

    pub async fn handle_event(&mut self, event: AppEvent) -> Result<()> {
        match event {
            AppEvent::Navigate(page) => {
                self.previous_page = Some(self.current_page.clone());
                self.current_page = page.clone();
                let _ = self.state_tx.send(page);
            }
            AppEvent::NavigateBack => {
                self.navigate_back().await?;
            }
            AppEvent::KeyPress(key_event) => {
                self.handle_keypress(key_event).await?;
            }
            AppEvent::SendMessage(content) => {
                if let Page::Chat {
                    group_id,
                    input,
                    messages,
                    ..
                } = &mut self.current_page
                {
                    if !content.is_empty() {
                        // Create the MLS message (following mls_memory.rs pattern)
                        let rumor = EventBuilder::new(Kind::Custom(9), content.clone())
                            .build(self.keys.public_key());

                        match self.storage.create_message(group_id, rumor) {
                            Ok(message_event) => {
                                // Publish the message to relays
                                match self.client.send_event(&message_event).await {
                                    Ok(_) => {
                                        log::info!("Message sent successfully");

                                        // Add to local messages immediately for UI feedback
                                        messages.push(Message {
                                            content: content.clone(),
                                            sender: self.keys.public_key(),
                                            timestamp: Timestamp::now(),
                                        });
                                    }
                                    Err(e) => {
                                        log::error!("Failed to send message: {e}");
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to create MLS message: {e}");
                            }
                        }

                        input.clear();
                        let _ = self.state_tx.send(self.current_page.clone());
                    }
                }
            }
            AppEvent::CreateGroup { name, members: _ } => {
                let _ = self
                    .command_tx
                    .send(NetworkCommand::CreateGroup { name })
                    .await;
                self.navigate_to(PageType::Chat(None)).await?;
            }
            AppEvent::JoinGroup(group_id) => {
                self.navigate_to(PageType::Chat(Some(group_id))).await?;
            }
            AppEvent::FlashMessage(msg, duration) => {
                self.flash = Some((msg, Instant::now() + duration));
            }
            AppEvent::ClearFlash => {
                self.flash = None;
            }
            AppEvent::Resize => {
                // Just trigger a re-render by sending current state
                let _ = self.state_tx.send(self.current_page.clone());
            }
            AppEvent::Paste(text) => {
                // Add pasted text to current input field
                if let Page::Onboarding { input, .. } | Page::Chat { input, .. } =
                    &mut self.current_page
                {
                    input.push_str(&text);
                    let _ = self.state_tx.send(self.current_page.clone());
                }
            }
            AppEvent::RefreshCurrentPage => {
                let page_type = self.current_page.page_type();
                let refreshed = self.load_page_data(page_type).await?;
                self.current_page = refreshed.clone();
                let _ = self.state_tx.send(refreshed);
            }
            AppEvent::MessageReceived { group_id, message } => {
                if let Page::Chat {
                    group_id: current_id,
                    messages,
                    ..
                } = &mut self.current_page
                {
                    if current_id == &group_id {
                        messages.push(message);
                        let _ = self.state_tx.send(self.current_page.clone());
                    }
                }
            }
            AppEvent::RawWelcomesReceived { events } => {
                // Process incoming GiftWrap events (welcomes)
                for gift_wrap in events {
                    log::info!("Processing incoming GiftWrap event: {}", gift_wrap.id);

                    // Use extract_rumor to properly unwrap the gift wrap (like whitenoise does)
                    match nip59::extract_rumor(&self.keys, &gift_wrap).await {
                        Ok(unwrapped) => {
                            // Check if it's a welcome event
                            if unwrapped.rumor.kind == Kind::MlsWelcome {
                                log::info!(
                                    "Processing welcome event from {}",
                                    unwrapped.sender.to_bech32().unwrap_or_default()
                                );

                                // Process the welcome to join the group (follows mls_memory.rs pattern)
                                // First, process the welcome to add it to pending welcomes
                                match self.storage.process_welcome(
                                    &gift_wrap.id,    // Use the gift wrap event ID
                                    &unwrapped.rumor, // Pass the rumor directly
                                ) {
                                    Ok(welcome) => {
                                        log::info!("Processed welcome for group '{}', now accepting to join", welcome.group_name);

                                        // Get the group IDs from the welcome before accepting
                                        let group_id = welcome.mls_group_id.clone();
                                        let nostr_group_id = welcome.nostr_group_id;

                                        // Accept the welcome to actually join the group
                                        match self.storage.accept_welcome(&welcome) {
                                            Ok(()) => {
                                                log::info!(
                                                    "Successfully joined group: {group_id:?}"
                                                );

                                                // Subscribe to messages for this group
                                                let h_tag_value = hex::encode(nostr_group_id);
                                                let filter = Filter::new()
                                                    .kind(Kind::MlsGroupMessage)
                                                    .custom_tag(
                                                        SingleLetterTag::lowercase(Alphabet::H),
                                                        h_tag_value,
                                                    )
                                                    .limit(100);
                                                self.client.subscribe(filter, None).await?;
                                                log::info!("Subscribed to messages for new group");

                                                // Navigate to chat with the new group
                                                self.navigate_to(PageType::Chat(Some(group_id)))
                                                    .await?;
                                            }
                                            Err(e) => {
                                                log::error!("Failed to accept welcome: {e}");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        log::error!("Failed to process welcome: {e}");
                                    }
                                }
                            } else {
                                log::debug!(
                                    "Received non-welcome GiftWrap of kind {:?}",
                                    unwrapped.rumor.kind
                                );
                            }
                        }
                        Err(e) => {
                            log::debug!(
                                "Failed to extract rumor from gift wrap (might not be for us): {e}"
                            );
                        }
                    }
                }
            }
            AppEvent::RawMessagesReceived { events } => {
                // Process incoming MLS messages
                log::info!("Received {} MLS message events", events.len());

                for event in events {
                    match self.storage.process_message(&event) {
                        Ok(result) => {
                            match result {
                                MessageProcessingResult::ApplicationMessage(msg) => {
                                    log::info!(
                                        "Received message in group {:?}: {}",
                                        msg.mls_group_id,
                                        msg.content
                                    );

                                    // Convert group ID for comparison
                                    let group_id = GroupId::from_slice(msg.mls_group_id.as_slice());

                                    // Update UI if this is the current chat
                                    if let Page::Chat {
                                        group_id: current_group_id,
                                        messages,
                                        ..
                                    } = &mut self.current_page
                                    {
                                        if *current_group_id == group_id {
                                            // Add message to current chat
                                            messages.push(Message {
                                                content: msg.content.clone(),
                                                sender: msg.pubkey,
                                                timestamp: msg.created_at,
                                            });
                                            let _ = self.state_tx.send(self.current_page.clone());
                                        }
                                    }
                                }
                                MessageProcessingResult::Proposal(_) => {
                                    log::info!("Received proposal (not yet handled)");
                                }
                                MessageProcessingResult::Commit => {
                                    log::info!("Received commit");
                                }
                                MessageProcessingResult::Unprocessable => {
                                    log::debug!("Message was unprocessable (might be duplicate)");
                                }
                                MessageProcessingResult::ExternalJoinProposal => {
                                    log::info!("Received external join proposal (not yet handled)");
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to process message: {e}");
                        }
                    }
                }
            }
            AppEvent::KeyPackageReceived { event } => {
                // We don't need to store key packages - we fetch them when needed
                log::debug!(
                    "Received key package from {} via subscription",
                    event.pubkey.to_bech32().unwrap_or_default()
                );
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_keypress(&mut self, key_event: crossterm::event::KeyEvent) -> Result<()> {
        use crossterm::event::{KeyCode, KeyModifiers};

        // Extract necessary data first to avoid borrowing conflicts
        let key_code = key_event.code;
        let key_modifiers = key_event.modifiers;

        match (&self.current_page, key_code) {
            (
                Page::Onboarding {
                    input: _,
                    mode: _,
                    error: _,
                },
                KeyCode::Char(c),
            ) => {
                if let Page::Onboarding {
                    input,
                    mode: _,
                    error,
                } = &mut self.current_page
                {
                    input.push(c);
                    *error = None;
                    let _ = self.state_tx.send(self.current_page.clone());
                }
            }
            (Page::Onboarding { .. }, KeyCode::Backspace) => {
                if let Page::Onboarding { input, .. } = &mut self.current_page {
                    input.pop();
                    let _ = self.state_tx.send(self.current_page.clone());
                }
            }
            (Page::Onboarding { input, mode, .. }, KeyCode::Enter) => {
                let input_clone = input.clone();
                let mode_clone = mode.clone();
                self.handle_onboarding_enter(input_clone, mode_clone)
                    .await?;
            }

            (Page::Chat { input: _, .. }, KeyCode::Char(c)) => {
                if let Page::Chat { input, .. } = &mut self.current_page {
                    input.push(c);
                    let _ = self.state_tx.send(self.current_page.clone());
                }
            }
            (Page::Chat { input: _, .. }, KeyCode::Backspace) => {
                if let Page::Chat { input, .. } = &mut self.current_page {
                    input.pop();
                    let _ = self.state_tx.send(self.current_page.clone());
                }
            }
            (Page::Chat { input, .. }, KeyCode::Enter) => {
                if !input.is_empty() {
                    let input_content = input.clone();

                    // Clear input immediately
                    if let Page::Chat { input, .. } = &mut self.current_page {
                        input.clear();
                        let _ = self.state_tx.send(self.current_page.clone());
                    }

                    // Check if it's a command
                    if input_content.starts_with("/") {
                        self.process_command(input_content).await?;
                    } else {
                        // Regular message
                        self.send_event(AppEvent::SendMessage(input_content))?;
                    }
                }
            }
            (
                Page::Chat {
                    scroll_offset: _,
                    messages: _,
                    ..
                },
                KeyCode::Up,
            ) => {
                if let Page::Chat { scroll_offset, .. } = &mut self.current_page {
                    if *scroll_offset > 0 {
                        *scroll_offset -= 1;
                        let _ = self.state_tx.send(self.current_page.clone());
                    }
                }
            }
            (
                Page::Chat {
                    scroll_offset: _,
                    messages,
                    ..
                },
                KeyCode::Down,
            ) => {
                let messages_len = messages.len();
                if let Page::Chat { scroll_offset, .. } = &mut self.current_page {
                    if *scroll_offset + 20 < messages_len {
                        *scroll_offset += 1;
                        let _ = self.state_tx.send(self.current_page.clone());
                    }
                }
            }

            (_, KeyCode::Esc) => {
                if self.can_navigate_back() {
                    self.navigate_back().await?;
                } else {
                    self.navigate_to(PageType::Chat(None)).await?;
                }
            }
            (_, KeyCode::F(1)) => {
                self.navigate_to(PageType::Help).await?;
            }
            (_, KeyCode::Char('s')) if key_modifiers.contains(KeyModifiers::CONTROL) => {
                // Settings page removed
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_onboarding_enter(
        &mut self,
        input: String,
        mode: crate::ui_state::OnboardingMode,
    ) -> Result<()> {
        use crate::ui_state::OnboardingMode;

        match mode {
            OnboardingMode::Choose => match input.as_str() {
                "1" => {
                    // Skip GenerateNew and go straight to EnterDisplayName
                    let new_page = Page::Onboarding {
                        input: String::new(),
                        mode: OnboardingMode::EnterDisplayName,
                        error: None,
                    };
                    self.current_page = new_page.clone();
                    let _ = self.state_tx.send(new_page);
                }
                "2" => {
                    let new_page = Page::Onboarding {
                        input: String::new(),
                        mode: OnboardingMode::ImportExisting,
                        error: None,
                    };
                    self.current_page = new_page.clone();
                    let _ = self.state_tx.send(new_page);
                }
                _ => {
                    let mut new_page = self.current_page.clone();
                    if let Page::Onboarding { error, input, .. } = &mut new_page {
                        *error = Some("Invalid choice. Enter 1 or 2.".to_string());
                        input.clear();
                    }
                    self.current_page = new_page.clone();
                    let _ = self.state_tx.send(new_page);
                }
            },
            OnboardingMode::EnterDisplayName => {
                if !input.trim().is_empty() {
                    let new_page = Page::Onboarding {
                        input: String::new(),
                        mode: OnboardingMode::CreatePassword,
                        error: None,
                    };
                    self.current_page = new_page.clone();
                    let _ = self.state_tx.send(new_page);
                }
            }
            OnboardingMode::CreatePassword | OnboardingMode::EnterPassword => {
                if input.len() >= 8 {
                    if mode == OnboardingMode::EnterPassword {
                        // Validate password by trying to load encrypted keys
                        match self.key_storage.load_encrypted(&input) {
                            Ok(loaded_keys) => {
                                // Password is correct, update keys
                                self.keys = loaded_keys;
                                self.navigate_to(PageType::Initializing).await?;

                                // Initialize MLS and publish key package
                                self.publish_key_package().await?;

                                self.navigate_to(PageType::Chat(None)).await?;
                            }
                            Err(_) => {
                                // Wrong password
                                let mut new_page = self.current_page.clone();
                                if let Page::Onboarding { error, input, .. } = &mut new_page {
                                    *error =
                                        Some("Invalid password. Please try again.".to_string());
                                    input.clear();
                                }
                                self.current_page = new_page.clone();
                                let _ = self.state_tx.send(new_page);
                            }
                        }
                    } else {
                        // CreatePassword mode - save and initialize MLS
                        self.navigate_to(PageType::Initializing).await?;

                        // Initialize MLS and publish key package
                        self.publish_key_package().await?;

                        self.navigate_to(PageType::Chat(None)).await?;
                    }
                } else {
                    let mut new_page = self.current_page.clone();
                    if let Page::Onboarding { error, input, .. } = &mut new_page {
                        *error = Some("Password must be at least 8 characters.".to_string());
                        input.clear();
                    }
                    self.current_page = new_page.clone();
                    let _ = self.state_tx.send(new_page);
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn get_state_receiver(&self) -> watch::Receiver<Page> {
        self.state_rx.clone()
    }

    pub fn send_event(&self, event: AppEvent) -> Result<()> {
        self.event_tx.send(event)?;
        Ok(())
    }

    async fn load_group_summaries(&self) -> Result<Vec<GroupSummary>> {
        let groups = self.storage.get_groups()?;
        let mut summaries = Vec::new();

        for group in groups {
            let id = group.mls_group_id.clone();
            let messages = self.storage.get_messages(&id)?;
            let last_message = messages.last().map(|m| Message {
                content: m.content.clone(),
                sender: m.pubkey,
                timestamp: Timestamp::now(),
            });

            summaries.push(GroupSummary {
                id,
                name: group.name.clone(),
                member_count: 0,
                last_message,
                unread_count: 0,
            });
        }

        Ok(summaries)
    }

    async fn load_chat_messages(&self, group_id: &GroupId, limit: usize) -> Result<Vec<Message>> {
        let stored_messages = self.storage.get_messages(group_id)?;
        let messages: Vec<Message> = stored_messages
            .into_iter()
            .rev()
            .take(limit)
            .rev()
            .map(|m| Message {
                content: m.content,
                sender: m.pubkey,
                timestamp: Timestamp::now(),
            })
            .collect();
        Ok(messages)
    }

    async fn load_group_members(&self, group_id: &GroupId) -> Result<Vec<crate::ui_state::Member>> {
        let _group = self
            .storage
            .get_group(group_id)?
            .ok_or_else(|| anyhow::anyhow!("Group not found"))?;

        let _profiles = self.profiles.lock().await;
        let members = vec![];

        Ok(members)
    }

    async fn process_command(&mut self, command: String) -> Result<()> {
        log::info!("Processing command: {command}");
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(());
        }

        match parts[0] {
            "/npub" | "/n" => {
                // Copy npub to clipboard
                let npub = self.keys.public_key().to_bech32()?;

                match clipboard::ClipboardContext::new() {
                    Ok(mut ctx) => match ctx.set_contents(npub.clone()) {
                        Ok(_) => {
                            self.flash = Some((
                                format!("Copied npub to clipboard: {npub}"),
                                std::time::Instant::now() + std::time::Duration::from_secs(5),
                            ));
                        }
                        Err(e) => {
                            self.flash = Some((
                                format!("Failed to copy to clipboard: {e}. Your npub: {npub}"),
                                std::time::Instant::now() + std::time::Duration::from_secs(10),
                            ));
                        }
                    },
                    Err(e) => {
                        self.flash = Some((
                            format!("Failed to access clipboard: {e}. Your npub: {npub}"),
                            std::time::Instant::now() + std::time::Duration::from_secs(10),
                        ));
                    }
                }
            }
            "/dm" | "/d" => {
                if parts.len() < 2 {
                    self.flash = Some((
                        "Usage: /dm <npub>".to_string(),
                        std::time::Instant::now() + std::time::Duration::from_secs(5),
                    ));
                    return Ok(());
                }

                // Parse the npub
                let npub_str = parts[1];
                match PublicKey::from_bech32(npub_str) {
                    Ok(pubkey) => {
                        // Fetch their key package and create group
                        self.create_dm_with(pubkey).await?;
                    }
                    Err(e) => {
                        self.flash = Some((
                            format!("Invalid npub: {e}"),
                            std::time::Instant::now() + std::time::Duration::from_secs(5),
                        ));
                    }
                }
            }
            _ => {
                self.flash = Some((
                    format!("Unknown command: {}", parts[0]),
                    std::time::Instant::now() + std::time::Duration::from_secs(5),
                ));
            }
        }

        Ok(())
    }

    async fn publish_key_package(&mut self) -> Result<()> {
        let relays: Result<Vec<RelayUrl>, _> = get_default_relays()
            .iter()
            .map(|&url| RelayUrl::parse(url))
            .collect();
        let relays = relays?;
        let (key_package_content, tags) = self
            .storage
            .create_key_package_for_event(&self.keys.public_key(), relays)?;

        let event = EventBuilder::new(Kind::MlsKeyPackage, key_package_content)
            .tags(tags)
            .build(self.keys.public_key())
            .sign(&self.keys)
            .await?;

        let send_result = self.client.send_event(&event).await?;
        log::info!(
            "Key package published with event ID: {} to {} relays",
            event
                .id
                .to_bech32()
                .unwrap_or_else(|_| "unknown".to_string()),
            send_result.success.len()
        );

        if send_result.success.is_empty() {
            log::error!("Failed to publish key package to any relays!");
            log::error!("Failed relays: {:?}", send_result.failed);
        }

        // Also subscribe to GiftWrap events for welcomes
        let filter = Filter::new()
            .kind(Kind::GiftWrap)
            .pubkey(self.keys.public_key());
        self.client.subscribe(filter, None).await?;

        log::info!("Key package published successfully");
        Ok(())
    }

    async fn create_dm_with(&mut self, other_pubkey: PublicKey) -> Result<()> {
        log::info!(
            "=== Starting create_dm_with for {} ===",
            other_pubkey.to_bech32()?
        );

        // Check if we're already in a group with this person
        let already_in_group = match &self.current_page {
            Page::Chat { groups, .. } => {
                groups.iter().any(|_group_summary| {
                    // TODO: check if other_pubkey is in this group
                    // For now, just assume no duplicates
                    false
                })
            }
            _ => false,
        };

        if already_in_group {
            let npub_str = other_pubkey.to_bech32()?;
            self.flash = Some((
                format!("Already in a group with {npub_str}"),
                std::time::Instant::now() + std::time::Duration::from_secs(5),
            ));
            return Ok(());
        }

        // Fetch the key package from relays
        let filter = Filter::new()
            .kind(Kind::MlsKeyPackage)
            .author(other_pubkey)
            .limit(1);

        log::info!(
            "Fetching key package for {} with filter: kind={:?}, author={}",
            other_pubkey.to_bech32()?,
            Kind::MlsKeyPackage,
            other_pubkey.to_hex()
        );

        // Ensure we're connected - just try to connect, it's idempotent
        self.client.connect().await;

        let events = self
            .client
            .fetch_events(filter.clone(), Duration::from_secs(10))
            .await?;

        log::info!("Fetch returned {} events", events.len());

        let key_package = match events.into_iter().next() {
            Some(kp) => {
                log::info!("Found key package for {}", other_pubkey.to_bech32()?);
                kp
            }
            None => {
                self.flash = Some((
                    format!(
                        "No key package found for {} (have they completed onboarding?)",
                        other_pubkey.to_bech32()?
                    ),
                    std::time::Instant::now() + std::time::Duration::from_secs(5),
                ));
                log::error!("No key package found for {}", other_pubkey.to_bech32()?);
                return Ok(());
            }
        };

        // Create group with them (copied from groups.rs create_group_with_member logic)
        // Get relay URLs from default relays
        let relay_urls: Result<Vec<RelayUrl>, _> = get_default_relays()
            .iter()
            .map(|&url| RelayUrl::parse(url))
            .collect();
        let relay_urls = relay_urls?;

        let config = NostrGroupConfigData::new(
            format!("DM with {}", other_pubkey.to_bech32()?),
            "Direct message".to_string(),
            None,
            None,
            None,
            relay_urls,
            vec![self.keys.public_key(), other_pubkey],
        );

        match self
            .storage
            .create_group(&self.keys.public_key(), vec![key_package.clone()], config)
        {
            Ok(group_result) => {
                let group_id = GroupId::from_slice(group_result.group.mls_group_id.as_slice());

                // Subscribe to messages for this group
                let h_tag_value = hex::encode(group_result.group.nostr_group_id);
                let filter = Filter::new()
                    .kind(Kind::MlsGroupMessage)
                    .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value)
                    .limit(100);
                self.client.subscribe(filter, None).await?;

                // Send them the welcome
                if let Some(welcome_rumor) = group_result.welcome_rumors.first() {
                    log::info!("Sending welcome to {}", other_pubkey.to_bech32()?);
                    let gift_wrapped = EventBuilder::gift_wrap(
                        &self.keys,
                        &other_pubkey,
                        welcome_rumor.clone(),
                        None,
                    )
                    .await?;
                    if let Err(e) = self.client.send_event(&gift_wrapped).await {
                        log::error!("Failed to send welcome: {e}");
                    }
                }

                // Update UI to show new group
                self.flash = Some((
                    format!("Created DM with {}", other_pubkey.to_bech32()?),
                    std::time::Instant::now() + std::time::Duration::from_secs(5),
                ));

                // Navigate to the chat
                self.navigate_to(PageType::Chat(Some(group_id))).await?;
            }
            Err(e) => {
                self.flash = Some((
                    format!("Failed to create group: {e}"),
                    std::time::Instant::now() + std::time::Duration::from_secs(5),
                ));
                log::error!("Failed to create group: {e}");
            }
        }

        Ok(())
    }

    pub async fn load_older_messages(&mut self, limit: usize) -> Result<()> {
        if let Page::Chat {
            group_id,
            messages,
            scroll_offset,
            ..
        } = &mut self.current_page
        {
            let older_messages = self.storage.get_messages(group_id)?;
            let skip = messages.len();
            let additional: Vec<Message> = older_messages
                .into_iter()
                .rev()
                .skip(skip)
                .take(limit)
                .rev()
                .map(|m| Message {
                    content: m.content,
                    sender: m.pubkey,
                    timestamp: Timestamp::now(),
                })
                .collect();

            if !additional.is_empty() {
                let mut new_messages = additional;
                new_messages.append(messages);
                *messages = new_messages;
                *scroll_offset += limit;
                let _ = self.state_tx.send(self.current_page.clone());
            }
        }
        Ok(())
    }

    pub async fn refresh_current_page(&mut self) -> Result<()> {
        let page_type = self.current_page.page_type();
        let refreshed = self.load_page_data(page_type).await?;

        if let (
            Page::Chat {
                input: old_input,
                scroll_offset: old_scroll,
                ..
            },
            Page::Chat {
                input: _new_input,
                scroll_offset: _new_scroll,
                ..
            },
        ) = (&self.current_page, &refreshed)
        {
            let mut refreshed = refreshed;
            if let Page::Chat {
                input,
                scroll_offset,
                ..
            } = &mut refreshed
            {
                *input = old_input.clone();
                *scroll_offset = *old_scroll;
            }
            self.current_page = refreshed.clone();
            let _ = self.state_tx.send(refreshed);
        } else {
            self.current_page = refreshed.clone();
            let _ = self.state_tx.send(refreshed);
        }

        Ok(())
    }
}
