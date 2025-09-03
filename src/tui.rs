use nrc::{AppState, Nrc, OnboardingMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use nostr_sdk::prelude::ToBech32;

pub fn draw(f: &mut Frame, nrc: &Nrc) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0)])
        .split(f.area());
    
    match &nrc.state {
        AppState::Onboarding { input, mode } => {
            draw_onboarding(f, chunks[0], input, mode);
        }
        AppState::Initializing => {
            draw_initializing(f, chunks[0]);
        }
        AppState::Ready { groups, .. } => {
            draw_ready_view(f, chunks[0], nrc, groups);
        }
    }
}

fn draw_onboarding(f: &mut Frame, area: Rect, input: &str, mode: &OnboardingMode) {
    let block = Block::default()
        .title("╔═══ NRC - ONBOARDING ═══╗")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Cyan));
    
    let inner = block.inner(area);
    f.render_widget(block, area);
    
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(10),
            Constraint::Min(0),
        ])
        .split(inner);
    
    let title = Paragraph::new("NOSTR RELAY CHAT")
        .style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center);
    f.render_widget(title, chunks[0]);
    
    match mode {
        OnboardingMode::Choose => {
            let menu = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("[1] ", Style::default().fg(Color::Yellow)),
                    Span::styled("Generate New Key", Style::default().fg(Color::White)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[2] ", Style::default().fg(Color::Yellow)),
                    Span::styled("Import Existing nsec", Style::default().fg(Color::White)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[ESC] ", Style::default().fg(Color::Red)),
                    Span::styled("Exit", Style::default().fg(Color::White)),
                ]),
            ];
            
            let paragraph = Paragraph::new(menu)
                .style(Style::default())
                .alignment(Alignment::Center);
            f.render_widget(paragraph, chunks[1]);
        }
        OnboardingMode::GenerateNew => {
            // This mode is no longer used - we generate immediately
            // But keeping it here in case the state somehow ends up here
            let content = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("Generating...", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                ]),
            ];
            
            let paragraph = Paragraph::new(content)
                .style(Style::default())
                .alignment(Alignment::Center);
            f.render_widget(paragraph, chunks[1]);
        }
        OnboardingMode::ImportExisting => {
            let content = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("Enter your nsec:", Style::default().fg(Color::Yellow)),
                ]),
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
                        .border_style(Style::default().fg(Color::Cyan))
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
    }
}

fn draw_initializing(f: &mut Frame, area: Rect) {
    let block = Block::default()
        .title("╔═══ NRC - INITIALIZING ═══╗")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Yellow));
    
    let inner = block.inner(area);
    f.render_widget(block, area);
    
    let loading = vec![
        Line::from(""),
        Line::from(""),
        Line::from(vec![
            Span::styled("INITIALIZING...", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK)),
        ]),
        Line::from(""),
        Line::from("Connecting to relays..."),
        Line::from("Publishing key package..."),
    ];
    
    let paragraph = Paragraph::new(loading)
        .style(Style::default())
        .alignment(Alignment::Center);
    f.render_widget(paragraph, inner);
}

fn draw_ready_view(f: &mut Frame, area: Rect, nrc: &Nrc, groups: &[openmls::group::GroupId]) {
    // Show help overlay if active
    if nrc.show_help {
        draw_help_overlay(f, area);
        return;
    }
    
    // First split for error display if needed
    let (error_area, main_area) = if nrc.last_error.is_some() {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(area);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, area)
    };
    
    // Draw error if present
    if let (Some(ref error), Some(error_rect)) = (&nrc.last_error, error_area) {
        let error_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Red));
        
        let error_text = Paragraph::new(error.as_str())
            .block(error_block)
            .style(Style::default().fg(Color::Red))
            .alignment(Alignment::Center);
        
        f.render_widget(error_text, error_rect);
    }
    
    let area = main_area;
    
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);
    
    let groups_block = Block::default()
        .title("═══ CHATS ═══")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Cyan));
    
    let items: Vec<ListItem> = if groups.is_empty() {
        vec![ListItem::new("No chats yet").style(Style::default().fg(Color::DarkGray))]
    } else {
        groups
            .iter()
            .enumerate()
            .map(|(i, _group)| {
                let content = format!("Chat #{}", i + 1);
                ListItem::new(content)
                    .style(if nrc.selected_group_index == Some(i) {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    })
            })
            .collect()
    };
    
    let list = List::new(items)
        .block(groups_block)
        .style(Style::default());
    
    f.render_widget(list, chunks[0]);
    
    // Split right side for content and input (4 lines: 1 for flash, 3 for input box)
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(4)])
        .split(chunks[1]);
    
    // Check if a chat is selected
    if let Some(selected_index) = nrc.selected_group_index {
        if selected_index < groups.len() {
            // Show messages for the selected chat
            let selected_group = &groups[selected_index];
            draw_messages(f, right_chunks[0], nrc, selected_group);
        } else {
            // Selected index out of bounds, show info
            draw_info_panel(f, right_chunks[0], nrc);
        }
    } else {
        // No chat selected, show info panel
        draw_info_panel(f, right_chunks[0], nrc);
    }
    
    // Draw input box at bottom right with flash message
    draw_input_with_flash(f, right_chunks[1], nrc);
}


fn draw_messages(f: &mut Frame, area: Rect, nrc: &Nrc, active_group: &openmls::group::GroupId) {
    let block = Block::default()
        .title("═══ MESSAGES ═══")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Green));
    
    let messages = nrc.get_messages(active_group);
    
    let content: Vec<Line> = if messages.is_empty() {
        vec![Line::from(Span::styled(
            "No messages yet. Start the conversation!",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        messages
            .iter()
            .flat_map(|msg| {
                vec![
                    Line::from(vec![
                        Span::styled(
                            format!("[{}] ", msg.sender.to_bech32().unwrap_or_else(|_| "unknown".to_string())),
                            Style::default().fg(Color::Cyan),
                        ),
                        Span::styled(
                            format!("{}", msg.timestamp.as_u64()),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]),
                    Line::from(Span::raw(&msg.content)),
                    Line::from(""),
                ]
            })
            .collect()
    };
    
    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: true })
        .scroll((nrc.scroll_offset, 0));
    
    f.render_widget(paragraph, area);
}

fn draw_input(f: &mut Frame, area: Rect, nrc: &Nrc) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow))
        .title("═ INPUT ═");
    
    // Create text with a cursor using spans
    let text = vec![
        Line::from(vec![
            Span::raw(&nrc.input),
            Span::styled("▌", Style::default().fg(Color::White).add_modifier(Modifier::SLOW_BLINK)),
        ])
    ];
    
    let input = Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(Color::White));
    
    f.render_widget(input, area);
}

fn draw_info_panel(f: &mut Frame, area: Rect, nrc: &Nrc) {
    let info_block = Block::default()
        .title("═══ INFO ═══")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Green));
    
    let info = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("NPUB: ", Style::default().fg(Color::Yellow)),
            Span::raw(nrc.public_key().to_bech32().unwrap_or_else(|_| "error".to_string())),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("COMMANDS:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
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
            Span::styled("↑↓ or Ctrl+j/k ", Style::default().fg(Color::DarkGray)),
            Span::raw("Navigate chats"),
        ]),
        Line::from(vec![
            Span::styled("/quit (/q) ", Style::default().fg(Color::Red)),
            Span::raw("Exit NRC"),
        ]),
    ];
    
    let paragraph = Paragraph::new(info)
        .block(info_block)
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

fn draw_help_overlay(f: &mut Frame, area: Rect) {
    // Create centered overlay
    let block = Block::default()
        .title("╔═══ HELP ═══╗")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Yellow));
    
    let help_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("NRC - NOSTR RELAY CHAT", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("COMMANDS:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("/join <npub> ", Style::default().fg(Color::Green)),
            Span::styled("(/j)", Style::default().fg(Color::DarkGray)),
            Span::raw(" - Start a new chat with someone"),
        ]),
        Line::from(vec![
            Span::styled("/npub ", Style::default().fg(Color::Cyan)),
            Span::styled("(/n)", Style::default().fg(Color::DarkGray)),
            Span::raw(" - Copy your npub to clipboard"),
        ]),
        Line::from(vec![
            Span::styled("/help ", Style::default().fg(Color::Cyan)),
            Span::styled("(/h)", Style::default().fg(Color::DarkGray)),
            Span::raw(" - Show this help screen"),
        ]),
        Line::from(vec![
            Span::styled("/quit ", Style::default().fg(Color::Red)),
            Span::styled("(/q)", Style::default().fg(Color::DarkGray)),
            Span::raw(" - Exit NRC"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("NAVIGATION:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("↑/↓ ", Style::default().fg(Color::Green)),
            Span::raw("- Navigate through chats (when input is empty)"),
        ]),
        Line::from(vec![
            Span::styled("Ctrl+j/Ctrl+k ", Style::default().fg(Color::Green)),
            Span::raw("- Navigate through chats (always works)"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("IN CHAT:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("Type your message and press "),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(" to send"),
        ]),
        Line::from(vec![
            Span::styled("/exit ", Style::default().fg(Color::Yellow)),
            Span::raw("- Leave the current chat view"),
        ]),
        Line::from(""),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press any key to dismiss this help...", Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
        ]),
    ];
    
    let paragraph = Paragraph::new(help_text)
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    
    // Create a centered popup
    let popup_area = centered_rect(80, 80, area);
    f.render_widget(Clear, popup_area);
    f.render_widget(paragraph, popup_area);
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

fn draw_input_with_flash(f: &mut Frame, area: Rect, nrc: &Nrc) {
    // Split area for flash message and input
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(3)])
        .split(area);
    
    // Draw flash message or error
    if let Some(ref flash) = nrc.flash_message {
        let flash_text = Paragraph::new(flash.as_str())
            .style(Style::default().fg(Color::Green))
            .alignment(Alignment::Center);
        f.render_widget(flash_text, chunks[0]);
    } else if let Some(ref error) = nrc.last_error {
        let error_text = Paragraph::new(error.as_str())
            .style(Style::default().fg(Color::Red))
            .alignment(Alignment::Center);
        f.render_widget(error_text, chunks[0]);
    }
    
    // Draw input box
    draw_input(f, chunks[1], nrc);
}