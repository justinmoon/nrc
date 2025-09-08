mod keyboard;
mod tui;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nrc::actions::Action;
use nrc::evented_nrc::{EventedNrc, convert_key_to_action};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::fs;
use std::io;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::time::Duration;

fn default_data_dir() -> String {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .expect("Failed to get home directory")
            .join("Library")
            .join("Application Support")
            .join("nrc")
            .to_string_lossy()
            .to_string()
    }
    #[cfg(target_os = "linux")]
    {
        dirs::home_dir()
            .expect("Failed to get home directory")
            .join(".local")
            .join("share")
            .join("nrc")
            .to_string_lossy()
            .to_string()
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Parser, Debug)]
#[command(name = "nrc", about = "Secure group chat", version)]
struct Args {
    /// Path to the data directory
    #[arg(long, default_value_t = default_data_dir())]
    datadir: String,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value_t = default_log_level())]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Ensure data directory exists
    let datadir = PathBuf::from(&args.datadir);
    fs::create_dir_all(&datadir)?;

    // Initialize logging
    let log_file = datadir.join("nrc.log");
    
    // Configure env_logger with custom format
    use std::io::Write;
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&args.log_level))
        .format(|buf, record| {
            writeln!(
                buf,
                "[{} {} {}:{}] {}",
                chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
                record.level(),
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.args()
            )
        })
        .target(env_logger::Target::Pipe(Box::new(
            OpenOptions::new()
                .create(true)
                .write(true)
                .append(true)
                .open(&log_file)
                .expect("Failed to open log file"),
        )))
        .init();

    log::info!("Starting nrc with data directory: {:?}", datadir);

    // Create EventedNrc with background processing
    let evented = EventedNrc::new(&datadir).await?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, evented).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Error: {err:?}");
    }

    Ok(())
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    mut evented: EventedNrc,
) -> Result<()> {
    // Channel for keyboard events
    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    
    // Spawn keyboard listener
    keyboard::spawn_keyboard_listener(key_tx.clone());
    
    // Main loop
    loop {
        // Draw UI with current state
        terminal.draw(|f| tui::draw_evented(f, &evented))?;
        
        // Use tokio::select! to handle multiple async operations
        tokio::select! {
            // Check for state changes (efficient redraw)
            _ = evented.ui_state.changed() => {
                // State changed, will redraw on next loop iteration
            }
            
            // Handle keyboard input
            Some(key_event) = key_rx.recv() => {
                // Convert key event to action and emit
                let state = evented.current_state();
                if let Some(action) = convert_key_to_action(key_event, &state) {
                    if matches!(action, Action::Quit) {
                        return Ok(());
                    }
                    evented.emit(action);
                }
            }
            
            // Add a small timeout to prevent busy waiting
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                // Periodic tick for any housekeeping
            }
        }
    }
}

use std::fs::OpenOptions;