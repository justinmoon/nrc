// Application event handling logic functions
// These will be called from Nrc's internal event handler

use anyhow::Result;
use nostr_sdk::prelude::*;
use std::time::Duration;

pub async fn handle_profile_fetch(client: &Client, pubkey: PublicKey) -> Result<Option<Metadata>> {
    let filter = Filter::new().kind(Kind::Metadata).author(pubkey).limit(1);

    let events = client.fetch_events(filter, Duration::from_secs(5)).await?;

    if let Some(event) = events.first() {
        if event.kind == Kind::Metadata {
            return Ok(Some(Metadata::from_json(&event.content)?));
        }
    }

    Ok(None)
}

pub async fn publish_profile(
    client: &Client,
    keys: &Keys,
    display_name: String,
) -> Result<EventId> {
    let metadata = Metadata::new().display_name(display_name);
    client.set_metadata(&metadata).await?;

    let event = EventBuilder::metadata(&metadata).sign_with_keys(keys)?;
    let event_id = client.send_event(&event).await?;

    Ok(event_id.val)
}
