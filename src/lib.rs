use anyhow::Result;
use nostr_mls::{groups::NostrGroupConfigData, messages::MessageProcessingResult, NostrMls};
use nostr_mls_memory_storage::NostrMlsMemoryStorage;
use nostr_mls_storage::groups::types as group_types;
use nostr_sdk::prelude::*;
use openmls::group::GroupId;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Message {
    pub content: String,
    pub sender: PublicKey,
    pub timestamp: Timestamp,
}

#[derive(Debug, Clone)]
pub enum AppState {
    Initializing,
    Ready {
        key_package_published: bool,
        groups: Vec<GroupId>,
    },
    InGroup {
        active_group: GroupId,
        members: Vec<PublicKey>,
    },
}

pub struct Nrc {
    nostr_mls: NostrMls<NostrMlsMemoryStorage>,
    keys: Keys,
    client: Client,
    state: AppState,
    messages: HashMap<GroupId, Vec<Message>>,
    welcome_rumors: HashMap<PublicKey, UnsignedEvent>,
    groups: HashMap<GroupId, group_types::Group>,
}

impl Nrc {
    pub async fn new() -> Result<Self> {
        let keys = Keys::generate();
        let client = Client::builder().signer(keys.clone()).build();

        // Add multiple relays for redundancy
        let relays = [
            "wss://relay.damus.io",
            "wss://nos.lol",
            "wss://relay.nostr.band",
            "wss://relay.snort.social",
            "wss://nostr.wine",
        ];

        for relay in relays {
            if let Err(e) = client.add_relay(relay).await {
                println!("Failed to add relay {relay}: {e}");
            }
        }

        client.connect().await;

        // Wait for connections to establish
        tokio::time::sleep(Duration::from_secs(2)).await;

        let storage = NostrMlsMemoryStorage::default();
        let nostr_mls = NostrMls::new(storage);

        Ok(Self {
            nostr_mls,
            keys,
            client,
            state: AppState::Initializing,
            messages: HashMap::new(),
            welcome_rumors: HashMap::new(),
            groups: HashMap::new(),
        })
    }

    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    pub async fn publish_key_package(&mut self) -> Result<()> {
        // First subscribe to key packages so we can verify our own
        let filter = Filter::new()
            .kind(Kind::from(443u16))
            .author(self.keys.public_key());
        self.client.subscribe(filter, None).await?;

        let relays = vec![
            RelayUrl::parse("wss://relay.damus.io")?,
            RelayUrl::parse("wss://nos.lol")?,
            RelayUrl::parse("wss://relay.nostr.band")?,
            RelayUrl::parse("wss://relay.snort.social")?,
            RelayUrl::parse("wss://nostr.wine")?,
        ];
        let (key_package_content, tags) = self
            .nostr_mls
            .create_key_package_for_event(&self.keys.public_key(), relays)?;

        let event = EventBuilder::new(Kind::from(443u16), key_package_content)
            .tags(tags)
            .build(self.keys.public_key())
            .sign(&self.keys)
            .await?;

        self.client.send_event(&event).await?;

        // Wait a bit to ensure it's published
        tokio::time::sleep(Duration::from_secs(1)).await;

        let filter = Filter::new()
            .kind(Kind::GiftWrap)
            .pubkey(self.keys.public_key());
        self.client.subscribe(filter, None).await?;

        if let AppState::Ready { groups, .. } = &self.state {
            self.state = AppState::Ready {
                key_package_published: true,
                groups: groups.clone(),
            };
        } else {
            self.state = AppState::Ready {
                key_package_published: true,
                groups: Vec::new(),
            };
        }

        Ok(())
    }

    pub async fn fetch_key_package(&self, pubkey: &PublicKey) -> Result<Event> {
        let filter = Filter::new()
            .kind(Kind::from(443u16))
            .author(*pubkey)
            .limit(1);

        // Subscribe to ensure we can fetch events
        self.client.subscribe(filter.clone(), None).await?;

        // Give time for event to propagate to relay and retry multiple times
        for attempt in 1..=10 {
            tokio::time::sleep(Duration::from_millis(1500)).await;

            // Try to fetch from relay
            if let Ok(events) = self
                .client
                .fetch_events(filter.clone(), Duration::from_secs(5))
                .await
            {
                if !events.is_empty() {
                    println!("Found key package on attempt {attempt}");
                    return Ok(events.into_iter().next().unwrap());
                }
            }

            if attempt % 3 == 0 {
                println!("Attempt {attempt} - key package not found yet for {pubkey}");
            }
        }

        // Last resort: check local database
        let events = self.client.database().query(filter).await?;

        events
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No key package found for {} after 10 attempts", pubkey))
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

        let group_result = self.nostr_mls.create_group(
            &self.keys.public_key(),
            vec![key_package.clone()],
            config,
        )?;

        let group_id = GroupId::from_slice(group_result.group.mls_group_id.as_slice());
        // Note: merge_pending_commit is already called inside create_group

        self.groups
            .insert(group_id.clone(), group_result.group.clone());

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

    pub async fn fetch_and_process_welcomes(&mut self) -> Result<()> {
        let filter = Filter::new()
            .kind(Kind::GiftWrap)
            .pubkey(self.keys.public_key())
            .limit(10);

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
                        .nostr_mls
                        .process_welcome(&gift_wrap.id, &unwrapped.rumor)?;

                    // Accept the welcome to actually join the group
                    self.nostr_mls.accept_welcome(&welcome)?;

                    // Get the group info from storage after accepting
                    let group_id = GroupId::from_slice(welcome.mls_group_id.as_slice());
                    if let Ok(Some(group)) = self.nostr_mls.get_group(&group_id) {
                        self.groups.insert(group_id.clone(), group);
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
                }
            }
        }

        Ok(())
    }

    pub async fn send_message(&mut self, group_id: &GroupId, content: &str) -> Result<()> {
        let text_note_rumor = EventBuilder::text_note(content).build(self.keys.public_key());

        let event = self.nostr_mls.create_message(group_id, text_note_rumor)?;

        // Note: merge_pending_commit is already called inside create_message

        println!(
            "Sending message event: id={}, kind={}",
            event.id, event.kind
        );
        self.client.send_event(&event).await?;

        // Don't store locally - we'll fetch it from the relay like other messages
        // This avoids duplicates

        Ok(())
    }

    pub async fn fetch_and_process_messages(&mut self) -> Result<()> {
        let groups = match &self.state {
            AppState::Ready { groups, .. } => groups.clone(),
            AppState::InGroup { active_group, .. } => vec![active_group.clone()],
            _ => return Ok(()),
        };

        for group_id in groups {
            // Get the actual nostr_group_id from storage
            let group = self
                .groups
                .get(&group_id)
                .ok_or(anyhow::anyhow!("Group not found in storage"))?;
            let h_tag_value = hex::encode(group.nostr_group_id);
            println!(
                "Fetching messages for MLS group: {}, Nostr group: {}",
                hex::encode(group_id.as_slice()),
                h_tag_value
            );
            let filter = Filter::new()
                .kind(Kind::from(445u16))
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value.clone())
                .limit(100);
            println!("Filter: kind=445, h-tag={h_tag_value}");

            tokio::time::sleep(Duration::from_secs(1)).await;

            let events = self
                .client
                .fetch_events(filter, Duration::from_secs(10))
                .await?;
            println!("Fetched {} events from relay", events.len());

            for event in events {
                if let Ok(MessageProcessingResult::ApplicationMessage(msg)) =
                    self.nostr_mls.process_message(&event)
                {
                    if msg.kind == Kind::TextNote {
                        if let Ok(Some(stored_msg)) = self.nostr_mls.get_message(&msg.id) {
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
                                messages.push(message);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub fn get_messages(&self, group_id: &GroupId) -> Result<Vec<Message>> {
        Ok(self.messages.get(group_id).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_two_nrc_instances_exchange_messages() -> Result<()> {
        let mut alice = Nrc::new().await?;
        let mut bob = Nrc::new().await?;

        // Publish key packages with time for propagation
        alice.publish_key_package().await?;
        println!("Alice key package published: {}", alice.public_key());
        sleep(Duration::from_secs(1)).await;

        bob.publish_key_package().await?;
        println!("Bob key package published: {}", bob.public_key());
        sleep(Duration::from_secs(1)).await;

        let bob_kp = alice.fetch_key_package(&bob.public_key()).await?;

        let group_id = alice.create_group_with_member(bob_kp).await?;

        let welcome_rumor = alice.get_welcome_rumor_for(&bob.public_key())?;
        alice
            .send_gift_wrapped_welcome(&bob.public_key(), welcome_rumor)
            .await?;

        sleep(Duration::from_secs(3)).await;

        bob.fetch_and_process_welcomes().await?;
        println!("Bob processed welcomes, his groups: {:?}", bob.state);

        alice.send_message(&group_id, "Hello Bob!").await?;
        println!(
            "Alice sent message to group: {:?}",
            hex::encode(group_id.as_slice())
        );

        sleep(Duration::from_secs(3)).await;

        bob.fetch_and_process_messages().await?;
        let messages = bob.get_messages(&group_id)?;
        println!("Bob fetched messages, count: {}", messages.len());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Hello Bob!");

        bob.send_message(&group_id, "Hi Alice!").await?;

        sleep(Duration::from_secs(3)).await;

        alice.fetch_and_process_messages().await?;
        let mut messages = alice.get_messages(&group_id)?;
        assert_eq!(messages.len(), 2);

        // Sort by timestamp since relay order isn't guaranteed
        messages.sort_by_key(|m| m.timestamp);

        // Verify both messages are present (order may vary)
        let contents: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();
        assert!(contents.contains(&"Hello Bob!".to_string()));
        assert!(contents.contains(&"Hi Alice!".to_string()));

        Ok(())
    }
}
