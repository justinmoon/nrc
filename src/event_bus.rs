use anyhow::Result;
use crossterm::event::KeyEvent;
use nostr_sdk::prelude::*;
use openmls::group::GroupId;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum UnifiedEvent {
    // User Input Events
    KeyPress(KeyEvent),
    Paste(String),
    Command(String),

    // Nostr Events
    NostrEventReceived(Event),
    NostrEventToSend(Event),
    NostrSubscribe(Filter),
    NostrFetch {
        filter: Filter,
        request_id: Uuid,
    },

    // MLS Events
    MlsMessageReceived {
        group_id: GroupId,
        content: String,
        sender: PublicKey,
        timestamp: Timestamp,
    },
    MlsWelcomeReceived {
        welcome: UnsignedEvent,
    },
    MlsKeyPackageRequest {
        pubkey: PublicKey,
        request_id: Uuid,
    },

    // Application State Events
    InitializeApp {
        display_name: Option<String>,
        nsec: Option<String>,
    },

    // Onboarding Events
    OnboardingChooseOption {
        option: u8,
    }, // 1=new, 2=import
    OnboardingInputChar {
        c: char,
    },
    OnboardingBackspace,
    OnboardingSubmit {
        input: String,
    },
    OnboardingEscape,
    OnboardingComplete,
    Quit,
    ShowHelp,
    CopyNpub,
    NextGroup,
    PrevGroup,
    CreateGroup {
        name: String,
        request_id: Uuid,
    },
    JoinGroup {
        npub: String,
        request_id: Uuid,
    },
    JoinGroupCommand {
        npub: String,
    },
    FetchKeyPackage {
        pubkey: PublicKey,
        request_id: Uuid,
    },
    KeyPackageFetched {
        pubkey: PublicKey,
        key_package: Event,
        request_id: Uuid,
    },
    SendMessage {
        group_id: GroupId,
        content: String,
        request_id: Uuid,
    },
    PublishKeyPackage {
        request_id: Uuid,
    },
    PublishProfile {
        display_name: String,
        request_id: Uuid,
    },

    // Network Operation Results
    NostrEventSent {
        event_id: EventId,
        request_id: Option<Uuid>,
    },
    NostrEventFetchComplete {
        request_id: Uuid,
        events: Vec<Event>,
    },
    NostrSubscriptionUpdate {
        filter: Filter,
        event: Event,
    },
    KeyPackagePublished {
        request_id: Uuid,
    },
    ProfilePublished {
        request_id: Uuid,
    },
    GroupCreated {
        group_id: GroupId,
        request_id: Uuid,
    },
    GroupJoined {
        group_id: GroupId,
        request_id: Uuid,
    },
    MessageSent {
        group_id: GroupId,
        request_id: Uuid,
    },

    // UI Events
    UpdateUI {
        component: UIComponent,
    },
    ShowError {
        error: String,
    },
    ShowFlashMessage {
        message: String,
        duration_ms: u64,
    },
    RefreshDisplay,
    SetInputMode {
        mode: InputMode,
    },
    UpdateInputBuffer {
        content: String,
    },

    // Timer Events
    ProcessPendingOperations,
    RetryOperation {
        operation_id: Uuid,
    },
    FetchMessages,
    FetchWelcomes,

    // System Events
    Shutdown,
    Error {
        error: String,
        context: String,
    },
}

#[derive(Debug, Clone)]
pub enum UIComponent {
    MessageList,
    GroupList,
    InputField,
    StatusBar,
    All,
}

#[derive(Debug, Clone)]
pub enum InputMode {
    Normal,
    Command,
    Onboarding(OnboardingMode),
}

#[derive(Debug, Clone)]
pub enum OnboardingMode {
    Choose,
    GenerateNew,
    EnterDisplayName,
    ImportExisting,
}

pub struct EventBus {
    sender: mpsc::UnboundedSender<UnifiedEvent>,
    receiver: Option<mpsc::UnboundedReceiver<UnifiedEvent>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            sender,
            receiver: Some(receiver),
        }
    }

    pub fn sender(&self) -> mpsc::UnboundedSender<UnifiedEvent> {
        self.sender.clone()
    }

    pub fn take_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<UnifiedEvent>> {
        self.receiver.take()
    }

    pub fn emit(&self, event: UnifiedEvent) -> Result<()> {
        self.sender
            .send(event)
            .map_err(|e| anyhow::anyhow!("Failed to send event: {}", e))
    }
}

// EventHandler trait removed - we'll use direct methods on Nrc instead

// EventRouter removed - routing will be done directly in Nrc
