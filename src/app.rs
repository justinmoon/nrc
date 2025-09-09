use anyhow::Result;
use nostr_sdk::prelude::*;
use nrc_mls::NostrMls;
use nrc_mls_sqlite_storage::NostrMlsSqliteStorage;
use openmls::group::GroupId;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch, Mutex};

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
            PageType::GroupList => {
                let groups = self.load_group_summaries().await?;
                Ok(Page::GroupList {
                    groups,
                    selected_index: 0,
                    filter: None,
                })
            }
            PageType::Chat(group_id) => {
                let group_info = self
                    .storage
                    .get_group(&group_id)?
                    .ok_or_else(|| anyhow::anyhow!("Group not found"))?;
                let messages = self.load_chat_messages(&group_id, 100).await?;
                let members = self.load_group_members(&group_id).await?;

                Ok(Page::Chat {
                    group_id,
                    group_info: Box::new(group_info),
                    messages,
                    members,
                    input: String::new(),
                    scroll_offset: 0,
                    typing_members: vec![],
                })
            }
            PageType::CreateGroup => {
                let available_members = self.load_available_contacts().await?;
                Ok(Page::CreateGroup {
                    name_input: String::new(),
                    member_search: String::new(),
                    selected_members: vec![],
                    available_members,
                })
            }
            PageType::Settings => {
                let current_settings = self.load_current_settings().await?;
                Ok(Page::Settings {
                    current_settings: current_settings.clone(),
                    edited_settings: current_settings,
                    selected_field: crate::ui_state::SettingField::DisplayName,
                })
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
                    group_id, input, ..
                } = &mut self.current_page
                {
                    if !content.is_empty() {
                        let _ = self
                            .command_tx
                            .send(NetworkCommand::SendMessage {
                                group_id: group_id.clone(),
                                content,
                            })
                            .await;
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
                self.navigate_to(PageType::GroupList).await?;
            }
            AppEvent::JoinGroup(group_id) => {
                self.navigate_to(PageType::Chat(group_id)).await?;
            }
            AppEvent::FlashMessage(msg, duration) => {
                self.flash = Some((msg, Instant::now() + duration));
            }
            AppEvent::ClearFlash => {
                self.flash = None;
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

            (
                Page::GroupList {
                    selected_index: _,
                    groups: _,
                    ..
                },
                KeyCode::Up,
            ) => {
                if let Page::GroupList { selected_index, .. } = &mut self.current_page {
                    if *selected_index > 0 {
                        *selected_index -= 1;
                        let _ = self.state_tx.send(self.current_page.clone());
                    }
                }
            }
            (
                Page::GroupList {
                    selected_index: _,
                    groups,
                    ..
                },
                KeyCode::Down,
            ) => {
                let groups_len = groups.len();
                if let Page::GroupList { selected_index, .. } = &mut self.current_page {
                    if *selected_index + 1 < groups_len {
                        *selected_index += 1;
                        let _ = self.state_tx.send(self.current_page.clone());
                    }
                }
            }
            (
                Page::GroupList {
                    selected_index,
                    groups,
                    ..
                },
                KeyCode::Enter,
            ) => {
                if let Some(group) = groups.get(*selected_index) {
                    let group_id = group.id.clone();
                    self.navigate_to(PageType::Chat(group_id)).await?;
                }
            }
            (Page::GroupList { .. }, KeyCode::Char('n'))
                if key_modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.navigate_to(PageType::CreateGroup).await?;
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
                    self.send_event(AppEvent::SendMessage(input_content))?;
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
                } else if !matches!(self.current_page, Page::GroupList { .. }) {
                    self.navigate_to(PageType::GroupList).await?;
                }
            }
            (_, KeyCode::F(1)) => {
                self.navigate_to(PageType::Help).await?;
            }
            (_, KeyCode::Char('s')) if key_modifiers.contains(KeyModifiers::CONTROL) => {
                self.navigate_to(PageType::Settings).await?;
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
                    let new_page = Page::Onboarding {
                        input: String::new(),
                        mode: OnboardingMode::GenerateNew,
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
            OnboardingMode::GenerateNew => {
                // Immediately transition to EnterDisplayName
                let new_page = Page::Onboarding {
                    input: String::new(),
                    mode: OnboardingMode::EnterDisplayName,
                    error: None,
                };
                self.current_page = new_page.clone();
                let _ = self.state_tx.send(new_page);
            }
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
                    self.navigate_to(PageType::Initializing).await?;

                    if mode == OnboardingMode::EnterPassword {
                        // TODO: implement key loading validation
                        // For now, just proceed with initialization
                    }

                    tokio::time::sleep(Duration::from_secs(2)).await;
                    self.navigate_to(PageType::GroupList).await?;
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

    async fn load_available_contacts(&self) -> Result<Vec<crate::ui_state::Contact>> {
        let profiles = self.profiles.lock().await;
        let contacts = profiles
            .iter()
            .map(|(pk, metadata)| crate::ui_state::Contact {
                public_key: *pk,
                display_name: metadata.display_name.clone(),
                metadata: Some(metadata.clone()),
            })
            .collect();
        Ok(contacts)
    }

    async fn load_current_settings(&self) -> Result<crate::ui_state::UserSettings> {
        let profiles = self.profiles.lock().await;
        let my_metadata = profiles.get(&self.keys.public_key());
        let display_name = my_metadata
            .and_then(|m| m.display_name.clone())
            .unwrap_or_else(|| "Anonymous".to_string());

        let relays = self
            .client
            .relays()
            .await
            .keys()
            .map(|url| url.to_string())
            .collect();

        Ok(crate::ui_state::UserSettings {
            display_name,
            relays,
            notification_enabled: true,
        })
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
