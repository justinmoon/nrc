use crossterm::event::KeyEvent;
use nostr_sdk::prelude::*;
use openmls::group::GroupId;
use std::time::Duration;

use crate::ui_state::{Member, Message, Page};

#[derive(Debug, Clone)]
pub enum AppEvent {
    Navigate(Page),
    NavigateBack,

    KeyPress(KeyEvent),
    Paste(String),
    SendMessage(String),
    CreateGroup {
        name: String,
        members: Vec<PublicKey>,
    },
    JoinGroup(GroupId),
    LeaveGroup(GroupId),

    MessageReceived {
        group_id: GroupId,
        message: Message,
    },
    GroupInviteReceived {
        invite: GroupInvite,
    },
    MemberJoined {
        group_id: GroupId,
        member: Member,
    },
    MemberLeft {
        group_id: GroupId,
        member: PublicKey,
    },
    ConnectionStatusChanged(ConnectionStatus),

    RefreshCurrentPage,
    FlashMessage(String, Duration),
    ClearFlash,

    WelcomeReceived {
        welcome: UnsignedEvent,
    },
    GroupCreated {
        group_id: GroupId,
    },
    KeyPackagePublished,
    ProfilePublished,
    NetworkError {
        error: String,
    },

    ProcessPendingOperationsTick,

    RawMessagesReceived {
        events: Vec<Event>,
    },
    RawWelcomesReceived {
        events: Vec<Event>,
    },
}

#[derive(Debug, Clone)]
pub struct GroupInvite {
    pub group_id: GroupId,
    pub inviter: PublicKey,
    pub group_name: String,
}

#[derive(Debug, Clone)]
pub enum ConnectionStatus {
    Connected,
    Connecting,
    Disconnected,
    Error(String),
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
