use crate::types::AppState;
use crate::utils::pubkey_to_bech32_safe;
use crate::Nrc;
use anyhow::{Context, Result};
use nostr_sdk::prelude::*;
use std::str::FromStr;

impl Nrc {
    pub async fn process_input(&mut self, input: String) -> Result<bool> {
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
            self.send_message(group_id.clone(), input)
                .await
                .context("Failed to send message")?;
            log::info!(
                "Message sent successfully to group {}",
                hex::encode(group_id.as_slice())
            );
        } else {
            anyhow::bail!("No chat selected");
        }

        Ok(false)
    }

    pub async fn process_command(&mut self, input: String) -> Result<bool> {
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
            anyhow::bail!("Commands: /group, /dm, /invite, /members, /leave, /npub, /help, /quit");
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
            .context("Failed to convert public key to bech32")?;

        let mut ctx = ClipboardContext::new()
            .map_err(|e| anyhow::anyhow!("Clipboard not available: {}", e))?;
        ctx.set_contents(npub.clone())
            .map_err(|e| anyhow::anyhow!("Failed to copy to clipboard: {}", e))?;

        // Note: Flash message for success will be handled at UI layer
        log::info!("Copied npub to clipboard: {npub}");
        Ok(false)
    }

    async fn handle_group_command(&mut self, input: &str) -> Result<bool> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.len() < 3 {
            anyhow::bail!("Usage: /group #channelname <npub1> [npub2] ...");
        }

        let channel_name = parts[1].trim_start_matches('#');
        if channel_name.is_empty() {
            anyhow::bail!("Channel name cannot be empty");
        }

        // Parse member public keys
        let mut member_pubkeys = Vec::new();
        for pubkey_str in &parts[2..] {
            let pubkey = PublicKey::from_str(pubkey_str).with_context(|| {
                format!("'{pubkey_str}' is not a valid public key (should start with 'npub1')")
            })?;
            member_pubkeys.push(pubkey);
        }

        self.create_multi_user_group(channel_name.to_string(), member_pubkeys)
            .await
            .context("Failed to create group")?;

        Ok(false)
    }

    async fn handle_dm_command(&mut self, input: &str) -> Result<bool> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.len() < 2 {
            anyhow::bail!("Usage: /dm <npub>");
        }

        let pubkey_str = parts[1];
        let pubkey = PublicKey::from_str(pubkey_str).with_context(|| {
            format!("'{pubkey_str}' is not a valid public key (should start with 'npub1')")
        })?;

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
            // Note: This is not an error, just informational
            log::info!("Already in a group with {pubkey_str}, not creating a new one");
            anyhow::bail!("Already in a group with {pubkey_str}");
        }

        // If not already in a group, fetch their key package and create one
        let key_package = self
            .fetch_key_package(&pubkey)
            .await
            .with_context(|| format!("Failed to fetch key package for {pubkey_str}"))?;

        // Create a group with them
        let group_id = self
            .create_group_with_member(key_package)
            .await
            .context("Failed to create group")?;

        // Send them the welcome
        match self.get_welcome_rumor_for(&pubkey) {
            Ok(welcome_rumor) => {
                log::info!("Sending welcome to {}", pubkey_to_bech32_safe(&pubkey));
                if let Err(e) = self.send_gift_wrapped_welcome(&pubkey, welcome_rumor).await {
                    log::error!("Failed to send welcome: {e}");
                    // Don't fail the whole operation if welcome fails to send
                }
            }
            Err(e) => {
                log::error!("Failed to get welcome rumor: {e}");
                // Don't fail the whole operation if welcome fails
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
        Ok(false)
    }

    async fn handle_invite_command(&mut self, input: &str) -> Result<bool> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.len() < 2 {
            anyhow::bail!("Usage: /invite <npub>");
        }

        let pubkey_str = parts[1];
        let pubkey = PublicKey::from_str(pubkey_str).with_context(|| {
            format!("'{pubkey_str}' is not a valid public key (should start with 'npub1')")
        })?;

        let group_id = self
            .get_selected_group()
            .ok_or_else(|| anyhow::anyhow!("No group selected. Select a group first."))?;

        self.invite_to_group(group_id, pubkey)
            .await
            .context("Failed to invite to group")?;

        log::info!("Invited {pubkey_str} to group");
        Ok(false)
    }

    async fn handle_members_command(&mut self) -> Result<bool> {
        let group_id = self
            .get_selected_group()
            .ok_or_else(|| anyhow::anyhow!("No group selected"))?;

        let members = self
            .storage
            .get_members(&group_id)
            .context("Failed to get members")?;

        let member_list: Vec<String> = members
            .iter()
            .map(|pk| self.get_display_name_for_pubkey(pk))
            .collect();

        // Note: Member list display will be handled at UI layer
        log::info!("Members: {}", member_list.join(", "));
        anyhow::bail!("Members: {}", member_list.join(", "));
    }

    async fn handle_leave_command(&mut self) -> Result<bool> {
        let group_id = self
            .get_selected_group()
            .ok_or_else(|| anyhow::anyhow!("No group selected"))?;

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
            log::info!("Left the group");
        }
        Ok(false)
    }
}
