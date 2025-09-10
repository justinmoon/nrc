use nostr_sdk::prelude::*;
use nrc_mls_storage::groups::types as group_types;
use openmls::group::GroupId;

#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Onboarding {
        input: String,
        mode: OnboardingMode,
        error: Option<String>,
    },

    Initializing {
        message: String,
        progress: f32,
    },

    Chat {
        groups: Vec<GroupSummary>,   // All groups for left sidebar
        selected_group_index: usize, // Which group is selected in sidebar
        group_id: GroupId,
        group_info: Box<group_types::Group>,
        messages: Vec<Message>,
        members: Vec<Member>,
        input: String,
        scroll_offset: usize,
        typing_members: Vec<PublicKey>,
    },

    Help {
        selected_section: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupSummary {
    pub id: GroupId,
    pub name: String,
    pub member_count: usize,
    pub last_message: Option<Message>,
    pub unread_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub content: String,
    pub sender: PublicKey,
    pub timestamp: Timestamp,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub public_key: PublicKey,
    pub display_name: Option<String>,
    pub metadata: Option<Metadata>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub public_key: PublicKey,
    pub display_name: Option<String>,
    pub metadata: Option<Metadata>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UserSettings {
    pub display_name: String,
    pub relays: Vec<String>,
    pub notification_enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingField {
    DisplayName,
    Relays,
    Notifications,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OnboardingMode {
    Choose,
    EnterDisplayName,
    CreatePassword,
    ImportExisting,
    EnterPassword,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    Confirm {
        message: String,
        on_confirm: ModalAction,
    },
    Error {
        message: String,
    },
    Info {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModalAction {
    LeaveGroup(GroupId),
    DeleteMessage(usize),
    ClearChat(GroupId),
}

pub enum PageType {
    Onboarding,
    Initializing,
    Chat(Option<GroupId>),
    Help,
}

impl Page {
    pub fn page_type(&self) -> PageType {
        match self {
            Page::Onboarding { .. } => PageType::Onboarding,
            Page::Initializing { .. } => PageType::Initializing,
            Page::Chat { group_id, .. } => PageType::Chat(Some(group_id.clone())),
            Page::Help { .. } => PageType::Help,
        }
    }
}
