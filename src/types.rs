use crossterm::event::KeyEvent;
use nostr_sdk::prelude::*;
use openmls::group::GroupId;

#[derive(Debug, Clone)]
pub enum AppEvent {
    // UI Events
    KeyPress(KeyEvent),
    Paste(String),

    // Network Events (from background task)
    MessageReceived { group_id: GroupId, message: Message },
    WelcomeReceived { welcome: UnsignedEvent },
    GroupCreated { group_id: GroupId },
    KeyPackagePublished,
    ProfilePublished,
    NetworkError { error: String },

    // Timer Events
    ProcessPendingOperationsTick,

    // Raw network data to be processed
    RawMessagesReceived { events: Vec<Event> },
    RawWelcomesReceived { events: Vec<Event> },
}

#[derive(Debug, Clone)]
pub enum NetworkCommand {
    SendMessage { group_id: GroupId, content: String },
    JoinGroup { npub: String },
    CreateGroup { name: String },
    PublishKeyPackage,
    PublishProfile { display_name: String },
    FetchMessages,
    FetchWelcomes,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub content: String,
    pub sender: PublicKey,
    pub timestamp: Timestamp,
}

#[derive(Debug, Clone)]
pub enum AppState {
    Onboarding {
        input: String,
        mode: OnboardingMode,
    },
    Initializing,
    Ready {
        key_package_published: bool,
        groups: Vec<GroupId>,
    },
}

#[derive(Debug, Clone)]
pub enum OnboardingMode {
    Choose,
    GenerateNew,
    EnterDisplayName,
    CreatePassword,
    ImportExisting,
    EnterPassword,
}

/// Temporary data during onboarding
#[derive(Debug, Clone)]
pub struct OnboardingData {
    pub display_name: Option<String>,
    pub nsec: Option<String>,
}
