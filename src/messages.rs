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
        // Use network task channel instead of direct network call
        if let Some(command_tx) = &self.command_tx {
            command_tx
                .send(crate::NetworkCommand::SendMessage { group_id, content })
                .await?;
        } else {
            return Err(anyhow::anyhow!("Network task not initialized"));
        }
        Ok(())
    }

    pub async fn fetch_welcomes_async(&mut self) -> Result<()> {
        // Use network task channel instead of direct network call
        if let Some(command_tx) = &self.command_tx {
            command_tx
                .send(crate::NetworkCommand::FetchWelcomes)
                .await?;
        } else {
            return Err(anyhow::anyhow!("Network task not initialized"));
        }
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

                                // Note: Message subscription and profile fetching
                                // are handled by background tasks for better performance
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
