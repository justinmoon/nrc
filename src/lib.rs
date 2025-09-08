use crate::config::get_default_relays;
use crate::key_storage::KeyStorage;
use anyhow::Result;
use nostr_sdk::prelude::*;
use nrc_mls::NostrMls;
use nrc_mls_sqlite_storage::NostrMlsSqliteStorage;
use nrc_mls_storage::groups::types as group_types;
use openmls::group::GroupId;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;

// Module declarations
pub mod commands;
pub mod config;
pub mod groups;
pub mod key_storage;
pub mod messages;
pub mod network;
pub mod network_task;
pub mod notification_handler;
pub mod profiles;
pub mod state;
pub mod types;
pub mod utils;

// Re-export commonly used types
pub use config::DEFAULT_RELAYS;
pub use types::{AppEvent, AppState, Message, NetworkCommand, OnboardingData, OnboardingMode};
pub use utils::pubkey_to_bech32_safe;

pub struct Nrc {
    pub(crate) storage: Box<NostrMls<NostrMlsSqliteStorage>>,
    pub keys: Keys,
    pub client: Client,
    pub state: types::AppState,
    pub(crate) messages: HashMap<GroupId, Vec<types::Message>>,
    pub welcome_rumors: HashMap<PublicKey, UnsignedEvent>,
    pub groups: HashMap<GroupId, group_types::Group>,
    pub input: String,
    pub selected_group_index: Option<usize>,
    pub scroll_offset: u16,
    pub last_error: Option<String>,
    pub flash_message: Option<String>,
    pub show_help: bool,
    pub help_explicitly_requested: bool,
    pub(crate) profiles: HashMap<PublicKey, Metadata>,
    pub event_tx: Option<mpsc::UnboundedSender<types::AppEvent>>,
    pub command_tx: Option<mpsc::Sender<types::NetworkCommand>>,
    pub(crate) key_storage: KeyStorage,
    pub onboarding_data: types::OnboardingData,
}

impl Nrc {
    pub async fn new(datadir: &Path) -> Result<Self> {
        // Create datadir if it doesn't exist
        std::fs::create_dir_all(datadir)?;

        let key_storage = KeyStorage::new(datadir);

        // Check if we have existing keys
        let (keys, initial_state) = if key_storage.keys_exist() {
            // We have stored keys, prompt for password
            let keys = Keys::generate(); // Temporary, will be replaced when password entered
            (
                keys,
                types::AppState::Onboarding {
                    input: String::new(),
                    mode: types::OnboardingMode::EnterPassword,
                },
            )
        } else {
            // No stored keys, show regular onboarding
            let keys = Keys::generate();
            (
                keys,
                types::AppState::Onboarding {
                    input: String::new(),
                    mode: types::OnboardingMode::Choose,
                },
            )
        };

        let client = Client::builder().signer(keys.clone()).build();

        // Add multiple relays for redundancy
        for &relay in get_default_relays() {
            if let Err(e) = client.add_relay(relay).await {
                log::warn!("Failed to add relay {relay}: {e}");
            }
        }

        client.connect().await;

        // Wait for connections to establish
        tokio::time::sleep(Duration::from_secs(2)).await;

        let db_path = datadir.join("nrc.db");
        log::info!("Using SQLite storage at: {db_path:?}");
        let storage = Box::new(NostrMls::new(NostrMlsSqliteStorage::new(db_path)?));

        Ok(Self {
            storage,
            keys,
            client,
            state: initial_state,
            messages: HashMap::new(),
            welcome_rumors: HashMap::new(),
            groups: HashMap::new(),
            input: String::new(),
            selected_group_index: None,
            scroll_offset: 0,
            last_error: None,
            flash_message: None,
            show_help: false,
            help_explicitly_requested: false,
            profiles: HashMap::new(),
            event_tx: None,
            command_tx: None,
            key_storage,
            onboarding_data: types::OnboardingData {
                display_name: None,
                nsec: None,
            },
        })
    }

    pub async fn handle_storage_command(
        &mut self,
        command: network_task::StorageCommand,
    ) -> network_task::StorageResponse {
        use network_task::{StorageCommand, StorageResponse};
        use nrc_mls::groups::NostrGroupConfigData;

        match command {
            StorageCommand::CreateMessage { group_id, rumor } => {
                match self.storage.create_message(&group_id, rumor) {
                    Ok(event) => StorageResponse::MessageCreated(event),
                    Err(e) => StorageResponse::Error(e.to_string()),
                }
            }
            StorageCommand::GetGroup { group_id } => {
                StorageResponse::Group(self.groups.get(&group_id).cloned())
            }
            StorageCommand::GetGroups => StorageResponse::Groups(self.groups.clone()),
            StorageCommand::CreateGroup { name, keys } => {
                let config = NostrGroupConfigData::new(
                    name,
                    "NRC Chat Group".to_string(),
                    None,
                    None,
                    None,
                    vec![get_default_relays()[0].parse().unwrap()],
                    vec![keys.public_key()],
                );

                match self
                    .storage
                    .create_group(&keys.public_key(), vec![], config.clone())
                {
                    Ok(group_result) => {
                        let group_id =
                            GroupId::from_slice(group_result.group.mls_group_id.as_slice());

                        // Store group in our local map
                        self.groups
                            .insert(group_id.clone(), group_result.group.clone());

                        // Create welcome rumor if there is one
                        let welcome_rumor = if let Some(rumor) = group_result.welcome_rumors.first()
                        {
                            rumor.clone()
                        } else {
                            // Create a dummy welcome rumor for solo groups
                            EventBuilder::new(Kind::from(442u16), "solo_group")
                                .build(keys.public_key())
                        };

                        StorageResponse::GroupCreated {
                            group_id,
                            welcome_rumor,
                        }
                    }
                    Err(e) => StorageResponse::Error(e.to_string()),
                }
            }
            StorageCommand::JoinGroupFromWelcome {
                welcome_rumor: _,
                keys: _,
            } => {
                // For now, just return an error since join_group is more complex
                StorageResponse::Error(
                    "Join group not yet implemented in storage handler".to_string(),
                )
            }
            StorageCommand::MergeGroupConfig { config: _ } => {
                // merge_group_config doesn't exist, return error for now
                StorageResponse::Error("Merge group config not yet implemented".to_string())
            }
            StorageCommand::GetKeyPackage { keys } => {
                // Create the key package event
                let relays: Result<Vec<RelayUrl>, _> = get_default_relays()
                    .iter()
                    .map(|&url| RelayUrl::parse(url))
                    .collect();

                match relays {
                    Ok(relay_urls) => {
                        match self
                            .storage
                            .create_key_package_for_event(&keys.public_key(), relay_urls)
                        {
                            Ok((key_package_content, tags)) => {
                                match EventBuilder::new(Kind::from(443u16), key_package_content)
                                    .tags(tags)
                                    .build(keys.public_key())
                                    .sign(&keys)
                                    .await
                                {
                                    Ok(event) => StorageResponse::KeyPackage(event),
                                    Err(e) => StorageResponse::Error(e.to_string()),
                                }
                            }
                            Err(e) => StorageResponse::Error(e.to_string()),
                        }
                    }
                    Err(e) => StorageResponse::Error(e.to_string()),
                }
            }
            StorageCommand::ProcessMessageEvent { event } => {
                // Process message using the process_message method
                match self.storage.process_message(&event) {
                    Ok(result) => {
                        use nrc_mls::messages::MessageProcessingResult;
                        match result {
                            MessageProcessingResult::ApplicationMessage(msg) => {
                                if msg.kind == Kind::TextNote {
                                    if let Ok(Some(stored_msg)) = self.storage.get_message(&msg.id)
                                    {
                                        let message = crate::Message {
                                            content: stored_msg.content.clone(),
                                            sender: stored_msg.pubkey,
                                            timestamp: stored_msg.created_at,
                                        };
                                        return StorageResponse::MessageProcessed(Some(message));
                                    }
                                }
                                StorageResponse::MessageProcessed(None)
                            }
                            _ => StorageResponse::MessageProcessed(None),
                        }
                    }
                    Err(e) => StorageResponse::Error(e.to_string()),
                }
            }
        }
    }
}
