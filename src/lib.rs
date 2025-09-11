// Module declarations
pub mod app;
pub mod config;
pub mod events;
pub mod key_storage;
pub mod notification_handler;
pub mod ops;
pub mod profiles;
pub mod ui_state;
pub mod utils;

// Re-export commonly used types
pub use app::App;
pub use config::DEFAULT_RELAYS;
pub use events::AppEvent;
pub use ui_state::{Page, PageType};
pub use utils::pubkey_to_bech32_safe;
