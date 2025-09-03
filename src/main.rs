mod tui;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nrc::{AppState, Nrc, OnboardingMode};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::io;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use tokio::time::{Duration, Instant};

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
    let mut last_tick = Instant::now();
    let mut last_message_fetch = Instant::now();
    let mut last_welcome_fetch = Instant::now();
    let tick_rate = Duration::from_millis(250);
    let message_fetch_interval = Duration::from_secs(2); // Fetch messages every 2 seconds
    let welcome_fetch_interval = Duration::from_secs(3); // Fetch welcomes every 3 seconds
    
    loop {
        terminal.draw(|f| tui::draw(f, nrc))?;
        
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));
        
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if handle_key_event(key, nrc).await? {
                    return Ok(());
                }
            }
        }
        
        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
            
            // Check if we should fetch messages
            if last_message_fetch.elapsed() >= message_fetch_interval {
                // Fetch messages in the background
                if let Err(e) = nrc.fetch_and_process_messages().await {
                    log::error!("Failed to fetch messages: {}", e);
                }
                last_message_fetch = Instant::now();
            }
            
            // Check if we should fetch welcomes (for auto-joining groups)
            if last_welcome_fetch.elapsed() >= welcome_fetch_interval {
                // Fetch welcomes to auto-join groups when invited
                if let Err(e) = nrc.fetch_and_process_welcomes().await {
                    log::error!("Failed to fetch welcomes: {}", e);
                }
                last_welcome_fetch = Instant::now();
            }
        }
    }
}

async fn handle_key_event(key: KeyEvent, nrc: &mut Nrc) -> Result<bool> {
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
                            // Generate new key immediately
                            nrc.state = AppState::Initializing;
                            nrc.initialize().await?;
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