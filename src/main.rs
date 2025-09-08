mod keyboard;
mod tui;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nrc::actions::Action;
use nrc::evented_nrc::{EventedNrc, EventLoop};
use nrc::AppState;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::time::Duration;

fn default_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .expect("Failed to get home directory")
            .join("Library")
            .join("Application Support")
            .join("nrc")
    }
    #[cfg(target_os = "linux")]
    {
        dirs::data_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .expect("Failed to get home directory")
                    .join(".local")
                    .join("share")
            })
            .join("nrc")
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Data directory for logs and other files
    #[arg(long, value_parser, default_value_os_t = default_data_dir())]
    datadir: PathBuf,
}

fn setup_logging(datadir: &PathBuf) -> Result<()> {
    use env_logger::Builder;
    use log::LevelFilter;
    use std::io::Write;

    // Create datadir if it doesn't exist
    fs::create_dir_all(datadir)?;

    // Use nrc.log in the datadir
    let log_path = datadir.join("nrc.log");

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)?;

    println!("Logging to: {}", log_path.display());

    Builder::new()
        .target(env_logger::Target::Pipe(Box::new(file)))
        .filter_level(LevelFilter::Debug)
        .format(|buf, record| {
            writeln!(
                buf,
                "[{} {} {}:{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                record.level(),
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.args()
            )
        })
        .init();

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    setup_logging(&args.datadir)?;
    log::info!("Starting NRC with datadir: {:?}", args.datadir);

    // Create EventedNrc and EventLoop
    let (evented, event_loop) = EventedNrc::new(&args.datadir).await?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, evented, event_loop).await;

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
    mut event_loop: EventLoop,
) -> Result<()> {
    // Channel for keyboard events
    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    
    // Spawn keyboard listener
    keyboard::spawn_keyboard_listener(key_tx.clone());
    
    // TODO: Update notification handler to emit Actions
    // For now, we'll handle notifications through the existing pattern
    
    // Main loop
    loop {
        // Draw UI with current state
        terminal.draw(|f| tui::draw_evented(f, &evented))?;
        
        // Use tokio::select! to handle multiple async operations
        tokio::select! {
            // Check for state changes (efficient redraw)
            _ = evented.state.changed() => {
                // State changed, will redraw on next loop iteration
            }
            
            // Handle keyboard input
            Some(key_event) = key_rx.recv() => {
                // Convert key event to action and emit
                if let Some(action) = convert_key_to_action(&evented, key_event) {
                    if matches!(action, Action::Quit) {
                        return Ok(());
                    }
                    evented.emit(action);
                }
            }
            
            // Process events in the event loop
            _ = event_loop.process_one() => {
                // An action was processed
            }
            
            // Add a small timeout to prevent busy waiting
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                // Periodic tick for any housekeeping
            }
        }
    }
}

/// Convert keyboard events to Actions based on current state
fn convert_key_to_action(evented: &EventedNrc, key: KeyEvent) -> Option<Action> {
    // Emergency exit
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }
    
    let state = evented.state.borrow();
    match &*state {
        AppState::Onboarding { mode, .. } => {
            use nrc::OnboardingMode;
            match mode {
                OnboardingMode::Choose => {
                    match key.code {
                        KeyCode::Char('1') => {
                            Some(Action::OnboardingChoice(nrc::actions::OnboardingChoice::GenerateNew))
                        }
                        KeyCode::Char('2') => {
                            Some(Action::OnboardingChoice(nrc::actions::OnboardingChoice::ImportExisting))
                        }
                        KeyCode::Esc => Some(Action::Quit),
                        _ => None,
                    }
                }
                OnboardingMode::EnterDisplayName => {
                    match key.code {
                        KeyCode::Char(c) => {
                            let mut input = evented.input.borrow().clone();
                            input.push(c);
                            Some(Action::SetInput(input))
                        }
                        KeyCode::Backspace => {
                            let mut input = evented.input.borrow().clone();
                            input.pop();
                            Some(Action::SetInput(input))
                        }
                        KeyCode::Enter if !evented.input.borrow().is_empty() => {
                            let display_name = evented.input.borrow().clone();
                            Some(Action::SetDisplayName(display_name))
                        }
                        KeyCode::Esc => {
                            Some(Action::OnboardingChoice(nrc::actions::OnboardingChoice::GenerateNew))
                        }
                        _ => None,
                    }
                }
                OnboardingMode::ImportExisting => {
                    match key.code {
                        KeyCode::Char(c) => {
                            let mut input = evented.input.borrow().clone();
                            input.push(c);
                            Some(Action::SetInput(input))
                        }
                        KeyCode::Backspace => {
                            let mut input = evented.input.borrow().clone();
                            input.pop();
                            Some(Action::SetInput(input))
                        }
                        KeyCode::Enter if !evented.input.borrow().is_empty() => {
                            let nsec = evented.input.borrow().clone();
                            Some(Action::SetNsec(nsec))
                        }
                        KeyCode::Esc => {
                            Some(Action::OnboardingChoice(nrc::actions::OnboardingChoice::ImportExisting))
                        }
                        _ => None,
                    }
                }
                OnboardingMode::CreatePassword | OnboardingMode::EnterPassword => {
                    match key.code {
                        KeyCode::Char(c) => {
                            let mut input = evented.input.borrow().clone();
                            input.push(c);
                            Some(Action::SetInput(input))
                        }
                        KeyCode::Backspace => {
                            let mut input = evented.input.borrow().clone();
                            input.pop();
                            Some(Action::SetInput(input))
                        }
                        KeyCode::Enter if !evented.input.borrow().is_empty() => {
                            let password = evented.input.borrow().clone();
                            Some(Action::SetPassword(password))
                        }
                        KeyCode::Esc => {
                            // TODO: Handle going back in onboarding
                            None
                        }
                        _ => None,
                    }
                }
                _ => None,
            }
        }
        AppState::Initializing => None,
        AppState::Ready { .. } => {
            // Check if help is showing
            if *evented.show_help.borrow() {
                return Some(Action::DismissHelp);
            }
            
            let input = evented.input.borrow();
            match key.code {
                // Arrow keys for navigation (only when input is empty)
                KeyCode::Up if input.is_empty() => Some(Action::PrevGroup),
                KeyCode::Down if input.is_empty() => Some(Action::NextGroup),
                // Ctrl+j/k for navigation
                KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(Action::NextGroup)
                }
                KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(Action::PrevGroup)
                }
                // Regular character input
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let mut new_input = input.clone();
                    new_input.push(c);
                    Some(Action::SetInput(new_input))
                }
                KeyCode::Backspace => {
                    let mut new_input = input.clone();
                    new_input.pop();
                    Some(Action::SetInput(new_input))
                }
                KeyCode::Enter if !input.is_empty() => {
                    let input_str = input.clone();
                    
                    // Clear input first
                    evented.emit(Action::ClearInput);
                    
                    // Check if it's a command
                    if input_str.starts_with("/") {
                        // Parse command
                        let parts: Vec<&str> = input_str.split_whitespace().collect();
                        if parts.is_empty() {
                            return None;
                        }
                        
                        match parts[0] {
                            "/quit" | "/q" => Some(Action::Quit),
                            "/npub" | "/n" => Some(Action::CopyNpub),
                            "/help" | "/h" => Some(Action::ShowHelp),
                            "/next" => Some(Action::NextGroup),
                            "/prev" => Some(Action::PrevGroup),
                            "/join" | "/j" if parts.len() > 1 => {
                                Some(Action::JoinGroup(parts[1].to_string()))
                            }
                            _ => {
                                // Unknown command, already cleared input
                                None
                            }
                        }
                    } else {
                        // Regular message
                        Some(Action::SendMessage(input_str))
                    }
                }
                _ => None,
            }
        }
    }
}