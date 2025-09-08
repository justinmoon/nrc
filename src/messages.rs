use crate::types::{AppState, Message};
use crate::utils::pubkey_to_bech32_safe;
use crate::Nrc;
use anyhow::Result;
use nostr_sdk::prelude::*;
use nrc_mls::messages::MessageProcessingResult;
use openmls::group::GroupId;
use std::time::Duration;

impl Nrc {
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

    pub fn get_messages(&self, group_id: &GroupId) -> Vec<Message> {
        self.messages.get(group_id).cloned().unwrap_or_default()
    }

    pub fn add_message(&mut self, group_id: GroupId, message: Message) {
        self.messages.entry(group_id).or_default().push(message);
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
                                    log::info!("Joined new group via invitation!");
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
