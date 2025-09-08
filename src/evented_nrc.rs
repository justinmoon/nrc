use crate::actions::{Action, OnboardingChoice};
use crate::{AppState, Message, Nrc, OnboardingMode};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nostr_sdk::prelude::*;
use openmls::group::GroupId;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use tokio::sync::{mpsc, watch};

pub struct EventedNrc {
    // Read-only state for UI
    pub state: watch::Receiver<AppState>,
    
    // UI can read these for display
    pub messages: watch::Receiver<HashMap<GroupId, Vec<Message>>>,
    pub groups: watch::Receiver<HashMap<GroupId, nrc_mls_storage::groups::types::Group>>,
    pub input: watch::Receiver<String>,
    pub selected_group_index: watch::Receiver<Option<usize>>,
    pub last_error: watch::Receiver<Option<String>>,
    pub flash_message: watch::Receiver<Option<String>>,
    pub show_help: watch::Receiver<bool>,
    pub npub: String, // Static after initialization
    
    // Send actions to the event loop
    action_tx: mpsc::UnboundedSender<Action>,
}

pub struct EventLoop {
    nrc: Nrc,
    action_rx: mpsc::UnboundedReceiver<Action>,
    state_tx: watch::Sender<AppState>,
    messages_tx: watch::Sender<HashMap<GroupId, Vec<Message>>>,
    groups_tx: watch::Sender<HashMap<GroupId, nrc_mls_storage::groups::types::Group>>,
    input_tx: watch::Sender<String>,
    selected_tx: watch::Sender<Option<usize>>,
    error_tx: watch::Sender<Option<String>>,
    flash_tx: watch::Sender<Option<String>>,
    help_tx: watch::Sender<bool>,
}

impl EventedNrc {
    pub async fn new(datadir: &Path) -> Result<(Self, EventLoop)> {
        // Create the underlying Nrc
        let nrc = Nrc::new(datadir).await?;
        let npub = nrc.keys.public_key().to_bech32()?;
        
        // Create channels for state watching
        let (state_tx, state_rx) = watch::channel(nrc.state.clone());
        let (messages_tx, messages_rx) = watch::channel(nrc.messages.clone());
        let (groups_tx, groups_rx) = watch::channel(nrc.groups.clone());
        let (input_tx, input_rx) = watch::channel(nrc.input.clone());
        let (selected_tx, selected_rx) = watch::channel(nrc.selected_group_index);
        let (error_tx, error_rx) = watch::channel(nrc.last_error.clone());
        let (flash_tx, flash_rx) = watch::channel(nrc.flash_message.clone());
        let (help_tx, help_rx) = watch::channel(nrc.show_help);
        
        // Create action channel
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        
        let evented = EventedNrc {
            state: state_rx,
            messages: messages_rx,
            groups: groups_rx,
            input: input_rx,
            selected_group_index: selected_rx,
            last_error: error_rx,
            flash_message: flash_rx,
            show_help: help_rx,
            npub,
            action_tx,
        };
        
        let event_loop = EventLoop {
            nrc,
            action_rx,
            state_tx,
            messages_tx,
            groups_tx,
            input_tx,
            selected_tx,
            error_tx,
            flash_tx,
            help_tx,
        };
        
        Ok((evented, event_loop))
    }
    
    /// Emit an action to be processed
    pub fn emit(&self, action: Action) {
        log::debug!("🚀 EMIT: {:?}", action);
        let _ = self.action_tx.send(action);
    }
    
    /// Helper methods for common UI operations
    pub fn get_selected_group(&self) -> Option<GroupId> {
        if let AppState::Ready { ref groups, .. } = &*self.state.borrow() {
            self.selected_group_index
                .borrow()
                .and_then(|idx| groups.get(idx))
                .cloned()
        } else {
            None
        }
    }
    
    pub fn get_messages_for_selected_group(&self) -> Vec<Message> {
        if let Some(group_id) = self.get_selected_group() {
            self.messages
                .borrow()
                .get(&group_id)
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    }
    
    pub fn get_group_display_name(&self, group_id: &GroupId) -> String {
        self.groups
            .borrow()
            .get(group_id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| "Unknown".to_string())
    }
}

impl EventLoop {
    /// Run the event loop - this should be called in the main task
    pub async fn run(mut self) {
        while let Some(action) = self.action_rx.recv().await {
            log::debug!("⚡ PROCESS: {:?}", action);
            if let Err(e) = process_action(
                &mut self.nrc,
                action,
                &self.state_tx,
                &self.messages_tx,
                &self.groups_tx,
                &self.input_tx,
                &self.selected_tx,
                &self.error_tx,
                &self.flash_tx,
                &self.help_tx,
            ).await {
                log::error!("❌ Error processing action: {}", e);
                let _ = self.error_tx.send(Some(format!("Error: {}", e)));
            }
        }
        log::debug!("🔚 Action channel closed, stopping event loop");
    }
    
    /// Process a single action (for testing or step-by-step execution)
    pub async fn process_one(&mut self) -> Option<()> {
        match self.action_rx.try_recv() {
            Ok(action) => {
                log::debug!("⚡ PROCESS: {:?}", action);
                if let Err(e) = process_action(
                    &mut self.nrc,
                    action,
                    &self.state_tx,
                    &self.messages_tx,
                    &self.groups_tx,
                    &self.input_tx,
                    &self.selected_tx,
                    &self.error_tx,
                    &self.flash_tx,
                    &self.help_tx,
                ).await {
                    log::error!("❌ Error processing action: {}", e);
                    let _ = self.error_tx.send(Some(format!("Error: {}", e)));
                }
                Some(())
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                log::debug!("🔚 No actions in queue");
                None
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                log::debug!("🔚 Action channel disconnected");
                None
            }
        }
    }
}

/// Process an action and update all state watchers
async fn process_action(
    nrc: &mut Nrc,
    action: Action,
    state_tx: &watch::Sender<AppState>,
    messages_tx: &watch::Sender<HashMap<GroupId, Vec<Message>>>,
    groups_tx: &watch::Sender<HashMap<GroupId, nrc_mls_storage::groups::types::Group>>,
    input_tx: &watch::Sender<String>,
    selected_tx: &watch::Sender<Option<usize>>,
    error_tx: &watch::Sender<Option<String>>,
    flash_tx: &watch::Sender<Option<String>>,
    help_tx: &watch::Sender<bool>,
) -> Result<()> {
    // Clear any previous errors/messages
    nrc.last_error = None;
    nrc.flash_message = None;
    
    match action {
        Action::KeyPress(key) => {
            handle_key_press(nrc, key).await?;
        }
        Action::Paste(text) => {
            nrc.input.push_str(&text);
        }
        Action::SetInput(text) => {
            nrc.input = text;
        }
        Action::ClearInput => {
            nrc.input.clear();
        }
        Action::Backspace => {
            nrc.input.pop();
        }
        Action::SendMessage(content) => {
            if let Some(group_id) = get_selected_group(nrc) {
                if let Err(e) = nrc.send_message(group_id, content).await {
                    nrc.last_error = Some(format!("Failed to send: {}", e));
                }
            } else {
                nrc.last_error = Some("No chat selected".to_string());
            }
        }
        Action::JoinGroup(npub) => {
            // Process the /join command
            let pubkey = match PublicKey::from_str(&npub) {
                Ok(pk) => pk,
                Err(e) => {
                    nrc.last_error = Some(format!("Invalid public key: {}", e));
                    update_watchers(nrc, state_tx, messages_tx, groups_tx, input_tx, selected_tx, error_tx, flash_tx, help_tx);
                    return Ok(());
                }
            };
            
            // Fetch profile first
            let _ = nrc.fetch_profile(&pubkey).await;
            
            // Check if already in group
            let already_in_group = if let AppState::Ready { ref groups, .. } = nrc.state {
                groups.iter().any(|group_id| {
                    nrc.groups.get(group_id)
                        .map(|g| g.admin_pubkeys.contains(&pubkey))
                        .unwrap_or(false)
                })
            } else {
                false
            };
            
            if already_in_group {
                nrc.flash_message = Some(format!("Already in a group with {}", npub));
            } else {
                // Fetch key package and create group
                match nrc.fetch_key_package(&pubkey).await {
                    Ok(key_package) => {
                        match nrc.create_group_with_member(key_package).await {
                            Ok(group_id) => {
                                // Send welcome
                                if let Ok(welcome_rumor) = nrc.get_welcome_rumor_for(&pubkey) {
                                    let _ = nrc.send_gift_wrapped_welcome(&pubkey, welcome_rumor).await;
                                }
                                
                                // Update state and select new group
                                if let AppState::Ready { key_package_published, groups } = &nrc.state {
                                    let mut updated_groups = groups.clone();
                                    if !updated_groups.contains(&group_id) {
                                        updated_groups.push(group_id.clone());
                                    }
                                    nrc.state = AppState::Ready {
                                        key_package_published: *key_package_published,
                                        groups: updated_groups.clone(),
                                    };
                                    if let Some(idx) = updated_groups.iter().position(|g| g == &group_id) {
                                        nrc.selected_group_index = Some(idx);
                                    }
                                }
                            }
                            Err(e) => {
                                nrc.last_error = Some(format!("Failed to create group: {}", e));
                            }
                        }
                    }
                    Err(e) => {
                        nrc.last_error = Some(format!("Failed to fetch key package: {}", e));
                    }
                }
            }
        }
        Action::NextGroup => {
            nrc.next_group();
        }
        Action::PrevGroup => {
            nrc.prev_group();
        }
        Action::ShowHelp => {
            nrc.show_help = true;
            nrc.help_explicitly_requested = true;
        }
        Action::DismissHelp => {
            nrc.dismiss_help();
        }
        Action::CopyNpub => {
            use clipboard::ClipboardContext;
            use clipboard::ClipboardProvider;
            
            let npub = nrc.keys.public_key().to_bech32().unwrap_or_else(|_| "error".to_string());
            match ClipboardContext::new() {
                Ok(mut ctx) => {
                    if let Err(e) = ctx.set_contents(npub.clone()) {
                        nrc.last_error = Some(format!("Failed to copy: {}", e));
                    } else {
                        nrc.flash_message = Some(format!("Copied npub to clipboard: {}", npub));
                    }
                }
                Err(e) => {
                    nrc.last_error = Some(format!("Clipboard not available: {}", e));
                }
            }
        }
        Action::OnboardingChoice(choice) => {
            handle_onboarding_choice(nrc, choice).await?;
        }
        Action::SetDisplayName(name) => {
            nrc.onboarding_data.display_name = Some(name);
            // Transition to password creation mode
            nrc.state = AppState::Onboarding {
                input: String::new(),
                mode: OnboardingMode::CreatePassword,
            };
        }
        Action::SetPassword(password) => {
            // Handle password based on current onboarding mode
            match &nrc.state {
                AppState::Onboarding { mode, .. } => {
                    match mode {
                        OnboardingMode::CreatePassword => {
                            // New user with password
                            if let Some(display_name) = &nrc.onboarding_data.display_name {
                                let _ = nrc.initialize_with_display_name_and_password(
                                    display_name.clone(),
                                    password
                                ).await;
                            }
                        }
                        OnboardingMode::ImportExisting => {
                            // Import with nsec and password
                            if let Some(nsec) = &nrc.onboarding_data.nsec {
                                let _ = nrc.initialize_with_nsec_and_password(
                                    nsec.clone(),
                                    password
                                ).await;
                            }
                        }
                        OnboardingMode::EnterPassword => {
                            // Returning user
                            let _ = nrc.initialize_with_password(password).await;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        Action::SetNsec(nsec) => {
            nrc.onboarding_data.nsec = Some(nsec);
            // Transition to password creation mode for import flow
            nrc.state = AppState::Onboarding {
                input: String::new(),
                mode: OnboardingMode::CreatePassword,
            };
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
    
    // Update all watchers with new state
    update_watchers(nrc, state_tx, messages_tx, groups_tx, input_tx, selected_tx, error_tx, flash_tx, help_tx);
    
    Ok(())
}

fn update_watchers(
    nrc: &Nrc,
    state_tx: &watch::Sender<AppState>,
    messages_tx: &watch::Sender<HashMap<GroupId, Vec<Message>>>,
    groups_tx: &watch::Sender<HashMap<GroupId, nrc_mls_storage::groups::types::Group>>,
    input_tx: &watch::Sender<String>,
    selected_tx: &watch::Sender<Option<usize>>,
    error_tx: &watch::Sender<Option<String>>,
    flash_tx: &watch::Sender<Option<String>>,
    help_tx: &watch::Sender<bool>,
) {
    log::debug!("📡 STATE UPDATE: {:?}", nrc.state);
    if let Some(ref err) = nrc.last_error {
        log::debug!("📡 ERROR UPDATE: {}", err);
    }
    if let Some(ref flash) = nrc.flash_message {
        log::debug!("📡 FLASH UPDATE: {}", flash);
    }
    
    let _ = state_tx.send(nrc.state.clone());
    let _ = messages_tx.send(nrc.messages.clone());
    let _ = groups_tx.send(nrc.groups.clone());
    let _ = input_tx.send(nrc.input.clone());
    let _ = selected_tx.send(nrc.selected_group_index);
    let _ = error_tx.send(nrc.last_error.clone());
    let _ = flash_tx.send(nrc.flash_message.clone());
    let _ = help_tx.send(nrc.show_help);
}

fn get_selected_group(nrc: &Nrc) -> Option<GroupId> {
    if let AppState::Ready { ref groups, .. } = &nrc.state {
        nrc.selected_group_index
            .and_then(|idx| groups.get(idx))
            .cloned()
    } else {
        None
    }
}

async fn handle_key_press(nrc: &mut Nrc, key: KeyEvent) -> Result<()> {
    match &nrc.state {
        AppState::Onboarding { mode, .. } => {
            handle_onboarding_key(nrc, key, mode.clone()).await?;
        }
        AppState::Ready { .. } => {
            handle_ready_key(nrc, key).await?;
        }
        _ => {}
    }
    Ok(())
}

async fn handle_onboarding_key(nrc: &mut Nrc, key: KeyEvent, mode: OnboardingMode) -> Result<()> {
    match mode {
        OnboardingMode::Choose => {
            match key.code {
                KeyCode::Char('1') => {
                    nrc.state = AppState::Onboarding {
                        input: String::new(),
                        mode: OnboardingMode::EnterDisplayName,
                    };
                }
                KeyCode::Char('2') => {
                    nrc.state = AppState::Onboarding {
                        input: String::new(),
                        mode: OnboardingMode::ImportExisting,
                    };
                }
                _ => {}
            }
        }
        OnboardingMode::EnterDisplayName => {
            match key.code {
                KeyCode::Enter if !nrc.input.is_empty() => {
                    nrc.onboarding_data.display_name = Some(nrc.input.clone());
                    nrc.input.clear();
                    nrc.state = AppState::Onboarding {
                        input: String::new(),
                        mode: OnboardingMode::CreatePassword,
                    };
                }
                KeyCode::Char(c) => nrc.input.push(c),
                KeyCode::Backspace => { nrc.input.pop(); }
                _ => {}
            }
        }
        OnboardingMode::CreatePassword | OnboardingMode::EnterPassword => {
            match key.code {
                KeyCode::Enter if !nrc.input.is_empty() => {
                    let password = nrc.input.clone();
                    nrc.input.clear();
                    
                    if mode == OnboardingMode::CreatePassword {
                        if let Some(display_name) = &nrc.onboarding_data.display_name {
                            let _ = nrc.initialize_with_display_name_and_password(
                                display_name.clone(),
                                password
                            ).await;
                        }
                    } else {
                        let _ = nrc.initialize_with_password(password).await;
                    }
                }
                KeyCode::Char(c) => nrc.input.push(c),
                KeyCode::Backspace => { nrc.input.pop(); }
                _ => {}
            }
        }
        OnboardingMode::ImportExisting => {
            match key.code {
                KeyCode::Enter if !nrc.input.is_empty() => {
                    nrc.onboarding_data.nsec = Some(nrc.input.clone());
                    nrc.input.clear();
                    // Now ask for password
                    nrc.state = AppState::Onboarding {
                        input: String::new(),
                        mode: OnboardingMode::ImportExisting, // Reuse for password entry
                    };
                }
                KeyCode::Char(c) => nrc.input.push(c),
                KeyCode::Backspace => { nrc.input.pop(); }
                _ => {}
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_ready_key(nrc: &mut Nrc, key: KeyEvent) -> Result<()> {
    if nrc.show_help && !nrc.help_explicitly_requested {
        nrc.dismiss_help();
        return Ok(());
    }
    
    match key.code {
        KeyCode::Enter if !nrc.input.is_empty() => {
            let input = nrc.input.clone();
            nrc.input.clear();
            let _ = nrc.process_input(input).await;
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            nrc.input.push(c);
        }
        KeyCode::Backspace => { nrc.input.pop(); }
        KeyCode::Tab => nrc.next_group(),
        KeyCode::BackTab => nrc.prev_group(),
        _ => {}
    }
    Ok(())
}

async fn handle_onboarding_choice(nrc: &mut Nrc, choice: OnboardingChoice) -> Result<()> {
    match choice {
        OnboardingChoice::GenerateNew => {
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
            // Handle multi-step onboarding continuation
        }
    }
    Ok(())
}