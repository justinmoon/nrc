use anyhow::Result;
use rusqlite::{Connection, Row, params};
use std::path::Path;
use uuid::Uuid;

use crate::outbox::{Command, CommandStatus, PersistentCommand, TaskEvent};

pub struct SqliteCommandStore {
    conn: Connection,
}

impl SqliteCommandStore {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        
        // Create tables if they don't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS outbox_tasks (
                id TEXT PRIMARY KEY,
                task_type TEXT NOT NULL,
                state TEXT NOT NULL,
                payload BLOB NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                retry_count INTEGER DEFAULT 0,
                next_retry_at INTEGER,
                error TEXT,
                dependencies TEXT -- JSON array of UUIDs
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_next_retry ON outbox_tasks(next_retry_at)",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_state ON outbox_tasks(state)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS task_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                data BLOB,
                FOREIGN KEY (task_id) REFERENCES outbox_tasks(id)
            )",
            [],
        )?;

        Ok(Self { conn })
    }

    pub fn store_command(&mut self, command: &PersistentCommand) -> Result<()> {
        let command_json = serde_json::to_vec(&command.command)?;
        let dependencies_json = serde_json::to_string(&command.dependencies)?;
        let status_str = match command.status {
            CommandStatus::Pending => "pending",
            CommandStatus::InProgress => "in_progress", 
            CommandStatus::Completed => "completed",
            CommandStatus::Failed => "failed",
        };

        let task_type = match &command.command {
            Command::CreateGroup { .. } => "create_group",
            Command::FetchKeyPackage { .. } => "fetch_key_package",
            Command::SendWelcome { .. } => "send_welcome",
            Command::ProcessMessage { .. } => "process_message",
        };

        self.conn.execute(
            "INSERT OR REPLACE INTO outbox_tasks 
             (id, task_type, state, payload, created_at, updated_at, retry_count, next_retry_at, error, dependencies)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                command.id.to_string(),
                task_type,
                status_str,
                command_json,
                command.metadata.created_at as i64,
                command.metadata.updated_at as i64,
                command.metadata.retry_count as i64,
                command.metadata.next_retry_at.map(|t| t as i64),
                command.metadata.error,
                dependencies_json
            ],
        )?;

        Ok(())
    }

    pub fn get_command(&self, id: &Uuid) -> Result<Option<PersistentCommand>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_type, state, payload, created_at, updated_at, retry_count, next_retry_at, error, dependencies 
             FROM outbox_tasks WHERE id = ?1"
        )?;

        let result = stmt.query_row(params![id.to_string()], |row| {
            self.row_to_command(row)
        });

        match result {
            Ok(command) => Ok(Some(command)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_pending_commands(&self) -> Result<Vec<PersistentCommand>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_type, state, payload, created_at, updated_at, retry_count, next_retry_at, error, dependencies 
             FROM outbox_tasks 
             WHERE state = 'pending' AND (next_retry_at IS NULL OR next_retry_at <= ?1)
             ORDER BY created_at ASC"
        )?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let rows = stmt.query_map(params![now], |row| {
            self.row_to_command(row)
        })?;

        let mut commands = Vec::new();
        for row in rows {
            commands.push(row?);
        }

        Ok(commands)
    }

    pub fn get_commands_by_status(&self, status: CommandStatus) -> Result<Vec<PersistentCommand>> {
        let status_str = match status {
            CommandStatus::Pending => "pending",
            CommandStatus::InProgress => "in_progress",
            CommandStatus::Completed => "completed", 
            CommandStatus::Failed => "failed",
        };

        let mut stmt = self.conn.prepare(
            "SELECT id, task_type, state, payload, created_at, updated_at, retry_count, next_retry_at, error, dependencies 
             FROM outbox_tasks WHERE state = ?1 ORDER BY created_at ASC"
        )?;

        let rows = stmt.query_map(params![status_str], |row| {
            self.row_to_command(row)
        })?;

        let mut commands = Vec::new();
        for row in rows {
            commands.push(row?);
        }

        Ok(commands)
    }

    pub fn update_command_status(&mut self, id: &Uuid, status: CommandStatus) -> Result<()> {
        let status_str = match status {
            CommandStatus::Pending => "pending",
            CommandStatus::InProgress => "in_progress",
            CommandStatus::Completed => "completed",
            CommandStatus::Failed => "failed",
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.conn.execute(
            "UPDATE outbox_tasks SET state = ?1, updated_at = ?2 WHERE id = ?3",
            params![status_str, now, id.to_string()],
        )?;

        Ok(())
    }

    pub fn delete_command(&mut self, id: &Uuid) -> Result<()> {
        self.conn.execute(
            "DELETE FROM outbox_tasks WHERE id = ?1",
            params![id.to_string()],
        )?;

        self.conn.execute(
            "DELETE FROM task_events WHERE task_id = ?1",
            params![id.to_string()],
        )?;

        Ok(())
    }

    pub fn add_event(&mut self, event: &TaskEvent) -> Result<()> {
        self.conn.execute(
            "INSERT INTO task_events (task_id, event_type, timestamp, data) VALUES (?1, ?2, ?3, ?4)",
            params![
                event.task_id.to_string(),
                event.event_type,
                event.timestamp as i64,
                event.data
            ],
        )?;

        Ok(())
    }

    pub fn get_events_for_task(&self, task_id: &Uuid) -> Result<Vec<TaskEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, event_type, timestamp, data 
             FROM task_events WHERE task_id = ?1 ORDER BY timestamp ASC"
        )?;

        let rows = stmt.query_map(params![task_id.to_string()], |row| {
            Ok(TaskEvent {
                id: row.get(0)?,
                task_id: Uuid::parse_str(&row.get::<_, String>(1)?)
                    .map_err(|_e| rusqlite::Error::InvalidColumnType(1, "Invalid UUID".to_string(), rusqlite::types::Type::Text))?,
                event_type: row.get(2)?,
                timestamp: row.get::<_, i64>(3)? as u64,
                data: row.get(4)?,
            })
        })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }

        Ok(events)
    }

    pub fn cleanup_completed_tasks(&mut self, older_than_seconds: u64) -> Result<usize> {
        let cutoff_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(older_than_seconds) as i64;

        let count = self.conn.execute(
            "DELETE FROM outbox_tasks WHERE state = 'completed' AND updated_at < ?1",
            params![cutoff_time],
        )?;

        Ok(count)
    }

    fn row_to_command(&self, row: &Row) -> rusqlite::Result<PersistentCommand> {
        let id_str: String = row.get(0)?;
        let id = Uuid::parse_str(&id_str)
            .map_err(|_| rusqlite::Error::InvalidColumnType(0, "Invalid UUID".to_string(), rusqlite::types::Type::Text))?;

        let payload: Vec<u8> = row.get(3)?;
        let command: Command = serde_json::from_slice(&payload)
            .map_err(|_| rusqlite::Error::InvalidColumnType(3, "Invalid JSON".to_string(), rusqlite::types::Type::Blob))?;

        let state_str: String = row.get(2)?;
        let status = match state_str.as_str() {
            "pending" => CommandStatus::Pending,
            "in_progress" => CommandStatus::InProgress,
            "completed" => CommandStatus::Completed,
            "failed" => CommandStatus::Failed,
            _ => return Err(rusqlite::Error::InvalidColumnType(2, "Invalid status".to_string(), rusqlite::types::Type::Text)),
        };

        let dependencies_str: String = row.get(9)?;
        let dependencies: Vec<Uuid> = serde_json::from_str(&dependencies_str)
            .map_err(|_| rusqlite::Error::InvalidColumnType(9, "Invalid dependencies JSON".to_string(), rusqlite::types::Type::Text))?;

        let mut metadata = crate::outbox::CommandMetadata::new();
        metadata.created_at = row.get::<_, i64>(4)? as u64;
        metadata.updated_at = row.get::<_, i64>(5)? as u64;
        metadata.retry_count = row.get::<_, i64>(6)? as u32;
        metadata.next_retry_at = row.get::<_, Option<i64>>(7)?.map(|t| t as u64);
        metadata.error = row.get(8)?;

        Ok(PersistentCommand {
            id,
            command,
            status,
            metadata,
            dependencies,
        })
    }
}