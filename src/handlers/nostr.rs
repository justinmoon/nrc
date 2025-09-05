// Nostr event handling logic functions
// These will be called from Nrc's internal event handler

use anyhow::Result;
use nostr_sdk::prelude::*;
use std::time::Duration;

pub async fn fetch_events_with_filter(client: &Client, filter: Filter) -> Result<Vec<Event>> {
    let events = client.fetch_events(filter, Duration::from_secs(5)).await?;
    Ok(events.into_iter().collect())
}

pub async fn subscribe_to_filter(client: &Client, filter: Filter) -> Result<()> {
    client.subscribe(filter, None).await?;
    Ok(())
}

pub async fn send_nostr_event(client: &Client, event: Event) -> Result<EventId> {
    let event_id = client.send_event(&event).await?;
    Ok(event_id.val)
}
