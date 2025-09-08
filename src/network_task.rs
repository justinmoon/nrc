use anyhow::Result;
use nostr_sdk::prelude::*;
use nrc_mls::groups::NostrGroupConfigData;
use nrc_mls_storage::groups::types as group_types;
use openmls::group::GroupId;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

use crate::{get_default_relays, AppEvent, Message, NetworkCommand};

#[derive(Debug, Clone)]
pub enum StorageCommand {
    CreateMessage {
        group_id: GroupId,
        rumor: UnsignedEvent,
    },
    GetGroup {
        group_id: GroupId,
    },
    GetGroups,
    CreateGroup {
        name: String,
        keys: Keys,
    },
    JoinGroupFromWelcome {
        welcome_rumor: UnsignedEvent,
        keys: Keys,
    },
    MergeGroupConfig {
        config: NostrGroupConfigData,
    },
    GetKeyPackage {
        keys: Keys,
    },
    ProcessMessageEvent {
        event: Event,
    },
}

#[derive(Debug, Clone)]
pub enum StorageResponse {
    MessageCreated(Event),
    Group(Option<group_types::Group>),
    Groups(HashMap<GroupId, group_types::Group>),
    GroupCreated {
        group_id: GroupId,
        welcome_rumor: UnsignedEvent,
    },
    GroupJoined(GroupId),
    GroupConfigMerged,
    KeyPackage(Event),
    MessageProcessed(Option<Message>),
    Error(String),
}

pub struct NetworkTaskState {
    keys: Keys,
    client: Client,
    storage_tx: mpsc::Sender<(StorageCommand, oneshot::Sender<StorageResponse>)>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    welcome_rumors: HashMap<PublicKey, UnsignedEvent>,
}

impl NetworkTaskState {
    async fn execute_storage_command(&self, command: StorageCommand) -> Result<StorageResponse> {
        let (tx, rx) = oneshot::channel();
        self.storage_tx.send((command, tx)).await?;
        Ok(rx.await?)
    }

    async fn fetch_key_package(&self, pubkey: &PublicKey) -> Result<Event> {
        let filter = Filter::new()
            .kind(Kind::from(443u16))
            .author(*pubkey)
            .limit(1);

        let opts = SubscribeAutoCloseOptions::default()
            .exit_policy(ReqExitPolicy::ExitOnEOSE)
            .timeout(Some(Duration::from_secs(15)));

        self.client.subscribe(filter.clone(), Some(opts)).await?;
        tokio::time::sleep(Duration::from_secs(16)).await;

        let events = self.client.database().query(filter).await?;
        events
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No key package found for {}", pubkey))
    }

    async fn publish_key_package(&self) -> Result<()> {
        let filter = Filter::new()
            .kind(Kind::from(443u16))
            .author(self.keys.public_key());
        self.client.subscribe(filter, None).await?;

        let response = self
            .execute_storage_command(StorageCommand::GetKeyPackage {
                keys: self.keys.clone(),
            })
            .await?;

        if let StorageResponse::KeyPackage(event) = response {
            self.client.send_event(&event).await?;
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Failed to get key package from storage"))
        }
    }

    async fn publish_profile(&self, display_name: String) -> Result<()> {
        let metadata = Metadata::new().display_name(display_name);
        let event = EventBuilder::metadata(&metadata).sign(&self.keys).await?;
        self.client.send_event(&event).await?;
        Ok(())
    }

    async fn create_group(&mut self, name: String) -> Result<GroupId> {
        let response = self
            .execute_storage_command(StorageCommand::CreateGroup {
                name,
                keys: self.keys.clone(),
            })
            .await?;

        if let StorageResponse::GroupCreated {
            group_id,
            welcome_rumor,
        } = response
        {
            self.welcome_rumors
                .insert(self.keys.public_key(), welcome_rumor);
            Ok(group_id)
        } else {
            Err(anyhow::anyhow!("Failed to create group"))
        }
    }

    async fn join_group(&mut self, npub: String) -> Result<GroupId> {
        let recipient = PublicKey::from_bech32(&npub)?;

        // Fetch their key package
        let key_package_event = self.fetch_key_package(&recipient).await?;
        let key_package_json = key_package_event.content.as_str();
        use base64::Engine;
        let _key_package_bytes =
            base64::engine::general_purpose::STANDARD.decode(key_package_json)?;

        // Create group with recipient
        let response = self
            .execute_storage_command(StorageCommand::CreateGroup {
                name: format!("Group with {npub}"),
                keys: self.keys.clone(),
            })
            .await?;

        if let StorageResponse::GroupCreated {
            group_id,
            welcome_rumor,
        } = response
        {
            // Send welcome message
            let gift_wrap =
                EventBuilder::gift_wrap(&self.keys, &recipient, welcome_rumor, None).await?;
            self.client.send_event(&gift_wrap).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(group_id)
        } else {
            Err(anyhow::anyhow!("Failed to create group for join"))
        }
    }

    async fn send_message(&self, group_id: GroupId, content: String) -> Result<()> {
        let text_note_rumor = EventBuilder::text_note(&content).build(self.keys.public_key());

        let response = self
            .execute_storage_command(StorageCommand::CreateMessage {
                group_id: group_id.clone(),
                rumor: text_note_rumor.clone(),
            })
            .await?;

        if let StorageResponse::MessageCreated(event) = response {
            self.client.send_event(&event).await?;

            // Message will be received back via notification handler and processed there
            // This prevents duplicate messages in the chat
            Ok(())
        } else {
            Err(anyhow::anyhow!("Failed to create message"))
        }
    }

    async fn fetch_messages(&self) -> Result<()> {
        let response = self
            .execute_storage_command(StorageCommand::GetGroups)
            .await?;

        if let StorageResponse::Groups(groups) = response {
            for group in groups.values() {
                let h_tag = hex::encode(group.nostr_group_id);
                let filter = Filter::new()
                    .kind(Kind::from(444u16))
                    .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag);

                let opts = SubscribeAutoCloseOptions::default()
                    .exit_policy(ReqExitPolicy::ExitOnEOSE)
                    .timeout(Some(Duration::from_secs(5)));

                self.client.subscribe(filter, Some(opts)).await?;
            }

            // Wait for subscriptions to complete
            tokio::time::sleep(Duration::from_secs(6)).await;

            // Query all messages
            let mut all_events = Vec::new();
            for group in groups.values() {
                let h_tag = hex::encode(group.nostr_group_id);
                let filter = Filter::new()
                    .kind(Kind::from(444u16))
                    .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag);

                let events = self.client.database().query(filter).await?;
                all_events.extend(events);
            }

            if !all_events.is_empty() {
                let _ = self
                    .event_tx
                    .send(AppEvent::RawMessagesReceived { events: all_events });
            }
        }
        Ok(())
    }

    async fn fetch_welcomes(&self) -> Result<()> {
        let filter = Filter::new()
            .kind(Kind::GiftWrap)
            .pubkey(self.keys.public_key());

        let opts = SubscribeAutoCloseOptions::default()
            .exit_policy(ReqExitPolicy::ExitOnEOSE)
            .timeout(Some(Duration::from_secs(10)));

        self.client.subscribe(filter.clone(), Some(opts)).await?;
        tokio::time::sleep(Duration::from_secs(11)).await;

        let events = self.client.database().query(filter).await?;
        if !events.is_empty() {
            let events_vec: Vec<Event> = events.into_iter().collect();
            let _ = self
                .event_tx
                .send(AppEvent::RawWelcomesReceived { events: events_vec });
        }
        Ok(())
    }
}

pub async fn spawn_network_task(
    mut command_rx: mpsc::Receiver<NetworkCommand>,
    storage_tx: mpsc::Sender<(StorageCommand, oneshot::Sender<StorageResponse>)>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    keys: Keys,
) {
    tokio::spawn(async move {
        // Initialize client
        let client = Client::builder().signer(keys.clone()).build();

        // Add default relays
        for relay_url in get_default_relays() {
            if let Err(e) = client.add_relay(*relay_url).await {
                log::warn!("Failed to add relay {relay_url}: {e}");
            }
        }

        // Connect to relays
        client.connect().await;

        let mut state = NetworkTaskState {
            keys,
            client,
            storage_tx,
            event_tx: event_tx.clone(),
            welcome_rumors: HashMap::new(),
        };

        while let Some(command) = command_rx.recv().await {
            match command {
                NetworkCommand::PublishKeyPackage => match state.publish_key_package().await {
                    Ok(()) => {
                        let _ = event_tx.send(AppEvent::KeyPackagePublished);
                    }
                    Err(e) => {
                        let _ = event_tx.send(AppEvent::NetworkError {
                            error: format!("Failed to publish key package: {e}"),
                        });
                    }
                },
                NetworkCommand::PublishProfile { display_name } => {
                    match state.publish_profile(display_name).await {
                        Ok(()) => {
                            let _ = event_tx.send(AppEvent::ProfilePublished);
                        }
                        Err(e) => {
                            let _ = event_tx.send(AppEvent::NetworkError {
                                error: format!("Failed to publish profile: {e}"),
                            });
                        }
                    }
                }
                NetworkCommand::CreateGroup { name } => match state.create_group(name).await {
                    Ok(group_id) => {
                        let _ = event_tx.send(AppEvent::GroupCreated { group_id });
                    }
                    Err(e) => {
                        let _ = event_tx.send(AppEvent::NetworkError {
                            error: format!("Failed to create group: {e}"),
                        });
                    }
                },
                NetworkCommand::JoinGroup { npub } => match state.join_group(npub).await {
                    Ok(group_id) => {
                        let _ = event_tx.send(AppEvent::GroupCreated { group_id });
                    }
                    Err(e) => {
                        let _ = event_tx.send(AppEvent::NetworkError {
                            error: format!("Failed to join group: {e}"),
                        });
                    }
                },
                NetworkCommand::SendMessage { group_id, content } => {
                    match state.send_message(group_id, content).await {
                        Ok(()) => {}
                        Err(e) => {
                            let _ = event_tx.send(AppEvent::NetworkError {
                                error: format!("Failed to send message: {e}"),
                            });
                        }
                    }
                }
                NetworkCommand::FetchMessages => match state.fetch_messages().await {
                    Ok(()) => {}
                    Err(e) => {
                        log::error!("Failed to fetch messages: {e}");
                    }
                },
                NetworkCommand::FetchWelcomes => match state.fetch_welcomes().await {
                    Ok(()) => {}
                    Err(e) => {
                        log::error!("Failed to fetch welcomes: {e}");
                    }
                },
            }
        }
    });
}
