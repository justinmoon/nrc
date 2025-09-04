use crate::AppEvent;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

pub async fn spawn_timer_task(tx: mpsc::UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let mut message_interval = interval(Duration::from_secs(2));
        let mut welcome_interval = interval(Duration::from_secs(3));
        let mut pending_ops_interval = interval(Duration::from_secs(30));

        loop {
            tokio::select! {
                _ = message_interval.tick() => {
                    let _ = tx.send(AppEvent::FetchMessagesTick);
                }
                _ = welcome_interval.tick() => {
                    let _ = tx.send(AppEvent::FetchWelcomesTick);
                }
                _ = pending_ops_interval.tick() => {
                    let _ = tx.send(AppEvent::ProcessPendingOperationsTick);
                }
            }
        }
    });
}
