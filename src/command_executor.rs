use anyhow::Result;
use nostr_mls::groups::NostrGroupConfigData;
use nostr_sdk::prelude::*;
use std::time::Duration;
use uuid::Uuid;

use crate::network_task::NetworkState;
use crate::outbox::{Command, CommandStatus, PersistentCommand, TaskEvent};
use crate::command_store::SqliteCommandStore;
use crate::{with_storage_mut, Storage};

pub struct CommandExecutor {
    store: SqliteCommandStore,
}

impl CommandExecutor {
    pub fn new(store: SqliteCommandStore) -> Self {
        Self { store }
    }

    pub async fn execute_command(
        &mut self,
        command: &PersistentCommand,
        state: &mut NetworkState,
    ) -> Result<CommandResult> {
        // Mark command as in progress
        self.store.update_command_status(&command.id, CommandStatus::InProgress)?;
        self.store.add_event(&TaskEvent::new(
            command.id,
            "started".to_string(),
            None,
        ))?;

        let result = match &command.command {
            Command::FetchKeyPackage { pubkey } => {
                self.execute_fetch_key_package(command, state, pubkey).await
            }
            Command::CreateGroup { name, members } => {
                self.execute_create_group(command, state, name, members).await
            }
            Command::SendWelcome { recipient, welcome } => {
                self.execute_send_welcome(command, state, recipient, welcome).await
            }
            Command::ProcessMessage { group_id, message } => {
                self.execute_process_message(command, state, group_id, message).await
            }
        };

        match &result {
            Ok(CommandResult::Success { data }) => {
                self.store.update_command_status(&command.id, CommandStatus::Completed)?;
                self.store.add_event(&TaskEvent::new(
                    command.id,
                    "completed".to_string(),
                    data.clone(),
                ))?;
            }
            Ok(CommandResult::Retry { delay_secs, error }) => {
                // Update retry metadata but keep status as pending for retry
                let mut updated_command = command.clone();
                updated_command.metadata.update_for_retry(*delay_secs);
                updated_command.metadata.mark_failed(error.clone());
                self.store.store_command(&updated_command)?;
                self.store.add_event(&TaskEvent::new(
                    command.id,
                    "retry_scheduled".to_string(),
                    Some(serde_json::to_vec(&format!("Retry in {}s: {}", delay_secs, error))?),
                ))?;
            }
            Err(e) => {
                let mut updated_command = command.clone();
                updated_command.status = CommandStatus::Failed;
                updated_command.metadata.mark_failed(e.to_string());
                self.store.store_command(&updated_command)?;
                self.store.add_event(&TaskEvent::new(
                    command.id,
                    "failed".to_string(),
                    Some(serde_json::to_vec(&e.to_string())?),
                ))?;
            }
        }

        result
    }

    async fn execute_fetch_key_package(
        &mut self,
        command: &PersistentCommand,
        state: &mut NetworkState,
        pubkey: &PublicKey,
    ) -> Result<CommandResult> {
        let filter = Filter::new()
            .kind(Kind::from(443u16))
            .author(*pubkey)
            .limit(1);

        // Subscribe to ensure we can fetch events
        state.client.subscribe(filter.clone(), None).await?;

        // Try to fetch from relay without sleep - the state machine handles retries
        match state
            .client
            .fetch_events(filter.clone(), Duration::from_secs(5))
            .await
        {
            Ok(events) if !events.is_empty() => {
                let event = events.into_iter().next().unwrap();
                log::debug!(
                    "Successfully fetched key package for {} on attempt {}",
                    pubkey,
                    command.metadata.retry_count + 1
                );
                Ok(CommandResult::Success {
                    data: Some(serde_json::to_vec(&event)?),
                })
            }
            Ok(_) => {
                // No events found, schedule retry with exponential backoff
                let delay = command.exponential_backoff_delay();
                Ok(CommandResult::Retry {
                    delay_secs: delay,
                    error: format!(
                        "Key package not found for {} (attempt {})",
                        pubkey,
                        command.metadata.retry_count + 1
                    ),
                })
            }
            Err(e) => {
                // Network error, schedule retry
                let delay = command.exponential_backoff_delay();
                Ok(CommandResult::Retry {
                    delay_secs: delay,
                    error: format!("Network error fetching key package: {}", e),
                })
            }
        }
    }

    async fn execute_create_group(
        &mut self,
        _command: &PersistentCommand,
        state: &mut NetworkState,
        name: &str,
        _members: &[PublicKey],
    ) -> Result<CommandResult> {
        // This assumes we have the key packages for all members already
        // In practice, this would be preceded by FetchKeyPackage commands
        
        let config = NostrGroupConfigData::new(
            name.to_string(),
            "NRC Chat Group".to_string(),
            None,
            None,
            None,
            vec![RelayUrl::parse("wss://relay.damus.io")?],
            vec![state.keys.public_key()],
        );

        // For now, create empty group. In full implementation, we'd fetch key packages first
        let key_packages = Vec::new(); // This would be populated from successful FetchKeyPackage commands
        
        let group_result = with_storage_mut!(
            state,
            create_group(&state.keys.public_key(), key_packages, config)
        )?;

        let group_id = openmls::group::GroupId::from_slice(group_result.group.mls_group_id.as_slice());
        state.groups.insert(group_id.clone(), group_result.group);

        log::info!("Created group '{}' with ID: {}", name, hex::encode(group_id.as_slice()));
        
        Ok(CommandResult::Success {
            data: Some(serde_json::to_vec(&group_id.as_slice().to_vec())?),
        })
    }

    async fn execute_send_welcome(
        &mut self,
        _command: &PersistentCommand,
        state: &mut NetworkState,
        recipient: &PublicKey,
        welcome_rumor: &UnsignedEvent,
    ) -> Result<CommandResult> {
        match EventBuilder::gift_wrap(&state.keys, recipient, welcome_rumor.clone(), None).await {
            Ok(gift_wrap) => {
                match state.client.send_event(&gift_wrap).await {
                    Ok(_) => {
                        log::info!("Successfully sent welcome to {}", recipient);
                        Ok(CommandResult::Success { data: None })
                    }
                    Err(e) => Err(e.into()),
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn execute_process_message(
        &mut self,
        _command: &PersistentCommand,
        _state: &mut NetworkState,
        _group_id: &[u8],
        _message: &[u8],
    ) -> Result<CommandResult> {
        // Placeholder for message processing logic
        // This would decrypt and process MLS messages
        Ok(CommandResult::Success { data: None })
    }

    pub fn get_commands_ready_for_execution(&self) -> Result<Vec<PersistentCommand>> {
        self.store.get_pending_commands()
    }

    pub fn cleanup_old_completed_tasks(&mut self) -> Result<usize> {
        // Clean up completed tasks older than 24 hours
        self.store.cleanup_completed_tasks(24 * 60 * 60)
    }

    pub fn get_command(&self, id: &Uuid) -> Result<Option<PersistentCommand>> {
        self.store.get_command(id)
    }

    pub fn get_failed_commands(&self) -> Result<Vec<PersistentCommand>> {
        self.store.get_commands_by_status(CommandStatus::Failed)
    }

    pub fn retry_failed_command(&mut self, id: &Uuid) -> Result<()> {
        if let Some(mut command) = self.store.get_command(id)? {
            command.status = CommandStatus::Pending;
            command.metadata.retry_count = 0;
            command.metadata.next_retry_at = None;
            command.metadata.error = None;
            self.store.store_command(&command)?;
        }
        Ok(())
    }

    pub fn store_command(&mut self, command: &PersistentCommand) -> Result<()> {
        self.store.store_command(command)
    }

    pub fn add_event(&mut self, event: &TaskEvent) -> Result<()> {
        self.store.add_event(event)
    }

    pub fn get_events_for_task(&self, id: &Uuid) -> Result<Vec<TaskEvent>> {
        self.store.get_events_for_task(id)
    }
}

#[derive(Debug, Clone)]
pub enum CommandResult {
    Success { data: Option<Vec<u8>> },
    Retry { delay_secs: u64, error: String },
}