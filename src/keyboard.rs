use crate::AppEvent;
use crossterm::event::{self, Event};
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc;

pub fn spawn_keyboard_listener(tx: mpsc::UnboundedSender<AppEvent>) {
    thread::spawn(move || {
        loop {
            // Poll with zero timeout - never blocks
            if event::poll(Duration::ZERO).unwrap_or(false) {
                match event::read() {
                    Ok(Event::Key(key)) => {
                        let _ = tx.send(AppEvent::KeyPress(key));
                    }
                    Ok(Event::Paste(text)) => {
                        let _ = tx.send(AppEvent::Paste(text));
                    }
                    _ => {}
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
    });
}
