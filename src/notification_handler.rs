use crate::AppEvent;
use nostr_sdk::prelude::*;
use tokio::sync::mpsc;

// Test helpers module - only used in tests
pub mod test_helpers {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    // Global event log for tests - maps client ID to event log
    lazy_static::lazy_static! {
        pub static ref TEST_EVENT_LOGS: Arc<Mutex<HashMap<String, Arc<Mutex<Vec<(String, Event)>>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
    }

    pub async fn register_test_event_log(client_id: String) -> Arc<Mutex<Vec<(String, Event)>>> {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut logs = TEST_EVENT_LOGS.lock().await;
        logs.insert(client_id, log.clone());
        log
    }

    pub async fn log_test_event(client_id: &str, event: Event) {
        let logs = TEST_EVENT_LOGS.lock().await;
        if let Some(log) = logs.get(client_id) {
            let timestamp = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
            let mut event_log = log.lock().await;
            event_log.push((timestamp, event.clone()));
        }
    }
}

/// Spawn a task to handle real-time subscription notifications
pub fn spawn_notification_handler(
    client: Client,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    pubkey: PublicKey,
) {
    // Use npub as ID for test logging
    let client_id = pubkey.to_bech32().unwrap_or_else(|_| pubkey.to_hex());

    tokio::spawn(async move {
        log::info!("Starting notification handler for real-time events");

        let result = client
            .handle_notifications(|notification| async {
                match notification {
                    RelayPoolNotification::Event { event, .. } => {
                        // Log all events for debugging
                        test_helpers::log_test_event(&client_id, event.as_ref().clone()).await;

                        match event.kind {
                            Kind::GiftWrap => {
                                // Send GiftWrap events (welcomes/messages) to main loop
                                let _ = event_tx.send(AppEvent::RawWelcomesReceived {
                                    events: vec![event.as_ref().clone()],
                                });
                            }
                            Kind::MlsGroupMessage => {
                                // Send MLS messages to main loop
                                let _ = event_tx.send(AppEvent::RawMessagesReceived {
                                    events: vec![event.as_ref().clone()],
                                });
                            }
                            Kind::MlsKeyPackage => {
                                // Key package events - forward to app for storage
                                log::info!(
                                    "Received key package from {}",
                                    event
                                        .pubkey
                                        .to_bech32()
                                        .unwrap_or_else(|_| "unknown".to_string())
                                );
                                let _ = event_tx.send(AppEvent::KeyPackageReceived {
                                    event: event.as_ref().clone(),
                                });
                            }
                            _ => {
                                // Handle other event types if needed
                                log::debug!("Received event of kind: {}", event.kind);
                            }
                        }
                    }
                    RelayPoolNotification::Message { message, .. } => {
                        log::debug!("Received relay message: {message:?}");
                    }
                    RelayPoolNotification::Shutdown => {
                        log::info!("Relay pool shutdown notification");
                        return Ok(true); // Stop processing
                    }
                }
                Ok(false) // Continue processing
            })
            .await;

        if let Err(e) = result {
            log::error!("Notification handler error: {e}");
            let _ = event_tx.send(AppEvent::NetworkError {
                error: format!("Notification handler failed: {e}"),
            });
        }
    });
}
