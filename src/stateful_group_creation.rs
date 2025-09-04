use anyhow::Result;
use nostr_sdk::prelude::*;
use uuid::Uuid;

use crate::command_executor::CommandExecutor;
use crate::command_store::SqliteCommandStore;
use crate::outbox::{Command, PersistentCommand};
use crate::scheduler::BackgroundScheduler;

/// Stateful group creation that replaces the sleep-based approach
/// This demonstrates how to use the command system for reliable group creation
pub struct StatefulGroupCreation {
    scheduler: BackgroundScheduler,
}

impl StatefulGroupCreation {
    pub fn new(db_path: &std::path::Path) -> Result<Self> {
        let store = SqliteCommandStore::new(db_path.join("commands.db"))?;
        let executor = CommandExecutor::new(store);
        let scheduler = BackgroundScheduler::new(executor);
        
        Ok(Self { scheduler })
    }

    /// Create a group with a member using the command system
    /// This replaces the original join_group function with stateful operations
    pub async fn create_group_with_member(&mut self, pubkey: PublicKey) -> Result<Uuid> {
        // Step 1: Create command to fetch key package
        let fetch_cmd = PersistentCommand::new(Command::FetchKeyPackage { pubkey });
        let fetch_id = self.scheduler.submit_command(fetch_cmd)?;

        // Step 2: Create command to create group (depends on key package fetch)
        let create_cmd = PersistentCommand::with_dependencies(
            Command::CreateGroup {
                name: "Test Group".to_string(),
                members: vec![pubkey],
            },
            vec![fetch_id],
        );
        let create_id = self.scheduler.submit_command(create_cmd)?;

        log::info!("Submitted commands for group creation with {}", pubkey);
        log::info!("  - Fetch key package: {}", fetch_id);
        log::info!("  - Create group: {}", create_id);

        Ok(create_id)
    }

    /// Monitor the progress of a group creation workflow
    pub fn get_workflow_status(&self, workflow_id: &Uuid) -> Result<WorkflowStatus> {
        match self.scheduler.get_command_status(workflow_id)? {
            Some(status) => Ok(WorkflowStatus::from_command_status(status)),
            None => Ok(WorkflowStatus::NotFound),
        }
    }

    /// Get detailed information about what went wrong (for debugging)
    pub fn get_workflow_events(&self, workflow_id: &Uuid) -> Result<Vec<crate::outbox::TaskEvent>> {
        self.scheduler.get_command_events(workflow_id)
    }

    /// Get all failed workflows for debugging
    pub fn get_failed_workflows(&self) -> Result<Vec<PersistentCommand>> {
        self.scheduler.get_failed_commands()
    }

    /// Retry a failed workflow
    pub fn retry_workflow(&mut self, workflow_id: &Uuid) -> Result<()> {
        self.scheduler.retry_command(workflow_id)
    }

    /// Get metrics about the scheduler performance
    pub fn get_metrics(&self) -> &crate::scheduler::SchedulerMetrics {
        self.scheduler.get_metrics()
    }
}

#[derive(Debug, Clone)]
pub enum WorkflowStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    NotFound,
}

impl WorkflowStatus {
    fn from_command_status(status: crate::outbox::CommandStatus) -> Self {
        match status {
            crate::outbox::CommandStatus::Pending => Self::Pending,
            crate::outbox::CommandStatus::InProgress => Self::InProgress,
            crate::outbox::CommandStatus::Completed => Self::Completed,
            crate::outbox::CommandStatus::Failed => Self::Failed,
        }
    }
}

/// Example usage demonstrating how to replace the old sleep-based approach
#[cfg(test)]
mod example_usage {
    use super::*;
    
    #[tokio::test]
    async fn example_stateful_group_creation() -> Result<()> {
        let temp_dir = std::env::temp_dir().join("nrc_test_commands");
        std::fs::create_dir_all(&temp_dir)?;
        
        let mut creator = StatefulGroupCreation::new(&temp_dir)?;
        
        // Generate a test pubkey
        let keys = Keys::generate();
        let pubkey = keys.public_key();
        
        // Start the workflow
        let workflow_id = creator.create_group_with_member(pubkey).await?;
        
        // In a real application, the scheduler would be running in the background
        // For this test, we just verify the commands were created
        let status = creator.get_workflow_status(&workflow_id)?;
        assert!(matches!(status, WorkflowStatus::Pending | WorkflowStatus::InProgress));
        
        // Check the events
        let events = creator.get_workflow_events(&workflow_id)?;
        assert!(!events.is_empty());
        
        println!("Created workflow {} with status {:?}", workflow_id, status);
        println!("Events: {:?}", events);
        
        Ok(())
    }
}

/// Documentation for migrating from the old approach
/// 
/// BEFORE (sleep-based approach):
/// ```rust
/// pub async fn join_group(state: &mut NetworkState, npub: String) -> Result<GroupId> {
///     let pubkey = PublicKey::from_bech32(&npub)?;
///     let key_package = state.fetch_key_package(&pubkey).await?; // <- Contains sleeps and retries
///     
///     let config = NostrGroupConfigData::new(/* ... */);
///     let group_result = with_storage_mut!(state, create_group(
///         &state.keys.public_key(),
///         vec![key_package.clone()],
///         config
///     ))?;
///     
///     let group_id = GroupId::from_slice(group_result.group.mls_group_id.as_slice());
///     state.groups.insert(group_id.clone(), group_result.group.clone());
/// 
///     if let Some(welcome_rumor) = group_result.welcome_rumors.first() {
///         let recipient_pubkey = key_package.pubkey;
///         state.welcome_rumors.insert(recipient_pubkey, welcome_rumor.clone());
///         state.send_gift_wrapped_welcome(recipient_pubkey, welcome_rumor.clone()).await?; // <- More sleeps
///     }
/// 
///     Ok(group_id)
/// }
/// ```
/// 
/// AFTER (command-based approach):
/// ```rust
/// pub async fn join_group_stateful(
///     creator: &mut StatefulGroupCreation, 
///     npub: String
/// ) -> Result<Uuid> {
///     let pubkey = PublicKey::from_bech32(&npub)?;
///     let workflow_id = creator.create_group_with_member(pubkey).await?;
///     
///     // The workflow runs asynchronously in the background
///     // No blocking, no sleeps, automatic retries with exponential backoff
///     // Full observability and recovery from failures
///     
///     Ok(workflow_id)
/// }
/// ```
/// 
/// KEY BENEFITS:
/// - No blocking operations - UI remains responsive
/// - Automatic retry with exponential backoff
/// - Persistent state - survives app restarts  
/// - Full observability - can query status and debug failures
/// - Composable - easy to add new steps to workflows
/// - Testable - can simulate failures and verify retry behavior
///
/// This concludes the migration guide for the state machine implementation.
pub struct MigrationGuide;

impl MigrationGuide {}