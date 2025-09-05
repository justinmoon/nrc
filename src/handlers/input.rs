// Input event handling logic functions
// These will be called from Nrc's internal event handler

use crossterm::event::KeyCode;

pub fn build_input_from_key(key_code: KeyCode, current_input: &str) -> String {
    match key_code {
        KeyCode::Char(c) => format!("{current_input}{c}"),
        KeyCode::Backspace => {
            if current_input.is_empty() {
                String::new()
            } else {
                current_input[..current_input.len() - 1].to_string()
            }
        }
        _ => current_input.to_string(),
    }
}
