use crate::Nrc;
use anyhow::Result;
use nostr_sdk::prelude::*;
use openmls::group::GroupId;
use std::time::Duration;

impl Nrc {
    pub async fn publish_profile(&mut self, display_name: String) -> Result<()> {
        // Create metadata for the profile
        let metadata = Metadata::new()
            .display_name(display_name.clone())
            .name(display_name.clone());

        // Store it locally too
        self.profiles
            .insert(self.keys.public_key(), metadata.clone());

        // Publish to Nostr
        let event = EventBuilder::metadata(&metadata).sign(&self.keys).await?;
        self.client.send_event(&event).await?;

        log::info!("Published profile with display name: {display_name}");
        Ok(())
    }

    pub async fn fetch_profile(&mut self, pubkey: &PublicKey) -> Result<()> {
        let filter = Filter::new().kind(Kind::Metadata).author(*pubkey).limit(1);

        let events = self
            .client
            .fetch_events(filter, Duration::from_secs(5))
            .await?;

        if let Some(event) = events.first() {
            if let Ok(metadata) = Metadata::from_json(&event.content) {
                self.profiles.insert(*pubkey, metadata);
            }
        }

        Ok(())
    }

    pub fn get_display_name_for_pubkey(&self, pubkey: &PublicKey) -> String {
        if let Some(profile) = self.profiles.get(pubkey) {
            if let Some(name) = profile.display_name.as_ref() {
                if !name.is_empty() {
                    return name.clone();
                }
            }
            if let Some(name) = profile.name.as_ref() {
                if !name.is_empty() {
                    return name.clone();
                }
            }
        }

        // Fall back to npub if no profile
        pubkey
            .to_bech32()
            .map(|npub| {
                // Shorten npub for display: npub1abc...xyz
                if npub.len() > 20 {
                    format!("{}...{}", &npub[..10], &npub[npub.len() - 3..])
                } else {
                    npub
                }
            })
            .unwrap_or_else(|_| "Unknown".to_string())
    }

    pub fn get_chat_display_name(&self, group_id: &GroupId) -> String {
        if let Some(group) = self.groups.get(group_id) {
            // Check if it's a multi-user group by member count
            match self.storage.get_members(group_id) {
                Ok(members) => {
                    if members.len() > 2 {
                        // Multi-user group - show as #channelname
                        return format!("#{}", group.name);
                    } else if members.len() == 2 {
                        // Direct message - show other person's name
                        let our_pubkey = self.keys.public_key();
                        for member in &members {
                            if member != &our_pubkey {
                                return self.get_display_name_for_pubkey(member);
                            }
                        }
                    }
                }
                Err(_) => {
                    // Fallback to old logic if we can't get members
                    let our_pubkey = self.keys.public_key();

                    // Check if group name looks like a channel (not "Test Group")
                    if !group.name.starts_with("Test") && !group.name.is_empty() {
                        // Likely a multi-user group with a proper name
                        return format!("#{}", group.name);
                    }

                    // Otherwise treat as DM - find the other person
                    for admin in &group.admin_pubkeys {
                        if admin != &our_pubkey {
                            return self.get_display_name_for_pubkey(admin);
                        }
                    }

                    // Try to find from messages
                    if let Some(messages) = self.messages.get(group_id) {
                        for msg in messages {
                            if msg.sender != our_pubkey {
                                return self.get_display_name_for_pubkey(&msg.sender);
                            }
                        }
                    }
                }
            }
        }
        "Unknown".to_string()
    }
}
