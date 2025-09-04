use anyhow::Result;
use nrc::outbox::{Command, CommandStatus, PersistentCommand};
use nrc::command_store::SqliteCommandStore;
use nrc::command_executor::CommandExecutor;
use nrc::scheduler::BackgroundScheduler;
use nostr_sdk::prelude::*;
use std::path::PathBuf;
use uuid::Uuid;

#[tokio::test]
async fn test_command_persistence_and_retrieval() -> Result<()> {
    let temp_dir = std::env::temp_dir().join("nrc_state_machine_test");
    std::fs::create_dir_all(&temp_dir)?;
    
    let db_path = temp_dir.join("test_commands.db");
    let mut store = SqliteCommandStore::new(&db_path)?;
    
    // Create a test command
    let keys = Keys::generate();
    let pubkey = keys.public_key();
    
    let command = PersistentCommand::new(Command::FetchKeyPackage { pubkey });
    let command_id = command.id;
    
    // Store the command
    store.store_command(&command)?;
    
    // Retrieve the command
    let retrieved = store.get_command(&command_id)?;
    assert!(retrieved.is_some());
    
    let retrieved_cmd = retrieved.unwrap();
    assert_eq!(retrieved_cmd.id, command_id);
    assert!(matches!(retrieved_cmd.status, CommandStatus::Pending));
    
    if let Command::FetchKeyPackage { pubkey: retrieved_pubkey } = retrieved_cmd.command {
        assert_eq!(retrieved_pubkey, pubkey);
    } else {
        panic!("Wrong command type retrieved");
    }
    
    // Test getting pending commands
    let pending = store.get_pending_commands()?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, command_id);
    
    // Update status
    store.update_command_status(&command_id, CommandStatus::InProgress)?;
    
    let updated = store.get_command(&command_id)?.unwrap();
    assert!(matches!(updated.status, CommandStatus::InProgress));
    
    // Test cleanup
    std::fs::remove_dir_all(&temp_dir)?;
    
    Ok(())
}

#[tokio::test]
async fn test_command_dependencies_and_workflow() -> Result<()> {
    let temp_dir = std::env::temp_dir().join("nrc_workflow_test");
    std::fs::create_dir_all(&temp_dir)?;
    
    let db_path = temp_dir.join("test_workflow.db");
    let mut store = SqliteCommandStore::new(&db_path)?;
    
    // Create first command (fetch key package)
    let keys = Keys::generate();
    let pubkey = keys.public_key();
    
    let fetch_cmd = PersistentCommand::new(Command::FetchKeyPackage { pubkey });
    let fetch_id = fetch_cmd.id;
    store.store_command(&fetch_cmd)?;
    
    // Create second command that depends on the first
    let create_cmd = PersistentCommand::with_dependencies(
        Command::CreateGroup {
            name: "Test Group".to_string(),
            members: vec![pubkey],
        },
        vec![fetch_id],
    );
    let create_id = create_cmd.id;
    store.store_command(&create_cmd)?;
    
    // Verify dependency relationship
    let retrieved_create = store.get_command(&create_id)?.unwrap();
    assert_eq!(retrieved_create.dependencies.len(), 1);
    assert_eq!(retrieved_create.dependencies[0], fetch_id);
    
    // Test that pending commands returns both
    let pending = store.get_pending_commands()?;
    assert_eq!(pending.len(), 2);
    
    // Complete the first command
    store.update_command_status(&fetch_id, CommandStatus::Completed)?;
    
    // Verify status change
    let completed_fetch = store.get_command(&fetch_id)?.unwrap();
    assert!(matches!(completed_fetch.status, CommandStatus::Completed));
    
    // The create command should still be pending
    let pending_create = store.get_command(&create_id)?.unwrap();
    assert!(matches!(pending_create.status, CommandStatus::Pending));
    
    // Test cleanup
    std::fs::remove_dir_all(&temp_dir)?;
    
    Ok(())
}

#[tokio::test]
async fn test_exponential_backoff_calculation() -> Result<()> {
    let keys = Keys::generate();
    let pubkey = keys.public_key();
    
    let mut command = PersistentCommand::new(Command::FetchKeyPackage { pubkey });
    
    // Test exponential backoff calculation
    assert_eq!(command.exponential_backoff_delay(), 1); // First attempt: 1 second
    
    command.metadata.retry_count = 1;
    assert_eq!(command.exponential_backoff_delay(), 2); // Second attempt: 2 seconds
    
    command.metadata.retry_count = 2;
    assert_eq!(command.exponential_backoff_delay(), 4); // Third attempt: 4 seconds
    
    command.metadata.retry_count = 3;
    assert_eq!(command.exponential_backoff_delay(), 8); // Fourth attempt: 8 seconds
    
    command.metadata.retry_count = 10; // Very high retry count
    assert_eq!(command.exponential_backoff_delay(), 60); // Should cap at 60 seconds
    
    Ok(())
}

#[tokio::test]
async fn test_scheduler_metrics_and_status_tracking() -> Result<()> {
    let temp_dir = std::env::temp_dir().join("nrc_scheduler_test");
    std::fs::create_dir_all(&temp_dir)?;
    
    let db_path = temp_dir.join("test_scheduler.db");
    let store = SqliteCommandStore::new(&db_path)?;
    let executor = CommandExecutor::new(store);
    let mut scheduler = BackgroundScheduler::new(executor);
    
    // Submit a command
    let keys = Keys::generate();
    let pubkey = keys.public_key();
    let command = PersistentCommand::new(Command::FetchKeyPackage { pubkey });
    let command_id = scheduler.submit_command(command)?;
    
    // Verify it was submitted
    let status = scheduler.get_command_status(&command_id)?;
    assert!(matches!(status, Some(CommandStatus::Pending)));
    
    // Check metrics
    let metrics = scheduler.get_metrics();
    assert_eq!(metrics.commands_processed, 0); // No processing done yet
    
    // Test cleanup
    std::fs::remove_dir_all(&temp_dir)?;
    
    Ok(())
}

#[tokio::test]
async fn test_command_event_logging() -> Result<()> {
    let temp_dir = std::env::temp_dir().join("nrc_events_test");
    std::fs::create_dir_all(&temp_dir)?;
    
    let db_path = temp_dir.join("test_events.db");
    let mut store = SqliteCommandStore::new(&db_path)?;
    
    // Create a command
    let keys = Keys::generate();
    let pubkey = keys.public_key();
    let command = PersistentCommand::new(Command::FetchKeyPackage { pubkey });
    let command_id = command.id;
    
    store.store_command(&command)?;
    
    // Add some events
    let event1 = nrc::outbox::TaskEvent::new(
        command_id,
        "started".to_string(),
        None,
    );
    store.add_event(&event1)?;
    
    let event2 = nrc::outbox::TaskEvent::new(
        command_id,
        "retry_scheduled".to_string(),
        Some(b"Network timeout".to_vec()),
    );
    store.add_event(&event2)?;
    
    // Retrieve events
    let events = store.get_events_for_task(&command_id)?;
    assert_eq!(events.len(), 2);
    
    assert_eq!(events[0].event_type, "started");
    assert_eq!(events[1].event_type, "retry_scheduled");
    assert!(events[1].data.is_some());
    
    // Test cleanup
    std::fs::remove_dir_all(&temp_dir)?;
    
    Ok(())
}

/// This test demonstrates the key benefit: operations are resumable after app restart
#[tokio::test]
async fn test_resumable_operations_after_restart() -> Result<()> {
    let temp_dir = std::env::temp_dir().join("nrc_resumable_test");
    std::fs::create_dir_all(&temp_dir)?;
    
    let db_path = temp_dir.join("test_resumable.db");
    
    let command_id: Uuid;
    
    // Simulate first app instance
    {
        let mut store = SqliteCommandStore::new(&db_path)?;
        
        let keys = Keys::generate();
        let pubkey = keys.public_key();
        let command = PersistentCommand::new(Command::FetchKeyPackage { pubkey });
        command_id = command.id;
        
        // Store command and mark as in-progress (simulating partial completion)
        store.store_command(&command)?;
        store.update_command_status(&command_id, CommandStatus::InProgress)?;
        
        let event = nrc::outbox::TaskEvent::new(
            command_id,
            "started".to_string(),
            None,
        );
        store.add_event(&event)?;
    } // Store goes out of scope, simulating app shutdown
    
    // Simulate second app instance (restart)
    {
        let store = SqliteCommandStore::new(&db_path)?;
        
        // Command should still exist
        let retrieved = store.get_command(&command_id)?;
        assert!(retrieved.is_some());
        
        let command = retrieved.unwrap();
        assert!(matches!(command.status, CommandStatus::InProgress));
        
        // Events should be preserved
        let events = store.get_events_for_task(&command_id)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "started");
        
        println!("✅ Successfully resumed command {} after 'restart'", command_id);
    }
    
    // Test cleanup
    std::fs::remove_dir_all(&temp_dir)?;
    
    Ok(())
}