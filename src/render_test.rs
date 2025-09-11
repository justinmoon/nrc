#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use ratatui::{
        backend::TestBackend,
        layout::{Constraint, Direction, Layout},
        style::{Color, Style},
        text::Line,
        widgets::{Block, Borders, Paragraph},
        Terminal,
    };
    use std::time::{Duration, Instant};

    fn render_flash_message_test(width: u16, height: u16, message: &str) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();

        let flash = Some((message.to_string(), Instant::now() + Duration::from_secs(5)));

        terminal
            .draw(|f| {
                let size = f.area();

                // Calculate dynamic height for flash message
                let available_width = size.width.saturating_sub(2) as usize;
                let mut line_count = 0;
                let words: Vec<&str> = message.split_whitespace().collect();
                let mut current_line = String::new();

                for word in &words {
                    if current_line.is_empty() {
                        if word.len() <= available_width {
                            current_line = word.to_string();
                        } else {
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
                                line_count += word.len().div_ceil(available_width);
                                current_line.clear();
                            }
                        }
                    }
                }
                if !current_line.is_empty() {
                    line_count += 1;
                }

                let flash_height = line_count.min(10) as u16;

                // Simulate the chat layout with dynamic height
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(0),               // Messages area
                        Constraint::Length(flash_height), // Dynamic flash message area
                        Constraint::Length(3),            // Input area
                    ])
                    .split(size);

                // Render flash message if active
                if let Some((msg, _)) = &flash {
                    // Fixed implementation with better word wrapping
                    let available_width = chunks[1].width.saturating_sub(2) as usize;
                    let mut wrapped_lines = Vec::new();

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
                        wrapped_lines.push(
                            Line::from(current_line).style(Style::default().fg(Color::Green)),
                        );
                    }

                    let flash_widget = Paragraph::new(wrapped_lines);
                    f.render_widget(flash_widget, chunks[1]);
                }

                // Render input area
                let input_widget =
                    Paragraph::new("").block(Block::default().borders(Borders::ALL).title("INPUT"));
                f.render_widget(input_widget, chunks[2]);
            })
            .unwrap();

        // Convert buffer to string representation
        let buffer = terminal.backend().buffer();
        let mut output = String::new();
        for y in 0..height {
            for x in 0..width {
                let cell = &buffer[(x, y)];
                output.push_str(cell.symbol());
            }
            if y < height - 1 {
                output.push('\n');
            }
        }
        output
    }

    #[test]
    fn test_flash_message_wrapping_fits() {
        let output = render_flash_message_test(80, 10, "Short message that fits");
        assert_snapshot!(output);
    }

    #[test]
    fn test_flash_message_wrapping_npub() {
        let output = render_flash_message_test(
            80,
            10,
            "Copied npub to clipboard: npub1rfs5zsr4v2qjizw8a8x2gvxgeamshcuam5y9v9m2ysg47mwue98q4v5je3"
        );
        assert_snapshot!(output);
    }

    #[test]
    fn test_flash_message_wrapping_narrow() {
        let output = render_flash_message_test(
            40,
            10,
            "Copied npub to clipboard: npub1rfs5zsr4v2qjizw8a8x2gvxgeamshcuam5y9v9m2ysg47mwue98q4v5je3"
        );
        assert_snapshot!(output);
    }

    #[test]
    fn test_flash_message_dynamic_height() {
        // Test with a message that needs more height
        let output = render_flash_message_test(
            30,
            15,
            "This is a very long flash message that should wrap across multiple lines to test dynamic height allocation"
        );
        assert_snapshot!(output);
    }
}
