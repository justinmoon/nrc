use crate::{AppEvent, ReactiveAppEvent};
use nostr_sdk::prelude::*;
use tokio::sync::mpsc;

/// Spawn a task to handle real-time subscription notifications
pub fn spawn_notification_handler(client: Client, event_tx: mpsc::UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        log::info!("Starting notification handler for real-time events");

        let result = client
            .handle_notifications(|notification| async {
                match notification {
                    RelayPoolNotification::Event { event, .. } => {
                        match event.kind {
                            Kind::GiftWrap => {
                                // Send GiftWrap events (welcomes/messages) to main loop
                                let _ = event_tx.send(AppEvent::RawWelcomesReceived {
                                    events: vec![event.as_ref().clone()],
                                });
                            }
                            kind if kind == Kind::from(445u16) => {
                                // Send MLS messages to main loop
                                let _ = event_tx.send(AppEvent::RawMessagesReceived {
                                    events: vec![event.as_ref().clone()],
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

pub fn spawn_reactive_notification_handler(
    client: Client,
    event_tx: mpsc::UnboundedSender<ReactiveAppEvent>,
) {
    tokio::spawn(async move {
        log::info!("Starting reactive notification handler for real-time events");

        let result = client
            .handle_notifications(|notification| async {
                match notification {
                    RelayPoolNotification::Event { event, .. } => match event.kind {
                        Kind::GiftWrap => {
                            let _ = event_tx.send(ReactiveAppEvent::RawWelcomesReceived {
                                events: vec![event.as_ref().clone()],
                            });
                        }
                        kind if kind == Kind::from(445u16) => {
                            let _ = event_tx.send(ReactiveAppEvent::RawMessagesReceived {
                                events: vec![event.as_ref().clone()],
                            });
                        }
                        _ => {
                            log::debug!("Received event of kind: {}", event.kind);
                        }
                    },
                    RelayPoolNotification::Message { message, .. } => {
                        log::debug!("Received relay message: {message:?}");
                    }
                    RelayPoolNotification::Shutdown => {
                        log::info!("Relay pool shutdown notification");
                        return Ok(true);
                    }
                }
                Ok(false)
            })
            .await;

        if let Err(e) = result {
            log::error!("Reactive notification handler error: {e}");
            let _ = event_tx.send(ReactiveAppEvent::NetworkError {
                error: format!("Notification handler failed: {e}"),
            });
        }
    });
}
