use crate::actions::{Action, OnboardingChoice};
use crate::{AppState, Message, Nrc, OnboardingMode};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nostr_sdk::prelude::*;
use openmls::group::GroupId;
use nrc_mls_storage::groups::types as group_types;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
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
}

pub struct EventedNrc {
    // Single unified state channel containing all UI data
    pub ui_state: watch::Receiver<UIState>,
    pub npub: String,
    
    // Send actions to the event loop
    action_tx: mpsc::UnboundedSender<Action>,
}

impl EventedNrc {
    pub async fn new(datadir: &Path) -> Result<Self> {
        // Create the underlying Nrc
        let nrc = Nrc::new(datadir).await?;
        let npub = nrc.keys.public_key().to_bech32()?;
        
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
        };
        
        // Create channels
        let (ui_state_tx, ui_state_rx) = watch::channel(initial_ui_state);
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        
        // Store the event processor components  
        let evented = EventedNrc {
            ui_state: ui_state_rx,
            npub,
            action_tx,
        };
        
        // Start the background processor (non-Send, so we use a different approach)
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                run_event_loop(nrc, action_rx, ui_state_tx).await;
            });
        });
        
        Ok(evented)
    }
    
    /// Emit an action to be processed
    pub fn emit(&self, action: Action) {
        log::debug!("🚀 EMIT: {:?}", action);
        let _ = self.action_tx.send(action);
    }
    
    /// Helper to get current app state
    pub fn current_state(&self) -> AppState {
        self.ui_state.borrow().app_state.clone()
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

/// Run the event processing loop
async fn run_event_loop(
    mut nrc: Nrc,
    mut action_rx: mpsc::UnboundedReceiver<Action>,
    ui_state_tx: watch::Sender<UIState>,
) {
    const MAX_EVENTS_PER_BATCH: usize = 100;
    let mut batch_count;
    
    loop {
        batch_count = 0;
        
        // Process up to MAX_EVENTS_PER_BATCH actions
        while batch_count < MAX_EVENTS_PER_BATCH {
            match action_rx.try_recv() {
                Ok(action) => {
                    log::debug!("⚡ PROCESS: {:?}", action);
                    if let Err(e) = process_action(&mut nrc, action, &ui_state_tx).await {
                        log::error!("Action processing error: {}", e);
                    }
                    batch_count += 1;
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    // No more actions, break inner loop
                    break;
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    log::info!("Action channel closed, stopping event loop");
                    return;
                }
            }
        }
        
        if batch_count >= MAX_EVENTS_PER_BATCH {
            log::warn!("Processed maximum batch size of {} events", MAX_EVENTS_PER_BATCH);
        }
        
        // Wait for next action if none available
        if batch_count == 0 {
            match action_rx.recv().await {
                Some(action) => {
                    log::debug!("⚡ PROCESS: {:?}", action);
                    if let Err(e) = process_action(&mut nrc, action, &ui_state_tx).await {
                        log::error!("Action processing error: {}", e);
                    }
                }
                None => {
                    log::info!("Action channel closed, stopping event loop");
                    return;
                }
            }
        }
        
        // Yield to other tasks
        tokio::task::yield_now().await;
    }
}

/// Process a single action and update state
async fn process_action(
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
            // Initialize with password
            if let Err(e) = nrc.initialize_with_password(password).await {
                nrc.last_error = Some(e.to_string());
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
                        let _ = nrc.create_group_with_member(key_package).await;
                    }
                    Err(e) => {
                        nrc.last_error = Some(format!("Failed to join group: {}", e));
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
                Err(e) => log::error!("📨 FETCH_MESSAGES: Error: {}", e),
            }
        }
        Action::FetchWelcomes => {
            log::debug!("📥 FETCH_WELCOMES: Starting fetch");
            match nrc.fetch_and_process_welcomes().await {
                Ok(_) => log::debug!("📥 FETCH_WELCOMES: Success"),
                Err(e) => log::error!("📥 FETCH_WELCOMES: Error: {}", e),
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
        _ => {
            log::debug!("Unhandled action: {:?}", action);
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
                OnboardingMode::Choose => {
                    match key.code {
                        KeyCode::Char('1') => Some(Action::OnboardingChoice(OnboardingChoice::GenerateNew)),
                        KeyCode::Char('2') => Some(Action::OnboardingChoice(OnboardingChoice::ImportExisting)),
                        _ => None,
                    }
                }
                OnboardingMode::EnterDisplayName => {
                    match key.code {
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
                    }
                }
                OnboardingMode::ImportExisting => {
                    match key.code {
                        KeyCode::Enter if !input.is_empty() => {
                            Some(Action::SetNsec(input.clone()))
                        }
                        KeyCode::Char(c) => {
                            let mut new_input = input.clone();
                            new_input.push(c);
                            Some(Action::SetInput(new_input))
                        }
                        KeyCode::Backspace => Some(Action::Backspace),
                        _ => None,
                    }
                }
                OnboardingMode::CreatePassword | OnboardingMode::EnterPassword => {
                    match key.code {
                        KeyCode::Enter if !input.is_empty() => {
                            Some(Action::SetPassword(input.clone()))
                        }
                        KeyCode::Char(c) => {
                            let mut new_input = input.clone();
                            new_input.push(c);
                            Some(Action::SetInput(new_input))
                        }
                        KeyCode::Backspace => Some(Action::Backspace),
                        _ => None,
                    }
                }
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