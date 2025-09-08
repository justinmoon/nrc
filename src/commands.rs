use crate::types::AppState;
use crate::utils::pubkey_to_bech32_safe;
use crate::Nrc;
use anyhow::Result;
use nostr_sdk::prelude::*;
use std::str::FromStr;

impl Nrc {
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
}
