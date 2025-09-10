use anyhow::{anyhow, Result};
use chrono::Utc;
use nostr_sdk::prelude::*;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::AppEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum OperationKind {
    SendMessage {
        event: Event,
    },
    PublishKeyPackage {
        event: Event,
    },
    CreateDm {
        other_pubkey: PublicKey,
        // State machine data
        step: CreateDmStep,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CreateDmStep {
    FetchKeyPackage,
    // After fetch, UI must create group in storage
    RequestCreateGroup {
        key_package: Event,
    },
    // After UI creates group, orchestrator subscribes and sends welcome
    SubscribeGroup {
        nostr_group_id_hex: String,
        welcome_rumor: UnsignedEvent,
        to: PublicKey,
    },
    SendWelcome {
        nostr_group_id_hex: String,
        welcome_rumor: UnsignedEvent,
        to: PublicKey,
    },
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OpStatus {
    Pending,
    InProgress,
    Success,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub kind: OperationKind,
    pub status: OpStatus,
    pub last_error: Option<String>,
    pub updated_at: i64,
    pub created_at: i64,
}

#[derive(Clone)]
pub struct OpsStore {
    db_path: PathBuf,
}

impl OpsStore {
    pub fn new(datadir: &Path) -> Result<Self> {
        let path = datadir.join("nrc_ops.db");
        let store = Self { db_path: path };
        store.init()?;
        Ok(store)
    }

    fn init(&self) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS operations (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                last_error TEXT,
                payload TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )",
            [],
        )?;
        Ok(())
    }

    pub fn enqueue(&self, kind: OperationKind) -> Result<String> {
        let now = Utc::now().timestamp();
        let id = Uuid::new_v4().to_string();
        let payload = serde_json::to_string(&kind)?;
        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "INSERT INTO operations (id, kind, status, last_error, payload, created_at, updated_at)
             VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6)",
            params![id, Self::kind_str(&kind), "Pending", payload, now, now],
        )?;
        Ok(id)
    }

    pub fn load(&self, id: &str) -> Result<Operation> {
        let conn = Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, kind, status, last_error, payload, created_at, updated_at
             FROM operations WHERE id = ?1",
        )?;
        let op = stmt.query_row(params![id], |row| {
            let id: String = row.get(0)?;
            let _kind_str: String = row.get(1)?;
            let status_str: String = row.get(2)?;
            let last_error: Option<String> = row.get(3)?;
            let payload: String = row.get(4)?;
            let created_at: i64 = row.get(5)?;
            let updated_at: i64 = row.get(6)?;
            let kind: OperationKind = serde_json::from_str(&payload).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    payload.len(),
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            let status = Self::status_from_str_lossy(&status_str);
            Ok(Operation {
                id,
                kind,
                status,
                last_error,
                created_at,
                updated_at,
            })
        })?;
        Ok(op)
    }

    pub fn take_next_pending(&self) -> Result<Option<Operation>> {
        let conn = Connection::open(&self.db_path)?;
        // Pick one pending or in_progress op to resume
        let mut stmt = conn.prepare(
            "SELECT id FROM operations
             WHERE status IN ('Pending','InProgress')
             ORDER BY created_at ASC LIMIT 1",
        )?;
        let next: Option<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .next()
            .transpose()?;
        if let Some(id) = next {
            let mut op = self.load(&id)?;
            // Mark in progress
            op.status = OpStatus::InProgress;
            self.save(&op)?;
            Ok(Some(op))
        } else {
            Ok(None)
        }
    }

    pub fn save(&self, op: &Operation) -> Result<()> {
        let now = Utc::now().timestamp();
        let payload = serde_json::to_string(&op.kind)?;
        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "UPDATE operations SET kind = ?2, status = ?3, last_error = ?4, payload = ?5, updated_at = ?6 WHERE id = ?1",
            params![
                op.id,
                Self::kind_str(&op.kind),
                Self::status_str(&op.status),
                op.last_error,
                payload,
                now
            ],
        )?;
        Ok(())
    }

    pub fn list_all(&self) -> Result<Vec<Operation>> {
        let conn = Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, kind, status, last_error, payload, created_at, updated_at
             FROM operations ORDER BY updated_at DESC LIMIT 500",
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let _kind_str: String = row.get(1)?;
            let status_str: String = row.get(2)?;
            let last_error: Option<String> = row.get(3)?;
            let payload: String = row.get(4)?;
            let created_at: i64 = row.get(5)?;
            let updated_at: i64 = row.get(6)?;
            let kind: OperationKind = serde_json::from_str(&payload).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    payload.len(),
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            let status = Self::status_from_str_lossy(&status_str);
            Ok(Operation {
                id,
                kind,
                status,
                last_error,
                created_at,
                updated_at,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn mark_success(&self, id: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "UPDATE operations SET status = 'Success', updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }

    fn kind_str(kind: &OperationKind) -> &'static str {
        match kind {
            OperationKind::SendMessage { .. } => "SendMessage",
            OperationKind::PublishKeyPackage { .. } => "PublishKeyPackage",
            OperationKind::CreateDm { .. } => "CreateDm",
        }
    }

    fn status_str(status: &OpStatus) -> &'static str {
        match status {
            OpStatus::Pending => "Pending",
            OpStatus::InProgress => "InProgress",
            OpStatus::Success => "Success",
            OpStatus::Error => "Error",
        }
    }

    fn status_from_str_lossy(s: &str) -> OpStatus {
        match s {
            "Pending" => OpStatus::Pending,
            "InProgress" => OpStatus::InProgress,
            "Success" => OpStatus::Success,
            "Error" => OpStatus::Error,
            _ => OpStatus::Error,
        }
    }
}

#[derive(Debug)]
pub enum OpsCommand {
    Wake,
    Updated(String),
}

pub fn spawn_orchestrator(
    ops: OpsStore,
    client: Client,
    keys: Keys,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    mut cmd_rx: mpsc::UnboundedReceiver<OpsCommand>,
) {
    tokio::spawn(async move {
        loop {
            // Process at most one operation per tick to avoid busy loops
            if let Ok(Some(mut op)) = ops.take_next_pending() {
                if let Err(e) = process_operation(&ops, &client, &keys, &event_tx, &mut op).await {
                    log::error!("Operation {} failed: {}", op.id, e);
                    op.status = OpStatus::Error;
                    op.last_error = Some(e.to_string());
                    let _ = ops.save(&op);
                }
            }

            // Wait for a wake signal or small delay
            tokio::select! {
                Some(_) = cmd_rx.recv() => { /* wake up */ }
                _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
            }
        }
    });
}

async fn process_operation(
    ops: &OpsStore,
    client: &Client,
    keys: &Keys,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    op: &mut Operation,
) -> Result<()> {
    let current = op.kind.clone();
    match current {
        OperationKind::SendMessage { event } => {
            client.send_event(&event).await?;
            ops.mark_success(&op.id)?;
        }
        OperationKind::PublishKeyPackage { event } => {
            client.send_event(&event).await?;
            // Subscribe to giftwraps for this pubkey using auto-close
            let filter = Filter::new().kind(Kind::GiftWrap).pubkey(event.pubkey);
            let opts = SubscribeAutoCloseOptions::default()
                .exit_policy(ReqExitPolicy::ExitOnEOSE)
                .timeout(Some(std::time::Duration::from_secs(5)));
            client.subscribe(filter, Some(opts)).await?;
            ops.mark_success(&op.id)?;
        }
        OperationKind::CreateDm { other_pubkey, step } => {
            match step {
                CreateDmStep::FetchKeyPackage => {
                    // Fetch directly with a short timeout
                    let filter = Filter::new()
                        .kind(Kind::MlsKeyPackage)
                        .author(other_pubkey)
                        .limit(1);
                    let events = client
                        .fetch_events(filter.clone(), std::time::Duration::from_secs(2))
                        .await?;
                    let key_package = events
                        .into_iter()
                        .next()
                        .ok_or_else(|| anyhow!("No key package found for {}", other_pubkey))?;

                    // Update op to request storage-side group creation
                    op.kind = OperationKind::CreateDm {
                        other_pubkey,
                        step: CreateDmStep::RequestCreateGroup {
                            key_package: key_package.clone(),
                        },
                    };
                    ops.save(op)?;

                    // Ask UI to perform storage-bound group creation
                    let group_name = format!("DM with {}", other_pubkey.to_bech32()?);
                    let _ = event_tx.send(AppEvent::OpNeedsStorageCreateGroup {
                        op_id: op.id.clone(),
                        other_pubkey,
                        key_package,
                        group_name,
                    });
                }
                CreateDmStep::RequestCreateGroup { key_package } => {
                    // Re-emit the storage request so UI can proceed (important on resume)
                    let group_name = format!("DM with {}", other_pubkey.to_bech32()?);
                    let _ = event_tx.send(AppEvent::OpNeedsStorageCreateGroup {
                        op_id: op.id.clone(),
                        other_pubkey,
                        key_package: key_package.clone(),
                        group_name,
                    });
                    // Leave op InProgress; UI will update to next step when storage completes
                }
                CreateDmStep::SubscribeGroup {
                    welcome_rumor, to, ..
                } => {
                    // Send welcome immediately for faster UX
                    let gift_wrapped =
                        EventBuilder::gift_wrap(keys, &to, welcome_rumor.clone(), None).await?;
                    client.send_event(&gift_wrapped).await?;
                    ops.mark_success(&op.id)?;
                    op.kind = OperationKind::CreateDm {
                        other_pubkey: to,
                        step: CreateDmStep::Done,
                    };
                }
                CreateDmStep::SendWelcome {
                    nostr_group_id_hex: _,
                    welcome_rumor,
                    to,
                } => {
                    let gift_wrapped =
                        EventBuilder::gift_wrap(keys, &to, welcome_rumor.clone(), None).await?;
                    client.send_event(&gift_wrapped).await?;
                    ops.mark_success(&op.id)?;
                    op.kind = OperationKind::CreateDm {
                        other_pubkey: to,
                        step: CreateDmStep::Done,
                    };
                }
                CreateDmStep::Done => {
                    ops.mark_success(&op.id)?;
                }
            }
        }
    }
    Ok(())
}
