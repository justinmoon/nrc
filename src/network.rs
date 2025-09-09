use crate::config::get_default_relays;
use crate::types::AppState;
use crate::Nrc;
use anyhow::Result;
use nostr_sdk::prelude::*;
use std::time::Duration;

impl Nrc {
    pub async fn publish_key_package(&mut self) -> Result<()> {
        // First subscribe to key packages so we can verify our own
        let filter = Filter::new()
            .kind(Kind::MlsKeyPackage)
            .author(self.keys.public_key());
        self.client.subscribe(filter, None).await?;

        let relays: Result<Vec<RelayUrl>, _> = get_default_relays()
            .iter()
            .map(|&url| RelayUrl::parse(url))
            .collect();
        let relays = relays?;
        let (key_package_content, tags) = self
            .storage
            .create_key_package_for_event(&self.keys.public_key(), relays)?;

        let event = EventBuilder::new(Kind::MlsKeyPackage, key_package_content)
            .tags(tags)
            .build(self.keys.public_key())
            .sign(&self.keys)
            .await?;

        self.client.send_event(&event).await?;

        // Wait a bit to ensure it's published
        tokio::time::sleep(Duration::from_secs(1)).await;

        let filter = Filter::new()
            .kind(Kind::GiftWrap)
            .pubkey(self.keys.public_key());
        self.client.subscribe(filter, None).await?;

        if let AppState::Ready { groups, .. } = &self.state {
            self.state = AppState::Ready {
                key_package_published: true,
                groups: groups.clone(),
            };
        } else {
            self.state = AppState::Ready {
                key_package_published: true,
                groups: Vec::new(),
            };
        }

        Ok(())
    }

    pub async fn fetch_key_package(&self, pubkey: &PublicKey) -> Result<Event> {
        let filter = Filter::new()
            .kind(Kind::MlsKeyPackage)
            .author(*pubkey)
            .limit(1);

        // Subscribe to ensure we can fetch events
        self.client.subscribe(filter.clone(), None).await?;

        // Give time for event to propagate to relay and retry multiple times
        for attempt in 1..=10 {
            tokio::time::sleep(Duration::from_millis(1500)).await;

            // Try to fetch from relay
            let events = self
                .client
                .fetch_events(filter.clone(), Duration::from_secs(5))
                .await?;

            if let Some(event) = events.into_iter().next() {
                log::debug!("Found key package on attempt {attempt}");
                return Ok(event);
            }
            log::debug!("Key package not found on attempt {attempt}");
        }

        Err(anyhow::anyhow!("No key package found for {pubkey}"))
    }
}
