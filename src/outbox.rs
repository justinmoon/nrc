use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    CreateGroup { name: String, members: Vec<PublicKey> },
    FetchKeyPackage { pubkey: PublicKey },
    SendWelcome { recipient: PublicKey, welcome: UnsignedEvent },
    ProcessMessage { group_id: Vec<u8>, message: Vec<u8> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandMetadata {
    pub created_at: u64,
    pub updated_at: u64,
    pub retry_count: u32,
    pub next_retry_at: Option<u64>,
    pub error: Option<String>,
    pub timeout_at: Option<u64>,
}

impl CommandMetadata {
    pub fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        Self {
            created_at: now,
            updated_at: now,
            retry_count: 0,
            next_retry_at: None,
            error: None,
            timeout_at: Some(now + 300), // 5 minute default timeout
        }
    }

    pub fn update_for_retry(&mut self, delay_secs: u64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.updated_at = now;
        self.retry_count += 1;
        self.next_retry_at = Some(now + delay_secs);
    }

    pub fn mark_completed(&mut self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.updated_at = now;
        self.next_retry_at = None;
        self.error = None;
    }

    pub fn mark_failed(&mut self, error: String) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.updated_at = now;
        self.error = Some(error);
    }

    pub fn is_ready_for_retry(&self) -> bool {
        if let Some(next_retry) = self.next_retry_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            now >= next_retry
        } else {
            false
        }
    }

    pub fn is_timed_out(&self) -> bool {
        if let Some(timeout) = self.timeout_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            now >= timeout
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
pub struct PersistentCommand {
    pub id: Uuid,
    pub command: Command,
    pub status: CommandStatus,
    pub metadata: CommandMetadata,
    pub dependencies: Vec<Uuid>,
}

impl PersistentCommand {
    pub fn new(command: Command) -> Self {
        Self {
            id: Uuid::new_v4(),
            command,
            status: CommandStatus::Pending,
            metadata: CommandMetadata::new(),
            dependencies: Vec::new(),
        }
    }

    pub fn with_dependencies(command: Command, dependencies: Vec<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            command,
            status: CommandStatus::Pending,
            metadata: CommandMetadata::new(),
            dependencies,
        }
    }

    pub fn exponential_backoff_delay(&self) -> u64 {
        // Start with 1 second, double each retry, cap at 60 seconds
        let base_delay = 1u64;
        let max_delay = 60u64;
        let delay = base_delay * (2u64.pow(self.metadata.retry_count.min(6)));
        delay.min(max_delay)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvent {
    pub id: u64,
    pub task_id: Uuid,
    pub event_type: String,
    pub timestamp: u64,
    pub data: Option<Vec<u8>>,
}

impl TaskEvent {
    pub fn new(task_id: Uuid, event_type: String, data: Option<Vec<u8>>) -> Self {
        Self {
            id: 0, // Will be set by database
            task_id,
            event_type,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            data,
        }
    }
}