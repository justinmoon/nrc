use crate::actions::{Action, OnboardingChoice};
use crate::notification_handler::spawn_notification_handler;
use crate::{AppEvent, AppState, Message, Nrc, OnboardingMode};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nostr_sdk::prelude::*;
use nrc_mls_storage::groups::types as group_types;
use openmls::group::GroupId;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};

/// UI State containing all data needed for rendering
#[derive(Clone, Debug)]
pub struct UIState {
    pub app_state: AppState,
    pub messages: HashMap<GroupId, Vec<Message>>,
    pub groups: HashMap<GroupId, group_types::Group>,
    pub input: String,
    pub selected_group_index: Option<usize>,
    pub last_error: Option<String>,
    pub flash_message: Option<String>,
    pub show_help: bool,
    pub profiles: HashMap<PublicKey, Metadata>,
    pub current_pubkey: PublicKey,
}

pub struct EventedNrc {
    // Single unified state channel containing all UI data
    pub ui_state: watch::Receiver<UIState>,

    // Send actions to the event loop
    action_tx: mpsc::UnboundedSender<Action>,
}

impl EventedNrc {
    #[allow(clippy::await_holding_lock)]
    pub async fn new(datadir: &Path) -> Result<Self> {
        // Create the underlying Nrc
        let nrc = Nrc::new(datadir).await?;

        // Create initial UI state
        let initial_ui_state = UIState {
            app_state: nrc.state.clone(),
            messages: nrc.messages.clone(),
            groups: nrc.groups.clone(),
            input: nrc.input.clone(),
            selected_group_index: nrc.selected_group_index,
            last_error: nrc.last_error.clone(),
            flash_message: nrc.flash_message.clone(),
            show_help: nrc.show_help,
            profiles: nrc.profiles.clone(),
            current_pubkey: nrc.keys.public_key(),
        };

        // Create channels
        let (ui_state_tx, ui_state_rx) = watch::channel(initial_ui_state);
        let (action_tx, mut action_rx) = mpsc::unbounded_channel();

        // Create channel for AppEvents from notification handler
        let (app_event_tx, mut app_event_rx) = mpsc::unbounded_channel();

        // Wrap Nrc in Arc<Mutex> for thread safety
        let nrc = Arc::new(Mutex::new(nrc));
        let nrc_clone = nrc.clone();
        let ui_state_tx_clone = ui_state_tx.clone();

        // Spawn notification handler for real-time events
        {
            let client = nrc.lock().unwrap().client.clone();
            spawn_notification_handler(client, app_event_tx);
        }

        // Spawn bridge to convert AppEvents to Actions
        let action_tx_bridge = action_tx.clone();
        tokio::spawn(async move {
            while let Some(app_event) = app_event_rx.recv().await {
                log::debug!("🌉 BRIDGE: Converting AppEvent to Action: {app_event:?}");
                let action = match app_event {
                    AppEvent::RawWelcomesReceived { events } => {
                        Some(Action::RawWelcomesReceived(events))
                    }
                    AppEvent::RawMessagesReceived { events } => {
                        Some(Action::RawMessagesReceived(events))
                    }
                    AppEvent::NetworkError { error } => {
                        log::error!("🌉 BRIDGE: Network error from notification handler: {error}");
                        None // Could add an action for this if needed
                    }
                    _ => {
                        log::debug!("🌉 BRIDGE: Unhandled AppEvent: {app_event:?}");
                        None
                    }
                };

                if let Some(action) = action {
                    let _ = action_tx_bridge.send(action);
                }
            }
        });

        // Spawn the event processor in a dedicated thread with its own runtime
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                while let Some(action) = action_rx.recv().await {
                    log::debug!("⚡ PROCESS: {action:?}");

                    // Process action with locked Nrc
                    // Note: This is safe in our single-threaded event loop design
                    {
                        #[allow(clippy::await_holding_lock)]
                        let mut nrc_guard = nrc_clone.lock().unwrap();
                        if let Err(e) =
                            process_action_sync(&mut nrc_guard, action, &ui_state_tx_clone).await
                        {
                            log::error!("Action processing error: {e}");
                        }
                    }
                }
            });
        });

        Ok(EventedNrc {
            ui_state: ui_state_rx,
            action_tx,
        })
    }

    /// Emit an action to be processed
    pub fn emit(&self, action: Action) {
        log::debug!("🚀 EMIT: {action:?}");
        let _ = self.action_tx.send(action);
    }

    /// Helper to get current app state
    pub fn current_state(&self) -> AppState {
        self.ui_state.borrow().app_state.clone()
    }

    /// Get current npub (derives from current keys in UI state)
    pub fn get_npub(&self) -> String {
        self.ui_state
            .borrow()
            .current_pubkey
            .to_bech32()
            .unwrap_or_else(|_| "unknown".to_string())
    }

    /// Get messages for a specific group
    pub fn get_messages(&self, group_id: &GroupId) -> Vec<Message> {
        self.ui_state
            .borrow()
            .messages
            .get(group_id)
            .cloned()
            .unwrap_or_default()
    }
}

// Removed run_event_loop as processing is now inline in EventedNrc::new()

/// Process a single action and update state (synchronous version for use with Mutex)
async fn process_action_sync(
    nrc: &mut Nrc,
    action: Action,
    ui_state_tx: &watch::Sender<UIState>,
) -> Result<()> {
    match action {
        Action::SetInput(input) => {
            nrc.input = input;
        }
        Action::Backspace => {
            nrc.input.pop();
        }
        Action::ClearInput => {
            nrc.input.clear();
        }
        Action::NextGroup => {
            nrc.next_group();
        }
        Action::PrevGroup => {
            nrc.prev_group();
        }
        Action::ScrollDown => {
            nrc.scroll_offset = nrc.scroll_offset.saturating_sub(1);
        }
        Action::ScrollUp => {
            nrc.scroll_offset = nrc.scroll_offset.saturating_add(1);
        }
        Action::OnboardingChoice(choice) => {
            match choice {
                OnboardingChoice::GenerateNew => {
                    nrc.keys = Keys::generate();
                    nrc.state = AppState::Onboarding {
                        input: String::new(),
                        mode: OnboardingMode::EnterDisplayName,
                    };
                }
                OnboardingChoice::ImportExisting => {
                    nrc.state = AppState::Onboarding {
                        input: String::new(),
                        mode: OnboardingMode::ImportExisting,
                    };
                }
                OnboardingChoice::Continue => {
                    // Handle continue in onboarding flow
                }
            }
        }
        Action::SetDisplayName(name) => {
            nrc.onboarding_data.display_name = Some(name);
            // Transition to password creation
            nrc.state = AppState::Onboarding {
                input: String::new(),
                mode: OnboardingMode::CreatePassword,
            };
        }
        Action::SetPassword(password) => {
            // Check if we have a display name from onboarding
            if let Some(display_name) = nrc.onboarding_data.display_name.take() {
                // New user flow - save keys and initialize with display name
                log::debug!("🔑 New user: initializing with display name and password...");
                match nrc
                    .initialize_with_display_name_and_password(display_name, password)
                    .await
                {
                    Ok(_) => {
                        log::debug!("🔑 New user initialization successful");
                        nrc.state = AppState::Ready {
                            groups: nrc.groups.keys().cloned().collect(),
                            key_package_published: true,
                        };
                        log::debug!("🔑 New state after init: {:?}", nrc.state);
                    }
                    Err(e) => {
                        log::error!("🔑 New user initialization failed: {e}");
                        nrc.last_error = Some(e.to_string());
                    }
                }
            } else {
                // Returning user flow - load keys with password
                log::debug!("🔑 Returning user: loading keys with password...");
                match nrc.initialize_with_password(password).await {
                    Ok(_) => {
                        log::debug!("🔑 Password initialization successful");
                        log::debug!("🔑 New state after init: {:?}", nrc.state);
                    }
                    Err(e) => {
                        log::error!("🔑 Password initialization failed: {e}");
                        nrc.last_error = Some(e.to_string());
                    }
                }
            }
        }
        Action::SetNsec(nsec_str) => {
            if let Ok(secret_key) = SecretKey::from_str(&nsec_str) {
                nrc.keys = Keys::new(secret_key);
                // Transition to password creation after importing
                nrc.state = AppState::Onboarding {
                    input: String::new(),
                    mode: OnboardingMode::CreatePassword,
                };
            } else {
                nrc.last_error = Some("Invalid nsec format".to_string());
            }
        }
        Action::SendMessage(content) => {
            if let Some(group_id) = nrc.get_selected_group() {
                nrc.send_message(group_id, content).await?;
            }
        }
        Action::JoinGroup(npub) => {
            // Parse the npub to get the public key
            if let Ok(pubkey) = PublicKey::from_str(&npub) {
                // Fetch their key package
                match nrc.fetch_key_package(&pubkey).await {
                    Ok(key_package) => {
                        // Create a group with them
                        match nrc.create_group_with_member(key_package).await {
                            Ok(group_id) => {
                                log::debug!(
                                    "Successfully created group with member, group_id: {group_id:?}"
                                );
                                log::debug!("Current groups count: {}", nrc.groups.len());
                                log::debug!("Current state: {:?}", nrc.state);
                            }
                            Err(e) => {
                                log::error!("Failed to create group: {e}");
                                nrc.last_error = Some(format!("Failed to create group: {e}"));
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to fetch key package: {e}");
                        nrc.last_error = Some(format!("Failed to join group: {e}"));
                    }
                }
            } else {
                nrc.last_error = Some("Invalid npub format".to_string());
            }
        }
        Action::FetchMessages => {
            log::debug!("📨 FETCH_MESSAGES: Starting fetch");
            match nrc.fetch_and_process_messages().await {
                Ok(_) => log::debug!("📨 FETCH_MESSAGES: Success"),
                Err(e) => log::error!("📨 FETCH_MESSAGES: Error: {e}"),
            }
        }
        Action::FetchWelcomes => {
            log::debug!("📥 FETCH_WELCOMES: Starting fetch");
            match nrc.fetch_and_process_welcomes().await {
                Ok(_) => log::debug!("📥 FETCH_WELCOMES: Success"),
                Err(e) => log::error!("📥 FETCH_WELCOMES: Error: {e}"),
            }
        }
        Action::NostrEventReceived(event) => {
            // Process incoming nostr events
            if event.kind == Kind::GiftWrap {
                let _ = nrc.process_welcome_event(event).await;
            } else if event.kind == Kind::from(445u16) {
                let _ = nrc.process_message_event(event).await;
            }
        }
        Action::RawWelcomesReceived(events) => {
            log::debug!("📥 RAW_WELCOMES: Processing {} events", events.len());
            for event in events {
                if let Err(e) = nrc.process_welcome_event(event).await {
                    log::error!("📥 RAW_WELCOMES: Failed to process welcome: {e}");
                }
            }
        }
        Action::RawMessagesReceived(events) => {
            log::debug!("📨 RAW_MESSAGES: Processing {} events", events.len());
            for event in events {
                if let Err(e) = nrc.process_message_event(event).await {
                    log::error!("📨 RAW_MESSAGES: Failed to process message: {e}");
                }
            }
        }
        _ => {
            log::debug!("Unhandled action: {action:?}");
        }
    }

    // Update UI state with all current data
    let ui_state = UIState {
        app_state: nrc.state.clone(),
        messages: nrc.messages.clone(),
        groups: nrc.groups.clone(),
        input: nrc.input.clone(),
        selected_group_index: nrc.selected_group_index,
        last_error: nrc.last_error.clone(),
        flash_message: nrc.flash_message.clone(),
        show_help: nrc.show_help,
        profiles: nrc.profiles.clone(),
        current_pubkey: nrc.keys.public_key(),
    };
    let _ = ui_state_tx.send(ui_state);
    log::debug!("📡 STATE UPDATE: {:?}", nrc.state);

    Ok(())
}

/// Convert keyboard event to action
pub fn convert_key_to_action(key: KeyEvent, state: &AppState) -> Option<Action> {
    match state {
        AppState::Onboarding { input, mode } => {
            match mode {
                OnboardingMode::Choose => match key.code {
                    KeyCode::Char('1') => {
                        Some(Action::OnboardingChoice(OnboardingChoice::GenerateNew))
                    }
                    KeyCode::Char('2') => {
                        Some(Action::OnboardingChoice(OnboardingChoice::ImportExisting))
                    }
                    _ => None,
                },
                OnboardingMode::EnterDisplayName => match key.code {
                    KeyCode::Enter if !input.is_empty() => {
                        Some(Action::SetDisplayName(input.clone()))
                    }
                    KeyCode::Char(c) => {
                        let mut new_input = input.clone();
                        new_input.push(c);
                        Some(Action::SetInput(new_input))
                    }
                    KeyCode::Backspace => Some(Action::Backspace),
                    _ => None,
                },
                OnboardingMode::ImportExisting => match key.code {
                    KeyCode::Enter if !input.is_empty() => Some(Action::SetNsec(input.clone())),
                    KeyCode::Char(c) => {
                        let mut new_input = input.clone();
                        new_input.push(c);
                        Some(Action::SetInput(new_input))
                    }
                    KeyCode::Backspace => Some(Action::Backspace),
                    _ => None,
                },
                OnboardingMode::CreatePassword | OnboardingMode::EnterPassword => match key.code {
                    KeyCode::Enter if !input.is_empty() => Some(Action::SetPassword(input.clone())),
                    KeyCode::Char(c) => {
                        let mut new_input = input.clone();
                        new_input.push(c);
                        Some(Action::SetInput(new_input))
                    }
                    KeyCode::Backspace => Some(Action::Backspace),
                    _ => None,
                },
                OnboardingMode::GenerateNew => {
                    // GenerateNew mode is just a transition state, no input handling needed
                    None
                }
            }
        }
        AppState::Ready { .. } => {
            // Navigation keys
            if key.modifiers == KeyModifiers::NONE {
                match key.code {
                    KeyCode::Up => Some(Action::PrevGroup),
                    KeyCode::Down => Some(Action::NextGroup),
                    KeyCode::PageUp => Some(Action::ScrollUp),
                    KeyCode::PageDown => Some(Action::ScrollDown),
                    KeyCode::Char(c) => {
                        // In ready state, typing starts a new input
                        Some(Action::SetInput(c.to_string()))
                    }
                    KeyCode::Backspace => Some(Action::Backspace),
                    KeyCode::Enter => {
                        // Process current input as a command
                        Some(Action::SendMessage(String::new()))
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => None,
    }
}
