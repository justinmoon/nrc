mod tui;
mod keyboard;
// mod network_task;  // TODO: Enable once storage can be shared
mod timer_task;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nrc::{AppState, AppEvent, NetworkCommand, Nrc, OnboardingMode};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::io;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Data directory for logs and other files
    #[arg(long, default_value = ".")]
    datadir: PathBuf,
    
    /// Use memory storage instead of SQLite
    #[arg(long)]
    memory: bool,
}

fn setup_logging(datadir: &PathBuf) -> Result<()> {
    use std::io::Write;
    use env_logger::Builder;
    use log::LevelFilter;
    
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
    log::info!("Starting NRC with datadir: {:?}, memory: {}", args.datadir, args.memory);
    
    let mut nrc = Nrc::new(&args.datadir, args.memory).await?;
    
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    
    let res = run_app(&mut terminal, &mut nrc).await;
    
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    
    if let Err(err) = res {
        eprintln!("Error: {err:?}");
    }
    
    Ok(())
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    nrc: &mut Nrc,
) -> Result<()> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let (command_tx, _command_rx) = mpsc::channel(100);
    
    // Store channels in Nrc
    nrc.event_tx = Some(event_tx.clone());
    nrc.command_tx = Some(command_tx.clone());
    
    // Spawn event producers
    keyboard::spawn_keyboard_listener(event_tx.clone());
    timer_task::spawn_timer_task(event_tx.clone()).await;
    
    // Note: We'll need to create network task differently since we can't clone storage
    // For now, we'll handle network commands directly in the main loop
    
    // Main event loop - THE ONLY PLACE WHERE STATE CHANGES
    loop {
        // Draw UI
        terminal.draw(|f| tui::draw(f, nrc))?;
        
        // Process events with small timeout for refresh rate
        match timeout(Duration::from_millis(50), event_rx.recv()).await {
            Ok(Some(event)) => {
                match event {
                    AppEvent::KeyPress(key) => {
                        if handle_key_press(nrc, key, &command_tx).await? {
                            return Ok(()); // Quit
                        }
                    }
                    AppEvent::MessageReceived { group_id, message } => {
                        nrc.add_message(group_id, message);
                    }
                    AppEvent::GroupCreated { group_id } => {
                        nrc.add_group(group_id);
                    }
                    AppEvent::NetworkError { error } => {
                        nrc.last_error = Some(error);
                    }
                    AppEvent::FetchMessagesTick => {
                        // Directly call the fetch method for now
                        if let Err(e) = nrc.fetch_and_process_messages().await {
                            log::error!("Failed to fetch messages: {}", e);
                        }
                    }
                    AppEvent::FetchWelcomesTick => {
                        // Directly call the fetch method for now
                        if let Err(e) = nrc.fetch_and_process_welcomes().await {
                            log::error!("Failed to fetch welcomes: {}", e);
                        }
                    }
                    AppEvent::KeyPackagePublished => {
                        if let AppState::Ready { groups, .. } = &nrc.state {
                            nrc.state = AppState::Ready {
                                key_package_published: true,
                                groups: groups.clone(),
                            };
                        }
                    }
                    _ => {}
                }
            }
            Ok(None) => break, // Channel closed
            Err(_) => {} // Timeout - just redraw
        }
    }
    
    Ok(())
}

async fn handle_key_press(nrc: &mut Nrc, key: KeyEvent, _command_tx: &mpsc::Sender<NetworkCommand>) -> Result<bool> {
    // Only allow Ctrl+C for emergency exit
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(true);
    }
    
    let state_clone = nrc.state.clone();
    match state_clone {
        AppState::Onboarding { input, mode } => {
            match mode {
                OnboardingMode::Choose => {
                    match key.code {
                        KeyCode::Char('1') => {
                            // Move to display name entry
                            nrc.state = AppState::Onboarding {
                                input: String::new(),
                                mode: OnboardingMode::EnterDisplayName,
                            };
                        }
                        KeyCode::Char('2') => {
                            nrc.state = AppState::Onboarding {
                                input: String::new(),
                                mode: OnboardingMode::ImportExisting,
                            };
                        }
                        KeyCode::Esc => return Ok(true),
                        _ => {}
                    }
                }
                OnboardingMode::GenerateNew => {
                    // This mode is no longer used since we generate immediately
                    match key.code {
                        KeyCode::Esc => {
                            nrc.state = AppState::Onboarding {
                                input,
                                mode: OnboardingMode::Choose,
                            };
                        }
                        _ => {}
                    }
                }
                OnboardingMode::EnterDisplayName => {
                    let mut new_input = input.clone();
                    match key.code {
                        KeyCode::Char(c) => {
                            new_input.push(c);
                            nrc.state = AppState::Onboarding {
                                input: new_input,
                                mode,
                            };
                        }
                        KeyCode::Backspace => {
                            new_input.pop();
                            nrc.state = AppState::Onboarding {
                                input: new_input,
                                mode,
                            };
                        }
                        KeyCode::Enter if !new_input.is_empty() => {
                            // Initialize with the display name
                            nrc.state = AppState::Initializing;
                            nrc.initialize_with_display_name(new_input).await?;
                        }
                        KeyCode::Esc => {
                            nrc.state = AppState::Onboarding {
                                input: String::new(),
                                mode: OnboardingMode::Choose,
                            };
                        }
                        _ => {}
                    }
                }
                OnboardingMode::ImportExisting => {
                    let mut new_input = input.clone();
                    match key.code {
                        KeyCode::Char(c) => {
                            new_input.push(c);
                            nrc.state = AppState::Onboarding {
                                input: new_input,
                                mode,
                            };
                        }
                        KeyCode::Backspace => {
                            new_input.pop();
                            nrc.state = AppState::Onboarding {
                                input: new_input,
                                mode,
                            };
                        }
                        KeyCode::Enter if !new_input.is_empty() => {
                            nrc.state = AppState::Initializing;
                            nrc.initialize_with_nsec(new_input).await?;
                        }
                        KeyCode::Esc => {
                            nrc.state = AppState::Onboarding {
                                input: String::new(),
                                mode: OnboardingMode::Choose,
                            };
                        }
                        _ => {}
                    }
                }
            }
        }
        AppState::Initializing => {}
        AppState::Ready { .. } => {
            // If help is showing, any key dismisses it
            if nrc.show_help {
                nrc.dismiss_help();
                return Ok(false);
            }
            
            match key.code {
                // Arrow keys for navigation (only when input is empty)
                KeyCode::Up if nrc.input.is_empty() => {
                    nrc.prev_group();
                }
                KeyCode::Down if nrc.input.is_empty() => {
                    nrc.next_group();
                }
                // Ctrl+j/k for navigation  
                KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    nrc.next_group();
                }
                KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    nrc.prev_group();
                }
                // Regular character input
                KeyCode::Char(c) => {
                    nrc.input.push(c);
                    nrc.clear_error(); // Clear error on new input
                    log::debug!("Input after char '{}': '{}'", c, nrc.input);
                }
                KeyCode::Backspace => {
                    nrc.input.pop();
                }
                KeyCode::Enter if !nrc.input.is_empty() => {
                    let input = nrc.input.clone();
                    nrc.input.clear();
                    if nrc.process_input(input).await? {
                        return Ok(true); // Quit was requested
                    }
                }
                _ => {}
            }
        }
    }
    
    Ok(false)
}