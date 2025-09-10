mod keyboard;
mod render;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nrc::{
    ui_state::{OnboardingMode, Page},
    App, AppEvent,
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
    /// Watch operations dashboard mode
    #[arg(long, default_value_t = false)]
    watch_ops: bool,
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

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, &args.datadir, args.watch_ops).await;

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
    datadir: &Path,
    watch_ops: bool,
) -> Result<()> {
    use nostr_sdk::prelude::*;
    use nrc::config::get_default_relays;
    use nrc_mls::NostrMls;
    use nrc_mls_sqlite_storage::NostrMlsSqliteStorage;

    let key_storage = nrc::key_storage::KeyStorage::new(datadir);

    let (keys, initial_page) = if watch_ops {
        let keys = Keys::generate();
        (
            keys,
            Page::OpsDashboard {
                items: vec![],
                selected: 0,
            },
        )
    } else if key_storage.keys_exist() {
        let keys = Keys::generate();
        (
            keys,
            Page::Onboarding {
                input: String::new(),
                mode: OnboardingMode::EnterPassword,
                error: None,
            },
        )
    } else {
        let keys = Keys::generate();
        (
            keys,
            Page::Onboarding {
                input: String::new(),
                mode: OnboardingMode::Choose,
                error: None,
            },
        )
    };

    let client = Client::builder().signer(keys.clone()).build();

    for &relay in get_default_relays() {
        if let Err(e) = client.add_relay(relay).await {
            log::warn!("Failed to add relay {relay}: {e}");
        }
    }

    client.connect().await;
    tokio::time::sleep(Duration::from_secs(2)).await;

    let db_path = datadir.join("nrc.db");
    #[allow(clippy::arc_with_non_send_sync)]
    let storage = Arc::new(NostrMls::new(NostrMlsSqliteStorage::new(db_path)?));

    let mut app = App::new(
        storage.clone(),
        client.clone(),
        keys,
        key_storage,
        initial_page,
    )
    .await?;

    let mut state_rx = app.get_state_receiver();
    let event_rx = app.event_rx.take().unwrap();

    let event_tx = app.event_tx.clone();
    keyboard::spawn_keyboard_listener(event_tx.clone());

    let ops_event_tx = event_tx.clone();
    tokio::spawn(async move {
        use tokio::time::{interval, Duration};
        let mut pending_ops_interval = if watch_ops {
            interval(Duration::from_millis(500))
        } else {
            interval(Duration::from_secs(30))
        };

        loop {
            pending_ops_interval.tick().await;
            let _ = ops_event_tx.send(AppEvent::ProcessPendingOperationsTick);
            if watch_ops {
                let _ = ops_event_tx.send(AppEvent::RefreshCurrentPage);
            }
        }
    });

    let mut last_rendered_state: Option<Page> = None;
    let mut event_rx = event_rx;

    loop {
        let should_render = if state_rx.has_changed().unwrap_or(false) {
            let state = state_rx.borrow_and_update().clone();
            let changed = last_rendered_state.as_ref() != Some(&state);
            if changed {
                last_rendered_state = Some(state);
            }
            changed
        } else {
            false
        };

        if should_render {
            terminal.draw(|f| render::render(f, &app))?;
        }

        match timeout(Duration::from_millis(16), event_rx.recv()).await {
            Ok(Some(event)) => match event {
                AppEvent::KeyPress(key) => {
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        return Ok(());
                    }
                    app.handle_event(event).await?;
                }
                _ => {
                    app.handle_event(event).await?;
                }
            },
            Ok(None) => break,
            Err(_) => {}
        }

        if app
            .flash
            .as_ref()
            .is_some_and(|(_, expiry)| std::time::Instant::now() >= *expiry)
        {
            app.send_event(AppEvent::ClearFlash)?;
        }
    }

    Ok(())
}
