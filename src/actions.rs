use crossterm::event::KeyEvent;
use nostr_sdk::prelude::*;

#[derive(Debug, Clone)]
pub enum Action {
    // Onboarding actions
    OnboardingChoice(OnboardingChoice),
    SetDisplayName(String),
    SetPassword(String),
    SetNsec(String),
    
    // Chat actions
    SendMessage(String),
    JoinGroup(String), // npub
    CreateGroup(String), // name
    
    // Navigation
    NextGroup,
    PrevGroup,
    ScrollUp,
    ScrollDown,
    
    // UI actions
    SetInput(String),
    ClearInput,
    Backspace,
    
    // Commands
    CopyNpub,
    ShowHelp,
    DismissHelp,
    Quit,
    
    // Network actions (internal)
    PublishKeyPackage,
    PublishProfile(String),
    FetchMessages,
    FetchWelcomes,
    ProcessPendingOperations,
    
    // Raw events from network
    NostrEventReceived(Event),
    
    // Keyboard events (raw)
    KeyPress(KeyEvent),
    Paste(String),
}

#[derive(Debug, Clone)]
pub enum OnboardingChoice {
    GenerateNew,
    ImportExisting,
    Continue, // For multi-step onboarding
}