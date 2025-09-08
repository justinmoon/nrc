use nostr_sdk::prelude::ToBech32;
use nrc::evented_nrc::{EventedNrc, UIState};
use nrc::{AppState, OnboardingMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

/// Main draw function for EventedNrc
pub fn draw_evented(f: &mut Frame, evented: &EventedNrc) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0)])
        .split(f.area());

    let ui_state = evented.ui_state.borrow();
    
    match &ui_state.app_state {
        AppState::Onboarding { mode, input } => {
            draw_onboarding(f, chunks[0], input, mode, ui_state.last_error.as_deref());
        }
        AppState::Initializing => {
            draw_initializing(f, chunks[0]);
        }
        AppState::Ready { groups, .. } => {
            draw_ready_view_with_state(f, chunks[0], &ui_state, &evented.npub, groups);
        }
    }
}

/// Helper to draw password input field
fn draw_password_input(
    f: &mut Frame,
    area: Rect,
    prompt_lines: Vec<Line>,
    input: &str,
    help_text: Vec<Line>,
) {
    let prompt_len = prompt_lines.len();
    let paragraph = Paragraph::new(prompt_lines)
        .style(Style::default())
        .alignment(Alignment::Center);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(prompt_len as u16 + 2),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    f.render_widget(paragraph, chunks[0]);

    // Hide password input
    let masked_input = "*".repeat(input.len());
    let input_box = Paragraph::new(masked_input)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .style(Style::default().fg(Color::White));

    let input_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(chunks[1]);

    // Center the password input box
    let centered_input = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(35),
            Constraint::Percentage(30),
            Constraint::Percentage(35),
        ])
        .split(input_area[0]);

    f.render_widget(input_box, centered_input[1]);

    let help = Paragraph::new(help_text).alignment(Alignment::Center);
    f.render_widget(help, chunks[2]);
}

fn draw_onboarding(
    f: &mut Frame,
    area: Rect,
    input: &str,
    mode: &OnboardingMode,
    error: Option<&str>,
) {
    let block = Block::default()
        .title("╔═══ NRC - ONBOARDING ═══╗")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(10),
            Constraint::Min(0),
        ])
        .split(inner);

    match mode {
        OnboardingMode::Choose => {
            let content = vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    "WELCOME TO NRC",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(""),
                Line::from("Choose an option:"),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[1] ", Style::default().fg(Color::Green)),
                    Span::raw("Generate new keys"),
                ]),
                Line::from(vec![
                    Span::styled("[2] ", Style::default().fg(Color::Cyan)),
                    Span::raw("Import existing nsec"),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[ESC] ", Style::default().fg(Color::Red)),
                    Span::raw("Exit"),
                ]),
            ];

            let paragraph = Paragraph::new(content)
                .style(Style::default())
                .alignment(Alignment::Center);
            f.render_widget(paragraph, chunks[1]);
        }
        OnboardingMode::GenerateNew => {
            let content = vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    "Your new keys have been generated!",
                    Style::default().fg(Color::Green),
                )]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("NPUB: ", Style::default().fg(Color::Yellow)),
                    Span::raw("..."), // We'd need to generate this
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("NSEC: ", Style::default().fg(Color::Red)),
                    Span::raw("..."), // We'd need to generate this
                ]),
                Line::from(""),
                Line::from(vec![Span::styled(
                    "⚠️  SAVE YOUR NSEC IN A SECURE LOCATION!",
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK),
                )]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[ENTER] ", Style::default().fg(Color::Green)),
                    Span::raw("Continue"),
                    Span::raw("  "),
                    Span::styled("[ESC] ", Style::default().fg(Color::Red)),
                    Span::raw("Back"),
                ]),
            ];

            let paragraph = Paragraph::new(content)
                .style(Style::default())
                .alignment(Alignment::Center);
            f.render_widget(paragraph, chunks[1]);
        }
        OnboardingMode::EnterDisplayName => {
            let content = vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    "Enter your display name:",
                    Style::default().fg(Color::Yellow),
                )]),
                Line::from(""),
            ];

            let paragraph = Paragraph::new(content)
                .style(Style::default())
                .alignment(Alignment::Center);
            f.render_widget(paragraph, chunks[1]);

            let input_box = Paragraph::new(input)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Color::DarkGray)),
                )
                .style(Style::default().fg(Color::White));

            let input_area = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(0)])
                .split(chunks[2]);

            // Center the input box
            let centered_input = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(25),
                    Constraint::Percentage(50),
                    Constraint::Percentage(25),
                ])
                .split(input_area[0]);

            f.render_widget(input_box, centered_input[1]);

            let help = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("[ENTER] ", Style::default().fg(Color::Green)),
                    Span::raw("Continue"),
                    Span::raw("  "),
                    Span::styled("[ESC] ", Style::default().fg(Color::Red)),
                    Span::raw("Back"),
                ]),
            ];

            let help_text = Paragraph::new(help).alignment(Alignment::Center);
            f.render_widget(help_text, input_area[1]);
        }
        OnboardingMode::ImportExisting => {
            let content = vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    "Enter your nsec:",
                    Style::default().fg(Color::Yellow),
                )]),
                Line::from(""),
            ];

            let paragraph = Paragraph::new(content)
                .style(Style::default())
                .alignment(Alignment::Center);
            f.render_widget(paragraph, chunks[1]);

            let input_box = Paragraph::new(input)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Color::DarkGray)),
                )
                .style(Style::default().fg(Color::White));

            let input_area = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(0)])
                .split(chunks[2]);

            f.render_widget(input_box, input_area[0]);

            let help = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("[ENTER] ", Style::default().fg(Color::Green)),
                    Span::raw("Import"),
                    Span::raw("  "),
                    Span::styled("[ESC] ", Style::default().fg(Color::Red)),
                    Span::raw("Cancel"),
                ]),
            ];

            let help_text = Paragraph::new(help).alignment(Alignment::Center);
            f.render_widget(help_text, input_area[1]);
        }
        OnboardingMode::CreatePassword => {
            let prompt_lines = vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    "Create a password to encrypt your keys:",
                    Style::default().fg(Color::Yellow),
                )]),
                Line::from(""),
                Line::from(vec![Span::styled(
                    "This password will be required each time you start NRC",
                    Style::default().fg(Color::DarkGray),
                )]),
                Line::from(""),
            ];

            let help_text = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("[ENTER] ", Style::default().fg(Color::Green)),
                    Span::raw("Continue"),
                    Span::raw("  "),
                    Span::styled("[ESC] ", Style::default().fg(Color::Red)),
                    Span::raw("Back"),
                ]),
            ];

            draw_password_input(f, chunks[1], prompt_lines, input, help_text);
        }
        OnboardingMode::EnterPassword => {
            let mut prompt_lines = vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    "Welcome back!",
                    Style::default().fg(Color::Green),
                )]),
                Line::from(""),
                Line::from(vec![Span::styled(
                    "Enter your password to decrypt your keys:",
                    Style::default().fg(Color::Yellow),
                )]),
            ];

            // Show error if present
            if let Some(err) = error {
                prompt_lines.push(Line::from(""));
                prompt_lines.push(Line::from(vec![Span::styled(
                    err,
                    Style::default().fg(Color::Red),
                )]));
            }

            prompt_lines.push(Line::from(""));

            let help_text = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("[ENTER] ", Style::default().fg(Color::Green)),
                    Span::raw("Unlock"),
                ]),
            ];

            draw_password_input(f, chunks[1], prompt_lines, input, help_text);
        }
    }
}

fn draw_initializing(f: &mut Frame, area: Rect) {
    let block = Block::default()
        .title("╔═══ NRC - INITIALIZING ═══╗")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let loading = vec![
        Line::from(""),
        Line::from(""),
        Line::from(vec![Span::styled(
            "INITIALIZING...",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK),
        )]),
        Line::from(""),
        Line::from("Connecting to relays..."),
        Line::from("Publishing key package..."),
    ];

    let paragraph = Paragraph::new(loading)
        .style(Style::default())
        .alignment(Alignment::Center);
    f.render_widget(paragraph, inner);
}

fn draw_ready_view_with_state(
    f: &mut Frame,
    area: Rect,
    ui_state: &UIState,
    npub: &str,
    groups: &[openmls::group::GroupId],
) {
    // Show help overlay if active
    if ui_state.show_help {
        draw_help_overlay(f, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);

    let groups_block = Block::default()
        .title("═══ CHATS ═══")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::DarkGray));

    let selected_index = ui_state.selected_group_index;
    let items: Vec<ListItem> = if groups.is_empty() {
        vec![ListItem::new("No chats yet").style(Style::default().fg(Color::DarkGray))]
    } else {
        groups
            .iter()
            .enumerate()
            .map(|(i, group)| {
                // Get the display name for this chat from groups metadata
                let display_name = ui_state.groups.get(group)
                    .map(|g| g.name.clone())
                    .unwrap_or_else(|| format!("Chat {}", i + 1));
                ListItem::new(display_name).style(if selected_index == Some(i) {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                })
            })
            .collect()
    };

    let list = List::new(items).block(groups_block).style(Style::default());
    f.render_widget(list, chunks[0]);

    // Handle right side layout based on whether there's an error
    if let Some(ref error) = ui_state.last_error {
        // Calculate how many lines the error needs (with wrapping)
        let available_width = chunks[1].width.saturating_sub(4) as usize; // Subtract borders and padding
        let estimated_lines = if available_width > 0 {
            (error.len() / available_width) + 1
        } else {
            1
        };
        let error_height = ((estimated_lines + 2) as u16).min(chunks[1].height.saturating_sub(4)); // +2 for borders, leave room for input

        // Split right side with dynamic error height
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(error_height),
                Constraint::Length(3),
            ])
            .split(chunks[1]);

        // Check if a chat is selected for the top area
        if let Some(selected_index) = ui_state.selected_group_index {
            if selected_index < groups.len() {
                // Show messages for the selected chat
                let selected_group = &groups[selected_index];
                draw_messages_with_state(f, right_chunks[0], ui_state, npub, selected_group);
            } else {
                // Selected index out of bounds, show appropriate content
                if should_show_help_message_with_state(ui_state, groups) {
                    draw_info_panel_with_state(f, right_chunks[0], ui_state, npub);
                } else {
                    draw_empty_chat_area(f, right_chunks[0]);
                }
            }
        } else {
            // No chat selected, show appropriate content
            if should_show_help_message_with_state(ui_state, groups) {
                draw_info_panel_with_state(f, right_chunks[0], ui_state, npub);
            } else {
                draw_empty_chat_area(f, right_chunks[0]);
            }
        }

        // Draw error above input
        let error_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Red))
            .title("═ ERROR ═");

        let error_text = Paragraph::new(error.as_str())
            .block(error_block)
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: true });

        f.render_widget(error_text, right_chunks[1]);

        // Draw input box
        draw_input_with_state(f, right_chunks[2], ui_state);
    } else {
        // No error - use standard layout
        // Split right side for content and input (3 lines for input box)
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(chunks[1]);

        // Check if a chat is selected
        if let Some(selected_index) = ui_state.selected_group_index {
            if selected_index < groups.len() {
                // Show messages for the selected chat
                let selected_group = &groups[selected_index];
                draw_messages_with_state(f, right_chunks[0], ui_state, npub, selected_group);
            } else {
                // Selected index out of bounds, show appropriate content
                if should_show_help_message_with_state(ui_state, groups) {
                    draw_info_panel_with_state(f, right_chunks[0], ui_state, npub);
                } else {
                    draw_empty_chat_area(f, right_chunks[0]);
                }
            }
        } else {
            // No chat selected, show appropriate content
            if should_show_help_message_with_state(ui_state, groups) {
                draw_info_panel_with_state(f, right_chunks[0], ui_state, npub);
            } else {
                draw_empty_chat_area(f, right_chunks[0]);
            }
        }

        // Draw input box with optional flash message
        draw_input_with_flash_with_state(f, right_chunks[1], ui_state);
    }
}

fn draw_messages_with_state(
    f: &mut Frame,
    area: Rect,
    ui_state: &UIState,
    npub: &str,
    active_group: &openmls::group::GroupId,
) {
    let block = Block::default()
        .title("═══ CHAT ═══")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::DarkGray));

    let messages = &ui_state.messages;
    let group_messages = messages.get(active_group);

    let content: Vec<Line> = if group_messages.is_none() || group_messages.unwrap().is_empty() {
        vec![Line::from(Span::styled(
            "No messages yet. Start the conversation!",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        let msgs = group_messages.unwrap();
        msgs.iter()
            .map(|msg| {
                // Get the display name for the sender
                // TODO: We need to get display names from somewhere
                let sender_name = msg.sender.to_bech32()
                    .map(|npub| {
                        if npub.len() > 20 {
                            format!("{}...{}", &npub[..10], &npub[npub.len() - 3..])
                        } else {
                            npub
                        }
                    })
                    .unwrap_or_else(|_| "Unknown".to_string());
                
                // Use different colors for different users
                // Current user gets green, others get cyan
                let our_npub = npub;
                let sender_npub = msg.sender.to_bech32().unwrap_or_default();
                let color = if sender_npub == *our_npub {
                    Color::Green
                } else {
                    Color::Cyan
                };
                Line::from(vec![
                    Span::styled(format!("{sender_name}: "), Style::default().fg(color)),
                    Span::raw(&msg.content),
                ])
            })
            .collect()
    };

    // TODO: Handle scroll offset
    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

fn draw_input_with_state(f: &mut Frame, area: Rect, ui_state: &UIState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray))
        .title("═ INPUT ═");

    let input = &ui_state.input;
    // Create text with a cursor using spans
    let text = vec![Line::from(vec![
        Span::raw(input.as_str()),
        Span::styled(
            "▌",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ])];

    let input_widget = Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(Color::White));

    f.render_widget(input_widget, area);
}

fn draw_input_with_flash_with_state(f: &mut Frame, area: Rect, ui_state: &UIState) {
    if let Some(ref msg) = ui_state.flash_message {
        // Split area for flash message and input
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(3)])
            .split(area);

        // Draw flash message
        let flash = Paragraph::new(msg.as_str())
            .style(Style::default().fg(Color::Green))
            .alignment(Alignment::Center);
        f.render_widget(flash, chunks[0]);

        // Draw input
        draw_input_with_state(f, chunks[1], ui_state);
    } else {
        // No flash message, just draw input
        draw_input_with_state(f, area, ui_state);
    }
}

fn draw_info_panel_with_state(f: &mut Frame, area: Rect, _ui_state: &UIState, npub: &str) {
    let info_block = Block::default()
        .title("═══ INFO ═══")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::DarkGray));

    let info = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("NPUB: ", Style::default().fg(Color::Yellow)),
            Span::raw(npub),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "COMMANDS:",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("/join <npub> (/j) ", Style::default().fg(Color::Green)),
            Span::raw("Start chat"),
        ]),
        Line::from(vec![
            Span::styled("/npub (/n) ", Style::default().fg(Color::Cyan)),
            Span::raw("Copy to clipboard"),
        ]),
        Line::from(vec![
            Span::styled("/help (/h) ", Style::default().fg(Color::Cyan)),
            Span::raw("Show help"),
        ]),
        Line::from(vec![
            Span::styled("/quit (/q) ", Style::default().fg(Color::Red)),
            Span::raw("Exit"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "NAVIGATION:",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("↑↓ or Ctrl+j/k ", Style::default().fg(Color::Cyan)),
            Span::raw("Navigate chats"),
        ]),
    ];

    let paragraph = Paragraph::new(info)
        .block(info_block)
        .style(Style::default());
    f.render_widget(paragraph, area);
}

fn should_show_help_message_with_state(_ui_state: &UIState, groups: &[openmls::group::GroupId]) -> bool {
    // Show help if no groups exist
    groups.is_empty()
}

fn draw_empty_chat_area(f: &mut Frame, area: Rect) {
    let block = Block::default()
        .title("═══ CHAT ═══")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::DarkGray));

    let content = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            "Select a chat to start messaging",
            Style::default().fg(Color::DarkGray),
        )]),
    ];

    let paragraph = Paragraph::new(content)
        .block(block)
        .style(Style::default())
        .alignment(Alignment::Center);
    f.render_widget(paragraph, area);
}

fn draw_help_overlay(f: &mut Frame, area: Rect) {
    // Create a centered popup
    let popup_area = centered_rect(60, 60, area);

    // Clear the area behind the popup
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title("═══ HELP ═══")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Yellow));

    let help_text = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            "NRC - Nostr Relay Chat",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "COMMANDS:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("/join <npub> ", Style::default().fg(Color::Green)),
            Span::raw("Start a chat with someone"),
        ]),
        Line::from(vec![
            Span::styled("/npub ", Style::default().fg(Color::Green)),
            Span::raw("Copy your npub to clipboard"),
        ]),
        Line::from(vec![
            Span::styled("/help ", Style::default().fg(Color::Green)),
            Span::raw("Show this help screen"),
        ]),
        Line::from(vec![
            Span::styled("/quit ", Style::default().fg(Color::Green)),
            Span::raw("Exit the application"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "NAVIGATION:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("↑↓ ", Style::default().fg(Color::Green)),
            Span::raw("Navigate between chats"),
        ]),
        Line::from(vec![
            Span::styled("Ctrl+j/k ", Style::default().fg(Color::Green)),
            Span::raw("Alternative navigation"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press any key to close this help",
                Style::default().fg(Color::DarkGray)),
        ]),
    ];

    let paragraph = Paragraph::new(help_text)
        .block(block)
        .style(Style::default())
        .alignment(Alignment::Left);

    f.render_widget(paragraph, popup_area);
}

/// Helper function to create a centered rect
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}