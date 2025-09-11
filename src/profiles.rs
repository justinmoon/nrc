use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use nostr_sdk::prelude::*;
use tokio::sync::Mutex;

/// Small helper that owns Kind 0 cache + subscription/fetch logic.
#[derive(Clone)]
pub struct Profiles {
    store: Arc<Mutex<HashMap<PublicKey, Metadata>>>,
}
impl Profiles {
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn cache(&self, pk: PublicKey, metadata: Metadata) {
        let mut map = self.store.lock().await;
        map.insert(pk, metadata);
    }

    /// Return best-effort display name for pk.
    pub async fn display_name_async(&self, pk: &PublicKey) -> Option<String> {
        let map = self.store.lock().await;
        Self::pick_name(map.get(pk))
    }

    /// Non-async convenience using a fast path if lock is available.
    pub fn display_name(&self, pk: &PublicKey) -> Option<String> {
        if let Ok(map) = self.store.try_lock() {
            return Self::pick_name(map.get(pk));
        }
        None
    }

    /// Try to get a snapshot of profiles without blocking.
    pub fn try_snapshot(&self) -> Option<HashMap<PublicKey, Metadata>> {
        self.store.try_lock().ok().map(|m| m.clone())
    }

    fn pick_name(meta: Option<&Metadata>) -> Option<String> {
        meta.and_then(|m| m.display_name.clone().filter(|s| !s.is_empty()))
            .or_else(|| meta.and_then(|m| m.name.clone().filter(|s| !s.is_empty())))
    }

    /// Subscribe and opportunistically fetch missing Kind 0 for pubkeys.
    pub async fn ensure(&self, client: &Client, pubkeys: Vec<PublicKey>) -> Result<()> {
        if pubkeys.is_empty() {
            return Ok(());
        }

        // Subscribe first
        let filter = Filter::new().kind(Kind::Metadata).authors(pubkeys.clone());
        let _ = client.subscribe(filter, None).await;

        // Determine what we still need
        let missing: Vec<PublicKey> = {
            let map = self.store.lock().await;
            pubkeys
                .into_iter()
                .filter(|pk| !map.contains_key(pk))
                .collect()
        };

        if missing.is_empty() {
            return Ok(());
        }

        // One-shot fetch fallback
        let filter = Filter::new().kind(Kind::Metadata).authors(missing);
        if let Ok(events) = client
            .fetch_events(filter, std::time::Duration::from_secs(3))
            .await
        {
            if !events.is_empty() {
                let mut map = self.store.lock().await;
                for ev in events {
                    if let Ok(meta) = Metadata::from_json(&ev.content) {
                        map.insert(ev.pubkey, meta);
                    }
                }
            }
        }
        Ok(())
    }
}

impl Default for Profiles {
    fn default() -> Self {
        Self::new()
    }
}
