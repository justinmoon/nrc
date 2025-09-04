use crate::AppEvent;
use nostr_sdk::prelude::*;
use openmls::group::GroupId;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

/// Spawn a task to fetch messages - just fetches, doesn't process
pub fn spawn_fetch_messages(
    groups: Vec<GroupId>,
    groups_map: HashMap<GroupId, nostr_mls_storage::groups::types::Group>,
    client: Client,
    event_tx: mpsc::UnboundedSender<AppEvent>,
) {
    tokio::spawn(async move {
        log::debug!("Background: fetching messages for {} groups", groups.len());
        let start = std::time::Instant::now();

        let mut all_events = Vec::new();

        for group_id in groups {
            let group = match groups_map.get(&group_id) {
                Some(g) => g,
                None => continue,
            };

            let h_tag_value = hex::encode(group.nostr_group_id);

            let filter = Filter::new()
                .kind(Kind::from(445u16))
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value.clone())
                .limit(100);

            // Subscribe and fetch
            if client.subscribe(filter.clone(), None).await.is_ok() {
                if let Ok(events) = client.fetch_events(filter, Duration::from_secs(2)).await {
                    let events_vec: Vec<Event> = events.into_iter().collect();
                    log::debug!(
                        "Fetched {} events for group {}",
                        events_vec.len(),
                        h_tag_value
                    );
                    all_events.extend(events_vec);
                }
            }
        }

        log::debug!(
            "Background: fetched {} total message events in {:?}",
            all_events.len(),
            start.elapsed()
        );

        // Send all events back to main loop for processing
        if !all_events.is_empty() {
            let _ = event_tx.send(AppEvent::RawMessagesReceived { events: all_events });
        }
    });
}

/// Spawn a task to fetch welcomes - just fetches GiftWrap events
pub fn spawn_fetch_welcomes(client: Client, keys: Keys, event_tx: mpsc::UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        log::debug!("Background: fetching welcomes");
        let start = std::time::Instant::now();

        let filter = Filter::new()
            .kind(Kind::GiftWrap)
            .pubkey(keys.public_key())
            .since(Timestamp::now() - Duration::from_secs(60 * 60)); // Last hour

        if let Ok(events) = client.fetch_events(filter, Duration::from_secs(2)).await {
            let events_vec: Vec<Event> = events.into_iter().collect();
            log::debug!(
                "Background: fetched {} GiftWrap events in {:?}",
                events_vec.len(),
                start.elapsed()
            );

            if !events_vec.is_empty() {
                let _ = event_tx.send(AppEvent::RawWelcomesReceived { events: events_vec });
            }
        }
    });
}
