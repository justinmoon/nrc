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

    GroupList {
        groups: Vec<GroupSummary>,
        selected_index: usize,
        filter: Option<String>,
    },

    Chat {
        group_id: GroupId,
        group_info: Box<group_types::Group>,
        messages: Vec<Message>,
        members: Vec<Member>,
        input: String,
        scroll_offset: usize,
        typing_members: Vec<PublicKey>,
    },

    CreateGroup {
        name_input: String,
        member_search: String,
        selected_members: Vec<PublicKey>,
        available_members: Vec<Contact>,
    },

    Settings {
        current_settings: UserSettings,
        edited_settings: UserSettings,
        selected_field: SettingField,
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
    GenerateNew,
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
    GroupList,
    Chat(GroupId),
    CreateGroup,
    Settings,
    Help,
}

impl Page {
    pub fn page_type(&self) -> PageType {
        match self {
            Page::Onboarding { .. } => PageType::Onboarding,
            Page::Initializing { .. } => PageType::Initializing,
            Page::GroupList { .. } => PageType::GroupList,
            Page::Chat { group_id, .. } => PageType::Chat(group_id.clone()),
            Page::CreateGroup { .. } => PageType::CreateGroup,
            Page::Settings { .. } => PageType::Settings,
            Page::Help { .. } => PageType::Help,
        }
    }
}
