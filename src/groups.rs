use crate::config::get_default_relays;
use crate::types::AppState;
use crate::utils::pubkey_to_bech32_safe;
use crate::Nrc;
use anyhow::Result;
use nostr_sdk::prelude::*;
use nrc_mls::groups::NostrGroupConfigData;
use openmls::group::GroupId;
use std::time::Duration;

impl Nrc {
    pub async fn create_group(&mut self, name: String) -> Result<GroupId> {
        // Use network task channel instead of direct network call
        if let Some(command_tx) = &self.command_tx {
            command_tx
                .send(crate::NetworkCommand::CreateGroup { name })
                .await?;
            // Return a placeholder - actual group ID will come via events
            Ok(GroupId::from_slice(&[0u8; 16]))
        } else {
            Err(anyhow::anyhow!("Network task not initialized"))
        }
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
}
