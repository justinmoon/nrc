// Module declarations
pub mod app;
pub mod config;
pub mod events;
pub mod key_storage;
pub mod notification_handler;
pub mod ui_state;
pub mod utils;

// Old modules kept for Nrc compatibility - will be removed when tests are migrated
// pub mod commands;
// pub mod groups;
// pub mod messages;
// pub mod network;
// pub mod profiles;

// Re-export commonly used types
pub use app::App;
pub use config::DEFAULT_RELAYS;
pub use events::AppEvent;
pub use ui_state::{Page, PageType};
pub use utils::pubkey_to_bech32_safe;

// Nrc struct kept for backward compatibility with tests
// TODO: Migrate tests to use App directly
/*
pub struct Nrc {
    pub(crate) storage: Box<NostrMls<NostrMlsSqliteStorage>>,
    pub keys: Keys,
    pub client: Client,
    pub state: types::AppState,
    pub(crate) messages: HashMap<GroupId, Vec<types::Message>>,
    pub welcome_rumors: HashMap<PublicKey, UnsignedEvent>,
    pub groups: HashMap<GroupId, group_types::Group>,
    pub input: String,
    pub selected_group_index: Option<usize>,
    pub scroll_offset: u16,
    pub last_error: Option<String>,
    pub flash_message: Option<String>,
    pub show_help: bool,
    pub help_explicitly_requested: bool,
    pub(crate) profiles: HashMap<PublicKey, Metadata>,
    pub event_tx: Option<mpsc::UnboundedSender<types::AppEvent>>,
    pub command_tx: Option<mpsc::Sender<types::NetworkCommand>>,
    pub(crate) key_storage: KeyStorage,
    pub onboarding_data: types::OnboardingData,
}

*/

// Temporary stub for tests
pub struct Nrc;

/*
impl Nrc {
    pub async fn new(datadir: &Path) -> Result<Self> {
        // Create datadir if it doesn't exist
        std::fs::create_dir_all(datadir)?;

        let key_storage = KeyStorage::new(datadir);

        // Check if we have existing keys
        let (keys, initial_state) = if key_storage.keys_exist() {
            // We have stored keys, prompt for password
            let keys = Keys::generate(); // Temporary, will be replaced when password entered
            (
                keys,
                types::AppState::Onboarding {
                    input: String::new(),
                    mode: types::OnboardingMode::EnterPassword,
                },
            )
        } else {
            // No stored keys, show regular onboarding
            let keys = Keys::generate();
            (
                keys,
                types::AppState::Onboarding {
                    input: String::new(),
                    mode: types::OnboardingMode::Choose,
                },
            )
        };

        let client = Client::builder().signer(keys.clone()).build();

        // Add multiple relays for redundancy
        for &relay in get_default_relays() {
            if let Err(e) = client.add_relay(relay).await {
                log::warn!("Failed to add relay {relay}: {e}");
            }
        }

        client.connect().await;

        // Wait for connections to establish
        tokio::time::sleep(Duration::from_secs(2)).await;

        let db_path = datadir.join("nrc.db");
        log::info!("Using SQLite storage at: {db_path:?}");
        let storage = Box::new(NostrMls::new(NostrMlsSqliteStorage::new(db_path)?));

        Ok(Self {
            storage,
            keys,
            client,
            state: initial_state,
            messages: HashMap::new(),
            welcome_rumors: HashMap::new(),
            groups: HashMap::new(),
            input: String::new(),
            selected_group_index: None,
            scroll_offset: 0,
            last_error: None,
            flash_message: None,
            show_help: false,
            help_explicitly_requested: false,
            profiles: HashMap::new(),
            event_tx: None,
            command_tx: None,
            key_storage,
            onboarding_data: types::OnboardingData {
                display_name: None,
                nsec: None,
            },
        })
    }
}
*/
