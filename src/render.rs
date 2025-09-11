use nrc::app::App;
use nrc::ui_state::{GroupSummary, Message, Modal, OnboardingMode, OpsItem, Page};
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
            &app.error,
        ),
        Page::Help { selected_section } => render_help(f, *selected_section),
        Page::OpsDashboard { items, selected } => render_ops_dashboard(f, items, *selected),
    }

    if let Some(modal) = &app.modal {
        render_modal(f, modal);
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
pub fn render_chat(
    f: &mut Frame,
    groups: &[GroupSummary],
    selected_group_index: usize,
    _group_info: &nrc_mls_storage::groups::types::Group,
    messages: &[Message],
    input: &str,
    scroll_offset: usize,
    flash: &Option<(String, std::time::Instant)>,
    error: &Option<String>,
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

    let groups_list =
        List::new(group_items).block(Block::default().borders(Borders::ALL).title("CHATS"));
    f.render_widget(groups_list, main_chunks[0]);

    // Calculate flash message height if present
    let flash_height = if let Some((msg, expiry)) = flash {
        if std::time::Instant::now() < *expiry {
            // Calculate how many lines we need for the flash message
            let available_width = main_chunks[1].width.saturating_sub(2) as usize;
            let mut line_count = 0;
            let words: Vec<&str> = msg.split_whitespace().collect();
            let mut current_line = String::new();

            for word in &words {
                if current_line.is_empty() {
                    if word.len() <= available_width {
                        current_line = word.to_string();
                    } else {
                        // Word needs to be broken
                        line_count += word.len().div_ceil(available_width);
                        current_line.clear();
                    }
                } else {
                    let test_line = format!("{current_line} {word}");
                    if test_line.len() <= available_width {
                        current_line = test_line;
                    } else {
                        line_count += 1;
                        if word.len() <= available_width {
                            current_line = word.to_string();
                        } else {
                            // Word needs to be broken
                            line_count += word.len().div_ceil(available_width);
                            current_line.clear();
                        }
                    }
                }
            }
            if !current_line.is_empty() {
                line_count += 1;
            }

            // No extra padding needed
            Some(line_count.min(10) as u16) // Cap at 10 lines max
        } else {
            None
        }
    } else {
        None
    };

    // Split chat area vertically - dynamic based on error + flash message
    let mut constraints: Vec<Constraint> = vec![
        Constraint::Min(0), // Messages area
    ];
    let has_error = error.is_some();
    if has_error {
        constraints.push(Constraint::Length(1)); // Single-line error banner
    }
    if let Some(h) = flash_height {
        constraints.push(Constraint::Length(h)); // Flash area
    }
    constraints.push(Constraint::Length(3)); // Input area with "INPUT" label
    let chat_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(main_chunks[1]);

    // Render messages
    let messages_area_index = 0;
    let visible_messages = messages
        .iter()
        .skip(scroll_offset)
        .take(chat_chunks[messages_area_index].height as usize - 2);

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
        Paragraph::new(message_lines).block(Block::default().borders(Borders::ALL).title("CHAT"));
    f.render_widget(messages_widget, chat_chunks[messages_area_index]);

    // Render error banner if present (single line)
    let mut next_slot = 2; // 0=header,1=messages, 2=optional error, then optional flash, then input
    if let Some(err) = error {
        let err_widget = Paragraph::new(err.as_str()).style(Style::default().fg(Color::Red));
        f.render_widget(err_widget, chat_chunks[next_slot]);
        next_slot += 1;
    }

    // Render flash message if active
    if flash_height.is_some() {
        if let Some((msg, _)) = flash {
            // Manually wrap text to fit the available width
            let available_width = chat_chunks[next_slot].width.saturating_sub(2) as usize; // Account for borders
            let mut wrapped_lines = Vec::new();

            // Split message into words and rebuild lines that fit
            let words: Vec<&str> = msg.split_whitespace().collect();
            let mut current_line = String::new();

            for word in words {
                // Check if we need to add this word to current line or start new line
                if current_line.is_empty() {
                    // Starting a new line
                    if word.len() <= available_width {
                        current_line = word.to_string();
                    } else {
                        // Word is too long, need to break it
                        let mut remaining = word;
                        while !remaining.is_empty() {
                            let chunk_size = remaining.len().min(available_width);
                            let (chunk, rest) = remaining.split_at(chunk_size);
                            wrapped_lines.push(
                                Line::from(chunk.to_string())
                                    .style(Style::default().fg(Color::Green)),
                            );
                            remaining = rest;
                        }
                    }
                } else {
                    // Adding to existing line
                    let test_line = format!("{current_line} {word}");
                    if test_line.len() <= available_width {
                        current_line = test_line;
                    } else {
                        // Current line is full, push it and start new one
                        wrapped_lines.push(
                            Line::from(current_line.clone())
                                .style(Style::default().fg(Color::Green)),
                        );

                        // Handle the word for the new line
                        if word.len() <= available_width {
                            current_line = word.to_string();
                        } else {
                            // Word is too long, need to break it
                            let mut remaining = word;
                            while !remaining.is_empty() {
                                let chunk_size = available_width.min(remaining.len());
                                let (chunk, rest) = remaining.split_at(chunk_size);
                                wrapped_lines.push(
                                    Line::from(chunk.to_string())
                                        .style(Style::default().fg(Color::Green)),
                                );
                                remaining = rest;
                            }
                            current_line.clear();
                        }
                    }
                }
            }

            if !current_line.is_empty() {
                wrapped_lines
                    .push(Line::from(current_line).style(Style::default().fg(Color::Green)));
            }

            let flash_widget = Paragraph::new(wrapped_lines);
            f.render_widget(flash_widget, chat_chunks[next_slot]);
        }
    }

    // Render input area with "INPUT" label
    let input_index = chat_chunks.len() - 1;
    let input_widget = Paragraph::new(input)
        .style(Style::default())
        .block(Block::default().borders(Borders::ALL).title("INPUT"));
    f.render_widget(input_widget, chat_chunks[input_index]);
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

fn render_ops_dashboard(f: &mut Frame, items: &[OpsItem], selected: usize) {
    use ratatui::widgets::{Row, Table};

    let size = f.area();
    let header = ["ID", "Kind", "Status", "Updated", "Error"];
    let rows = items.iter().enumerate().map(|(i, it)| {
        let mut id = it.id.clone();
        if id.len() > 8 {
            id = format!("{}…", &id[..8]);
        }
        let updated = chrono::DateTime::<chrono::Utc>::from_timestamp(it.updated_at, 0)
            .map(|dt| dt.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| it.updated_at.to_string());
        let err = it
            .last_error
            .as_ref()
            .map(|e| {
                if e.len() > 20 {
                    format!("{}…", &e[..20])
                } else {
                    e.clone()
                }
            })
            .unwrap_or_default();
        let style = if i == selected {
            Style::default().bg(Color::Blue).fg(Color::White)
        } else {
            Style::default()
        };
        Row::new(vec![id, it.kind.clone(), it.status.clone(), updated, err]).style(style)
    });

    let table = Table::new(rows, [20, 18, 12, 10, 30])
        .header(Row::new(header).style(Style::default().fg(Color::Yellow)))
        .block(Block::default().borders(Borders::ALL).title("Operations"));

    f.render_widget(table, size);
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
