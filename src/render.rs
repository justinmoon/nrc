use nrc::app::App;
use nrc::ui_state::{GroupSummary, Message, Modal, OnboardingMode, Page};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

pub fn render(f: &mut Frame, app: &App) {
    match &app.current_page {
        Page::Onboarding { input, mode, error } => render_onboarding(f, input, mode, error),
        Page::Initializing { message, progress } => render_initializing(f, message, *progress),
        Page::Chat {
            groups,
            selected_group_index,
            group_info,
            messages,
            input,
            scroll_offset,
            ..
        } => render_chat(
            f,
            groups,
            *selected_group_index,
            group_info.as_ref(),
            messages,
            input,
            *scroll_offset,
            &app.flash,
        ),
        Page::Help { selected_section } => render_help(f, *selected_section),
    }

    if let Some(modal) = &app.modal {
        render_modal(f, modal);
    }

    if let Some((msg, expiry)) = &app.flash {
        if std::time::Instant::now() < *expiry {
            render_flash(f, msg);
        }
    }
}

fn render_onboarding(f: &mut Frame, input: &str, mode: &OnboardingMode, error: &Option<String>) {
    let size = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(5),    // Content
            Constraint::Length(1), // Error
        ])
        .split(size);

    let title = Paragraph::new("NRC - Nostr Relay Chat")
        .style(Style::default().fg(Color::Green))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, chunks[0]);

    let content = match mode {
        OnboardingMode::Choose => Text::from(vec![
            Line::from("Welcome to NRC!"),
            Line::from(""),
            Line::from("Choose an option:"),
            Line::from("1. Generate new keys"),
            Line::from("2. Import existing keys"),
            Line::from(""),
            Line::from(format!("Your choice: {input}")),
        ]),
        OnboardingMode::GenerateNew => Text::from("Generating new keys..."),
        OnboardingMode::EnterDisplayName => Text::from(vec![
            Line::from("Enter your display name:"),
            Line::from(""),
            Line::from(format!("Name: {input}")),
            Line::from(""),
            Line::from("Press Enter to continue, Esc to go back"),
        ]),
        OnboardingMode::CreatePassword => {
            let masked = "*".repeat(input.len());
            Text::from(vec![
                Line::from("Create a password to encrypt your keys:"),
                Line::from("(minimum 8 characters)"),
                Line::from(""),
                Line::from(format!("Password: {masked}")),
                Line::from(""),
                Line::from("Press Enter to continue, Esc to go back"),
            ])
        }
        OnboardingMode::ImportExisting => Text::from(vec![
            Line::from("Enter your private key (nsec):"),
            Line::from(""),
            Line::from(format!("Key: {}", "*".repeat(input.len()))),
            Line::from(""),
            Line::from("Press Enter to continue, Esc to go back"),
        ]),
        OnboardingMode::EnterPassword => {
            let masked = "*".repeat(input.len());
            Text::from(vec![
                Line::from("Enter your password to unlock keys:"),
                Line::from(""),
                Line::from(format!("Password: {masked}")),
                Line::from(""),
                Line::from("Press Enter to continue"),
            ])
        }
    };

    let content_block = Paragraph::new(content).block(Block::default().borders(Borders::ALL));
    f.render_widget(content_block, chunks[1]);

    if let Some(error_msg) = error {
        let error_widget =
            Paragraph::new(error_msg.as_str()).style(Style::default().fg(Color::Red));
        f.render_widget(error_widget, chunks[2]);
    }
}

fn render_initializing(f: &mut Frame, message: &str, _progress: f32) {
    let size = f.area();

    let text = Text::from(vec![
        Line::from(""),
        Line::from(message),
        Line::from(""),
        Line::from("Please wait..."),
    ]);

    let paragraph = Paragraph::new(text)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::ALL).title("Initializing"));

    let area = centered_rect(50, 25, size);
    f.render_widget(paragraph, area);
}

#[allow(clippy::too_many_arguments)]
fn render_chat(
    f: &mut Frame,
    groups: &[GroupSummary],
    selected_group_index: usize,
    _group_info: &nrc_mls_storage::groups::types::Group,
    messages: &[Message],
    input: &str,
    scroll_offset: usize,
    flash: &Option<(String, std::time::Instant)>,
) {
    let size = f.area();

    // Split horizontally: groups list on left, chat on right
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(30), // Groups sidebar
            Constraint::Min(0),     // Chat area
        ])
        .split(size);

    // Render groups sidebar with "CHATS" header
    let groups_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Groups list
        ])
        .split(main_chunks[0]);

    let groups_header = Paragraph::new("CHATS")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(groups_header, groups_chunks[0]);

    // Render group list
    let group_items: Vec<ListItem> = groups
        .iter()
        .enumerate()
        .map(|(i, group)| {
            let style = if i == selected_group_index {
                Style::default().bg(Color::Blue).fg(Color::White)
            } else {
                Style::default()
            };
            ListItem::new(group.name.clone()).style(style)
        })
        .collect();

    let groups_list = List::new(group_items).block(Block::default().borders(Borders::ALL));
    f.render_widget(groups_list, groups_chunks[1]);

    // Split chat area vertically
    let chat_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header "CHAT"
            Constraint::Min(0),    // Messages area
            Constraint::Length(2), // Flash/error area (hidden when not used)
            Constraint::Length(3), // Input area with "INPUT" label
        ])
        .split(main_chunks[1]);

    // Render chat header
    let chat_header = Paragraph::new("CHAT")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(chat_header, chat_chunks[0]);

    // Render messages
    let visible_messages = messages
        .iter()
        .skip(scroll_offset)
        .take(chat_chunks[1].height as usize - 2);

    let message_lines: Vec<Line> = visible_messages
        .map(|msg| {
            // Format sender with shortened pubkey
            let sender_str = format!("{}", msg.sender);
            let sender_short = if sender_str.len() > 8 {
                format!("{}...", &sender_str[..8])
            } else {
                sender_str
            };
            Line::from(format!("{}: {}", sender_short, msg.content))
        })
        .collect();

    let messages_widget =
        Paragraph::new(message_lines).block(Block::default().borders(Borders::ALL));
    f.render_widget(messages_widget, chat_chunks[1]);

    // Render flash/error area if there's a message
    if let Some((msg, expiry)) = flash {
        if std::time::Instant::now() < *expiry {
            let flash_widget = Paragraph::new(msg.as_str())
                .style(Style::default().fg(Color::Yellow))
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(flash_widget, chat_chunks[2]);
        }
    }

    // Render input area with "INPUT" label
    let input_widget = Paragraph::new(input)
        .style(Style::default())
        .block(Block::default().borders(Borders::ALL).title("INPUT"));
    f.render_widget(input_widget, chat_chunks[3]);
}

fn render_help(f: &mut Frame, _selected_section: usize) {
    let size = f.area();

    let text = Text::from(vec![
        Line::from("NRC Help"),
        Line::from(""),
        Line::from("Navigation:"),
        Line::from("  ↑/↓: Navigate lists"),
        Line::from("  Enter: Select/Join"),
        Line::from("  Esc: Go back"),
        Line::from(""),
        Line::from("Shortcuts:"),
        Line::from("  Ctrl+N: New group"),
        Line::from("  Ctrl+S: Settings"),
        Line::from("  F1: This help"),
        Line::from(""),
        Line::from("Press any key to close help"),
    ]);

    let paragraph =
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Help"));
    f.render_widget(paragraph, size);
}

fn render_modal(f: &mut Frame, modal: &Modal) {
    let size = f.area();
    let area = centered_rect(60, 20, size);

    f.render_widget(Clear, area);

    let text = match modal {
        Modal::Confirm { message, .. } => Text::from(vec![
            Line::from(message.as_str()),
            Line::from(""),
            Line::from("Y/N to confirm/cancel"),
        ]),
        Modal::Error { message } => Text::from(vec![
            Line::from(message.as_str()),
            Line::from(""),
            Line::from("Press any key to continue"),
        ]),
        Modal::Info { message } => Text::from(vec![
            Line::from(message.as_str()),
            Line::from(""),
            Line::from("Press any key to continue"),
        ]),
    };

    let block = Block::default().borders(Borders::ALL).title("Modal");
    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, area);
}

fn render_flash(f: &mut Frame, message: &str) {
    let size = f.area();
    let area = Rect {
        x: 0,
        y: 0,
        width: size.width,
        height: 1,
    };

    let paragraph =
        Paragraph::new(message).style(Style::default().bg(Color::Yellow).fg(Color::Black));
    f.render_widget(paragraph, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
