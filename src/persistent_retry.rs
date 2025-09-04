use anyhow::Result;
use nostr_sdk::prelude::*;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PersistentOp {
    FetchKeyPackage {
        pubkey: PublicKey,
    },
    SendMessage {
        group_id: Vec<u8>,
        content: String,
    },
    JoinGroup {
        pubkey: PublicKey,
        group_name: String,
    },
    SendWelcome {
        recipient: PublicKey,
        welcome_event_json: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SuccessEvent {
    MessageSent {
        group_id: Vec<u8>,
        timestamp: u64,
    },
    GroupJoined {
        group_id: Vec<u8>,
        group_name: String,
    },
    WelcomeSent {
        recipient: PublicKey,
    },
    KeyPackageFetched {
        pubkey: PublicKey,
    },
}

pub struct PersistentRetryQueue {
    conn: Connection,
}

impl PersistentRetryQueue {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS persistent_operations (
                id TEXT PRIMARY KEY,
                operation_type TEXT NOT NULL,
                params TEXT NOT NULL,
                success_event TEXT,
                retry_count INTEGER DEFAULT 0,
                next_retry_at INTEGER,
                created_at INTEGER NOT NULL,
                status TEXT DEFAULT 'pending',
                error TEXT
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pending_ops ON persistent_operations(status, next_retry_at) 
             WHERE status = 'pending'",
            [],
        )?;

        Ok(Self { conn })
    }

    pub fn enqueue(
        &mut self,
        op: PersistentOp,
        success_event: Option<SuccessEvent>,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;

        let op_type = match &op {
            PersistentOp::FetchKeyPackage { .. } => "fetch_key_package",
            PersistentOp::SendMessage { .. } => "send_message",
            PersistentOp::JoinGroup { .. } => "join_group",
            PersistentOp::SendWelcome { .. } => "send_welcome",
        };

        let params = serde_json::to_string(&op)?;
        let event_json = success_event
            .map(|e| serde_json::to_string(&e))
            .transpose()?;

        self.conn.execute(
            "INSERT INTO persistent_operations 
             (id, operation_type, params, success_event, created_at, next_retry_at) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id.to_string(), op_type, params, event_json, now,],
        )?;

        log::info!("Queued persistent operation: {} ({})", op_type, id);
        Ok(id)
    }

    pub fn get_pending_operations(&self) -> Result<Vec<(Uuid, PersistentOp, u32)>> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;

        let mut stmt = self.conn.prepare(
            "SELECT id, params, retry_count FROM persistent_operations 
             WHERE status = 'pending' 
             AND (next_retry_at IS NULL OR next_retry_at <= ?1)
             ORDER BY created_at ASC",
        )?;

        let ops = stmt.query_map(params![now], |row| {
            let id_str: String = row.get(0)?;
            let id = Uuid::parse_str(&id_str).unwrap();
            let params_json: String = row.get(1)?;
            let op: PersistentOp = serde_json::from_str(&params_json).unwrap();
            let retry_count: u32 = row.get(2)?;
            Ok((id, op, retry_count))
        })?;

        ops.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn complete(&mut self, id: &Uuid) -> Result<Option<SuccessEvent>> {
        // First get the success event if any
        let event_json: Option<String> = self
            .conn
            .query_row(
                "SELECT success_event FROM persistent_operations WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        // Mark as completed
        self.conn.execute(
            "UPDATE persistent_operations SET status = 'completed' WHERE id = ?1",
            params![id.to_string()],
        )?;

        log::info!("Completed persistent operation: {}", id);

        // Decode and return event
        event_json
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(Into::into)
    }

    pub fn schedule_retry(&mut self, id: &Uuid, retry_count: u32) -> Result<()> {
        // Exponential backoff: 1s, 2s, 4s, 8s, 16s, 32s, then every 60s
        let delay_secs = (2u64.pow(retry_count.min(5))).min(60);
        let next_retry = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + delay_secs;

        self.conn.execute(
            "UPDATE persistent_operations 
             SET retry_count = ?1, next_retry_at = ?2 
             WHERE id = ?3",
            params![retry_count + 1, next_retry as i64, id.to_string()],
        )?;

        log::debug!(
            "Scheduled retry for {} in {}s (attempt #{})",
            id,
            delay_secs,
            retry_count + 1
        );
        Ok(())
    }

    pub fn mark_failed(&mut self, id: &Uuid, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE persistent_operations 
             SET status = 'failed', error = ?1 
             WHERE id = ?2",
            params![error, id.to_string()],
        )?;

        log::warn!("Marked operation {} as failed: {}", id, error);
        Ok(())
    }

    pub fn cleanup_old(&mut self, older_than_secs: u64) -> Result<usize> {
        let cutoff = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() - older_than_secs;

        let count = self.conn.execute(
            "DELETE FROM persistent_operations 
             WHERE status IN ('completed', 'failed') 
             AND created_at < ?1",
            params![cutoff as i64],
        )?;

        if count > 0 {
            log::info!("Cleaned up {} old operations", count);
        }

        Ok(count)
    }

    pub fn pending_count(&self) -> Result<usize> {
        let count: i32 = self.conn.query_row(
            "SELECT COUNT(*) FROM persistent_operations WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_persistence_across_restart() -> Result<()> {
        let temp_dir = tempdir()?;
        let db_path = temp_dir.path().join("test_persist.db");

        // First "session"
        {
            let mut queue = PersistentRetryQueue::new(&db_path)?;
            let op = PersistentOp::SendMessage {
                group_id: vec![1, 2, 3],
                content: "Hello".to_string(),
            };
            let _id = queue.enqueue(op, None)?;
            // Simulate crash - don't complete
        }

        // Second "session"
        {
            let queue = PersistentRetryQueue::new(&db_path)?;
            let pending = queue.get_pending_operations()?;
            assert_eq!(pending.len(), 1, "Should have 1 pending operation");
            assert!(matches!(pending[0].1, PersistentOp::SendMessage { .. }));
        }

        Ok(())
    }

    #[test]
    fn test_retry_backoff() -> Result<()> {
        let temp_dir = tempdir()?;
        let db_path = temp_dir.path().join("test_backoff.db");

        let test_pubkey =
            PublicKey::parse("npub1gcxzte5zlkncx26j68ez60fzkvtkm9e0vrwdcvsjakxf9mu9qewqlfnj5z")?;

        let mut queue = PersistentRetryQueue::new(&db_path)?;

        let op = PersistentOp::FetchKeyPackage {
            pubkey: test_pubkey,
        };
        let id = queue.enqueue(op, None)?;

        // Schedule first retry
        queue.schedule_retry(&id, 0)?;

        // Should not be available immediately
        let pending = queue.get_pending_operations()?;
        assert_eq!(
            pending.len(),
            0,
            "Should not be ready for retry immediately"
        );

        Ok(())
    }

    #[test]
    fn test_completion_with_event() -> Result<()> {
        let temp_dir = tempdir()?;
        let db_path = temp_dir.path().join("test_complete.db");

        let mut queue = PersistentRetryQueue::new(&db_path)?;

        let op = PersistentOp::SendMessage {
            group_id: vec![1, 2, 3],
            content: "Test".to_string(),
        };

        let success_event = SuccessEvent::MessageSent {
            group_id: vec![1, 2, 3],
            timestamp: 12345,
        };

        let id = queue.enqueue(op, Some(success_event.clone()))?;

        // Complete the operation
        let event = queue.complete(&id)?;
        assert!(event.is_some(), "Should return success event");

        // Verify it's no longer pending
        let pending = queue.get_pending_operations()?;
        assert_eq!(pending.len(), 0, "Should have no pending operations");

        Ok(())
    }
}
