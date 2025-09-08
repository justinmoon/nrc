use crate::Nrc;
use anyhow::Result;
use nostr_sdk::prelude::*;
use std::time::Duration;

impl Nrc {
    pub async fn publish_key_package(&mut self) -> Result<()> {
        // Use network task channel instead of direct network call
        if let Some(command_tx) = &self.command_tx {
            command_tx
                .send(crate::NetworkCommand::PublishKeyPackage)
                .await?;
        } else {
            return Err(anyhow::anyhow!("Network task not initialized"));
        }
        Ok(())
    }

    pub async fn fetch_key_package(&self, pubkey: &PublicKey) -> Result<Event> {
        let filter = Filter::new()
            .kind(Kind::from(443u16))
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
