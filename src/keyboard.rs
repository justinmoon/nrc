use crossterm::event::{self, Event, KeyEvent};
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc;

pub fn spawn_keyboard_listener(tx: mpsc::UnboundedSender<KeyEvent>) {
    thread::spawn(move || {
        loop {
            // Poll with zero timeout - never blocks
            if event::poll(Duration::ZERO).unwrap_or(false) {
                if let Ok(Event::Key(key)) = event::read() {
                    let _ = tx.send(key);
                }
                // TODO: Handle paste events
            }
            thread::sleep(Duration::from_millis(10));
        }
    });
}
