use anyhow::Result;
use nostr_mls::{groups::NostrGroupConfigData, NostrMls};
use nostr_mls_memory_storage::NostrMlsMemoryStorage;
use nostr_mls_sqlite_storage::NostrMlsSqliteStorage;
use nostr_sdk::prelude::*;
use openmls::group::GroupId;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::{AppEvent, NetworkCommand, Message, Storage, with_storage, with_storage_mut};

pub struct NetworkState {
    pub storage: Storage,
    pub keys: Keys,
    pub client: Client,
    pub groups: HashMap<GroupId, nostr_mls_storage::groups::types::Group>,
    pub welcome_rumors: HashMap<PublicKey, UnsignedEvent>,
}

impl NetworkState {
    pub async fn fetch_key_package(&self, pubkey: &PublicKey) -> Result<Event> {
        let filter = Filter::new()
            .kind(Kind::from(443u16))
            .author(*pubkey)
            .limit(1);

        self.client.subscribe(filter.clone(), None).await?;

        for attempt in 1..=10 {
            tokio::time::sleep(Duration::from_millis(1500)).await;

            if let Ok(events) = self
                .client
                .fetch_events(filter.clone(), Duration::from_secs(5))
                .await
            {
                if !events.is_empty() {
                    log::debug!("Found key package on attempt {attempt}");
                    return Ok(events.into_iter().next().unwrap());
                }
            }

            if attempt % 3 == 0 {
                log::debug!("Attempt {attempt} - key package not found yet for {pubkey}");
            }
        }

        let events = self.client.database().query(filter).await?;
        events
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No key package found for {} after 10 attempts", pubkey))
    }

    pub async fn send_gift_wrapped_welcome(
        &mut self,
        recipient: PublicKey,
        welcome_rumor: UnsignedEvent,
    ) -> Result<()> {
        let gift_wrap = EventBuilder::gift_wrap(&self.keys, &recipient, welcome_rumor, None)?;
        self.client.send_event(gift_wrap).await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(())
    }
}

pub async fn spawn_network_task(
    mut command_rx: mpsc::Receiver<NetworkCommand>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    mut state: NetworkState,
) {
    tokio::spawn(async move {
        while let Some(command) = command_rx.recv().await {
            match command {
                NetworkCommand::PublishKeyPackage => {
                    match publish_key_package(&mut state).await {
                        Ok(()) => {
                            let _ = event_tx.send(AppEvent::KeyPackagePublished);
                        }
                        Err(e) => {
                            let _ = event_tx.send(AppEvent::NetworkError {
                                error: format!("Failed to publish key package: {}", e),
                            });
                        }
                    }
                }
                NetworkCommand::PublishProfile { display_name } => {
                    match publish_profile(&mut state, display_name).await {
                        Ok(()) => {
                            let _ = event_tx.send(AppEvent::ProfilePublished);
                        }
                        Err(e) => {
                            let _ = event_tx.send(AppEvent::NetworkError {
                                error: format!("Failed to publish profile: {}", e),
                            });
                        }
                    }
                }
                NetworkCommand::CreateGroup { name } => {
                    match create_group(&mut state, name).await {
                        Ok(group_id) => {
                            let _ = event_tx.send(AppEvent::GroupCreated { group_id });
                        }
                        Err(e) => {
                            let _ = event_tx.send(AppEvent::NetworkError {
                                error: format!("Failed to create group: {}", e),
                            });
                        }
                    }
                }
                NetworkCommand::JoinGroup { npub } => {
                    match join_group(&mut state, npub).await {
                        Ok(group_id) => {
                            let _ = event_tx.send(AppEvent::GroupCreated { group_id });
                        }
                        Err(e) => {
                            let _ = event_tx.send(AppEvent::NetworkError {
                                error: format!("Failed to join group: {}", e),
                            });
                        }
                    }
                }
                NetworkCommand::SendMessage { group_id, content } => {
                    match send_message(&mut state, group_id, content).await {
                        Ok(()) => {
                            // Message sent successfully
                        }
                        Err(e) => {
                            let _ = event_tx.send(AppEvent::NetworkError {
                                error: format!("Failed to send message: {}", e),
                            });
                        }
                    }
                }
                NetworkCommand::FetchMessages => {
                    match fetch_and_process_messages(&mut state, event_tx.clone()).await {
                        Ok(()) => {}
                        Err(e) => {
                            log::error!("Failed to fetch messages: {}", e);
                        }
                    }
                }
                NetworkCommand::FetchWelcomes => {
                    match fetch_and_process_welcomes(&mut state, event_tx.clone()).await {
                        Ok(()) => {}
                        Err(e) => {
                            log::error!("Failed to fetch welcomes: {}", e);
                        }
                    }
                }
            }
        }
    });
}

async fn publish_key_package(state: &mut NetworkState) -> Result<()> {
    let filter = Filter::new()
        .kind(Kind::from(443u16))
        .author(state.keys.public_key());
    state.client.subscribe(filter, None).await?;

    let relays = vec![
        RelayUrl::parse("wss://relay.damus.io")?,
        RelayUrl::parse("wss://nos.lol")?,
        RelayUrl::parse("wss://relay.nostr.band")?,
        RelayUrl::parse("wss://relay.snort.social")?,
        RelayUrl::parse("wss://nostr.wine")?,
    ];
    
    let (key_package_content, tags) = with_storage_mut!(state, create_key_package_for_event(&state.keys.public_key(), relays))?;

    let event = EventBuilder::new(Kind::from(443u16), key_package_content)
        .tags(tags)
        .build(state.keys.public_key())
        .sign(&state.keys)
        .await?;

    state.client.send_event(&event).await?;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let filter = Filter::new()
        .kind(Kind::GiftWrap)
        .pubkey(state.keys.public_key());
    state.client.subscribe(filter, None).await?;

    Ok(())
}

async fn publish_profile(state: &mut NetworkState, display_name: String) -> Result<()> {
    let metadata = Metadata::new()
        .display_name(display_name)
        .name("NRC User")
        .about("Secure messaging with MLS+Nostr");

    let builder = EventBuilder::metadata(&metadata);
    let event = builder.build(state.keys.public_key()).sign(&state.keys).await?;

    state.client.set_metadata(&metadata).await?;
    state.client.send_event(&event).await?;
    
    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(())
}

async fn create_group(state: &mut NetworkState, name: String) -> Result<GroupId> {
    let config = NostrGroupConfigData::new(
        name,
        "NRC Chat Group".to_string(),
        None,
        None,
        None,
        vec![RelayUrl::parse("wss://relay.damus.io")?],
        vec![state.keys.public_key()],
    );
    
    let group_result = with_storage_mut!(state, create_group(
        &state.keys.public_key(),
        vec![],
        config
    ))?;
    let group_id = GroupId::from_slice(group_result.group.mls_group_id.as_slice());
    
    state.groups.insert(group_id.clone(), group_result.group);
    
    Ok(group_id)
}

async fn join_group(state: &mut NetworkState, npub: String) -> Result<GroupId> {
    let pubkey = PublicKey::from_bech32(&npub)?;
    let key_package = state.fetch_key_package(&pubkey).await?;
    
    let config = NostrGroupConfigData::new(
        "Test Group".to_string(),
        "Test group for NRC".to_string(),
        None,
        None,
        None,
        vec![],
        vec![state.keys.public_key()],
    );

    let group_result = with_storage_mut!(state, create_group(
        &state.keys.public_key(),
        vec![key_package.clone()],
        config
    ))?;

    let group_id = GroupId::from_slice(group_result.group.mls_group_id.as_slice());
    state.groups.insert(group_id.clone(), group_result.group.clone());

    if let Some(welcome_rumor) = group_result.welcome_rumors.first() {
        let recipient_pubkey = key_package.pubkey;
        state.welcome_rumors.insert(recipient_pubkey, welcome_rumor.clone());
        state.send_gift_wrapped_welcome(recipient_pubkey, welcome_rumor.clone()).await?;
    }

    Ok(group_id)
}

async fn send_message(state: &mut NetworkState, group_id: GroupId, content: String) -> Result<()> {
    let message = with_storage_mut!(state, create_message(
        &state.keys.public_key(),
        group_id.as_slice().to_vec(),
        content.as_bytes().to_vec()
    ))?;
    
    let rumor = EventBuilder::new(Kind::from(444u16), message.rumor.rumor_content.clone())
        .tags(message.rumor.rumor_tags.clone())
        .build_unsigned(state.keys.public_key());

    let gift_wrap = EventBuilder::gift_wrap(&state.keys, &state.keys.public_key(), rumor, None)?;
    state.client.send_event(gift_wrap).await?;
    
    Ok(())
}

async fn fetch_and_process_messages(
    state: &mut NetworkState,
    event_tx: mpsc::UnboundedSender<AppEvent>,
) -> Result<()> {
    let filter = Filter::new()
        .kind(Kind::GiftWrap)
        .pubkey(state.keys.public_key())
        .since(Timestamp::now() - Duration::from_secs(60 * 60));

    let events = state
        .client
        .fetch_events(filter, Duration::from_secs(2))
        .await?;

    for event in events {
        if event.kind() != Kind::GiftWrap {
            continue;
        }

        match state.client.unwrap_gift_wrap(&state.keys, &event).await {
            Ok(unwrapped_gift) => {
                if unwrapped_gift.rumor.kind != Kind::from(444u16) {
                    continue;
                }

                match with_storage_mut!(state, process_message(
                    &state.keys.public_key(),
                    unwrapped_gift.rumor.content.clone(),
                    unwrapped_gift.rumor.tags
                )) {
                    Ok(msg) => {
                        if let Some(group_id) = msg.group_id {
                            let group_id = GroupId::from_slice(&group_id);
                            let message = Message {
                                content: String::from_utf8_lossy(&msg.message).to_string(),
                                sender: unwrapped_gift.sender,
                                timestamp: unwrapped_gift.rumor.created_at,
                            };
                            let _ = event_tx.send(AppEvent::MessageReceived { group_id, message });
                        }
                    }
                    Err(e) => {
                        log::debug!("Failed to process message: {}", e);
                    }
                }
            }
            Err(e) => {
                log::debug!("Failed to unwrap gift wrap: {}", e);
            }
        }
    }

    Ok(())
}

async fn fetch_and_process_welcomes(
    state: &mut NetworkState,
    event_tx: mpsc::UnboundedSender<AppEvent>,
) -> Result<()> {
    let filter = Filter::new()
        .kind(Kind::GiftWrap)
        .pubkey(state.keys.public_key())
        .since(Timestamp::now() - Duration::from_secs(60 * 60));

    let events = state
        .client
        .fetch_events(filter, Duration::from_secs(2))
        .await?;

    for event in events {
        if event.kind() != Kind::GiftWrap {
            continue;
        }

        match state.client.unwrap_gift_wrap(&state.keys, &event).await {
            Ok(unwrapped_gift) => {
                if unwrapped_gift.rumor.kind != Kind::from(444u16) {
                    continue;
                }

                if let Ok(result) = with_storage_mut!(state, process_welcome(
                    &state.keys.public_key(),
                    unwrapped_gift.rumor.content.clone(),
                    unwrapped_gift.rumor.tags
                )) {
                    let group_id = GroupId::from_slice(&result.group.mls_group_id);
                    state.groups.insert(group_id.clone(), result.group);
                    let _ = event_tx.send(AppEvent::GroupCreated { group_id });
                }
            }
            Err(e) => {
                log::debug!("Failed to unwrap gift wrap for welcome: {}", e);
            }
        }
    }

    Ok(())
}