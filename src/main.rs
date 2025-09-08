mod keyboard;
mod tui;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nrc::{AppEvent, AppState, NetworkCommand, Nrc, OnboardingData, OnboardingMode};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

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

    let mut nrc = Nrc::new(&args.datadir).await?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, &mut nrc).await;

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
    nrc: &mut Nrc,
) -> Result<()> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let (command_tx, command_rx) = mpsc::channel(100);
    let (storage_tx, mut storage_rx) = mpsc::channel::<(
        nrc::network_task::StorageCommand,
        tokio::sync::oneshot::Sender<nrc::network_task::StorageResponse>,
    )>(100);

    // Store channels in Nrc
    nrc.event_tx = Some(event_tx.clone());
    nrc.command_tx = Some(command_tx.clone());

    // Spawn network task
    nrc::network_task::spawn_network_task(
        command_rx,
        storage_tx,
        event_tx.clone(),
        nrc.keys.clone(),
    )
    .await;

    // Spawn event producers
    keyboard::spawn_keyboard_listener(event_tx.clone());
    // Start real-time notification handler for subscriptions
    nrc::notification_handler::spawn_notification_handler(nrc.client.clone(), event_tx.clone());

    // Start timer for pending operations processing only
    let ops_event_tx = event_tx.clone();
    tokio::spawn(async move {
        use tokio::time::{interval, Duration};
        let mut pending_ops_interval = interval(Duration::from_secs(30));

        loop {
            pending_ops_interval.tick().await;
            let _ = ops_event_tx.send(AppEvent::ProcessPendingOperationsTick);
        }
    });

    // Main event loop - THE ONLY PLACE WHERE STATE CHANGES
    loop {
        // Draw UI
        terminal.draw(|f| tui::draw(f, nrc))?;

        // Handle storage commands from network task
        while let Ok((cmd, tx)) = storage_rx.try_recv() {
            let response = nrc.handle_storage_command(cmd).await;
            let _ = tx.send(response);
        }

        // Process events with small timeout for refresh rate
        match timeout(Duration::from_millis(50), event_rx.recv()).await {
            Ok(Some(event)) => {
                match event {
                    AppEvent::KeyPress(key) => {
                        if handle_key_press(nrc, key, &command_tx).await? {
                            return Ok(()); // Quit
                        }
                    }
                    AppEvent::Paste(text) => {
                        handle_paste(nrc, text);
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
                    // Timer-based fetch events removed - now handled by subscription notifications
                    AppEvent::ProcessPendingOperationsTick => {
                        // Reserved for future persistent retry functionality
                        log::debug!("Pending operations tick - no operations to process");
                    }
                    AppEvent::RawMessagesReceived { events } => {
                        // Process the fetched messages in the main loop
                        log::debug!("Processing {} fetched message events", events.len());
                        for event in events {
                            // Process each event - this is fast since it's just decryption
                            if let Err(e) = nrc.process_message_event(event).await {
                                log::debug!("Failed to process message: {e}");
                            }
                        }
                    }
                    AppEvent::RawWelcomesReceived { events } => {
                        // Process the fetched welcomes in the main loop
                        log::debug!("Processing {} fetched welcome events", events.len());
                        for event in events {
                            if let Err(e) = nrc.process_welcome_event(event).await {
                                log::debug!("Failed to process welcome: {e}");
                            }
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
            Err(_) => {}       // Timeout - just redraw
        }
    }

    Ok(())
}

fn handle_paste(nrc: &mut Nrc, text: String) {
    // Only handle paste in Ready state
    if matches!(nrc.state, AppState::Ready { .. }) {
        nrc.input.push_str(&text);
        nrc.clear_error(); // Clear error on new input
        log::debug!("Pasted text: '{}', Input now: '{}'", text, nrc.input);
    } else if let AppState::Onboarding { ref mut input, .. } = nrc.state {
        // Also handle paste during onboarding
        input.push_str(&text);
    }
}

async fn handle_key_press(
    nrc: &mut Nrc,
    key: KeyEvent,
    _command_tx: &mpsc::Sender<NetworkCommand>,
) -> Result<bool> {
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
                    if key.code == KeyCode::Esc {
                        nrc.state = AppState::Onboarding {
                            input,
                            mode: OnboardingMode::Choose,
                        };
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
                            // Store display name for later use
                            nrc.onboarding_data.display_name = Some(new_input);
                            // Move to password creation
                            nrc.state = AppState::Onboarding {
                                input: String::new(),
                                mode: OnboardingMode::CreatePassword,
                            };
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
                            // Store nsec for later use
                            nrc.onboarding_data.nsec = Some(new_input);
                            // Move to password creation
                            nrc.state = AppState::Onboarding {
                                input: String::new(),
                                mode: OnboardingMode::CreatePassword,
                            };
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
                OnboardingMode::CreatePassword => {
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

                            // Check if we have a display name (new user) or nsec (import)
                            if let Some(display_name) = nrc.onboarding_data.display_name.clone() {
                                // New user with display name
                                nrc.initialize_with_display_name_and_password(
                                    display_name,
                                    new_input,
                                )
                                .await?;
                            } else if let Some(nsec) = nrc.onboarding_data.nsec.clone() {
                                // Import with nsec
                                nrc.initialize_with_nsec_and_password(nsec, new_input)
                                    .await?;
                            }
                            // Clear onboarding data
                            nrc.onboarding_data = OnboardingData {
                                display_name: None,
                                nsec: None,
                            };
                        }
                        KeyCode::Esc => {
                            // Go back to previous state
                            if let Some(display_name) = nrc.onboarding_data.display_name.take() {
                                // Was entering display name
                                nrc.state = AppState::Onboarding {
                                    input: display_name,
                                    mode: OnboardingMode::EnterDisplayName,
                                };
                            } else if let Some(nsec) = nrc.onboarding_data.nsec.take() {
                                // Was importing nsec
                                nrc.state = AppState::Onboarding {
                                    input: nsec,
                                    mode: OnboardingMode::ImportExisting,
                                };
                            } else {
                                nrc.state = AppState::Onboarding {
                                    input: String::new(),
                                    mode: OnboardingMode::Choose,
                                };
                            }
                        }
                        _ => {}
                    }
                }
                OnboardingMode::EnterPassword => {
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
                            if let Err(e) = nrc.initialize_with_password(new_input).await {
                                // Failed to decrypt - show error and stay in password prompt
                                nrc.last_error = Some(format!("Invalid password: {e}"));
                                nrc.state = AppState::Onboarding {
                                    input: String::new(),
                                    mode: OnboardingMode::EnterPassword,
                                };
                            }
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
