use std::thread;
use std::time::Duration;
use crossterm::event::{self, Event};
use tokio::sync::mpsc;
use crate::AppEvent;

pub fn spawn_keyboard_listener(tx: mpsc::UnboundedSender<AppEvent>) {
    thread::spawn(move || {
        loop {
            // Poll with zero timeout - never blocks
            if event::poll(Duration::ZERO).unwrap_or(false) {
                if let Ok(Event::Key(key)) = event::read() {
                    let _ = tx.send(AppEvent::KeyPress(key));
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
    });
}