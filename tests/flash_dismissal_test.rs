#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use nostr_sdk::prelude::*;
    use nrc::ui_state::Page;
    use nrc::{App, AppEvent};
    use nrc_mls::NostrMls;
    use nrc_mls_sqlite_storage::NostrMlsSqliteStorage;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    async fn setup_test_app() -> App {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let keys = Keys::generate();
        let storage = NostrMlsSqliteStorage::new(db_path.to_str().unwrap()).unwrap();
        let nostr_mls = NostrMls::new(storage);
        #[allow(clippy::arc_with_non_send_sync)]
        let storage_arc = Arc::new(nostr_mls);
        let client = Client::default();
        let key_storage = nrc::key_storage::KeyStorage::new(temp_dir.path());

        let initial_page = Page::Chat {
            groups: vec![],
            selected_group_index: 0,
            group_id: openmls::group::GroupId::from_slice(&[1, 2, 3, 4]),
            group_info: Box::new(nrc_mls_storage::groups::types::Group {
                mls_group_id: openmls::group::GroupId::from_slice(&[1, 2, 3, 4]),
                nostr_group_id: [0u8; 32],
                name: "Test Group".to_string(),
                description: "Test group".to_string(),
                admin_pubkeys: std::collections::BTreeSet::new(),
                last_message_id: None,
                last_message_at: None,
                epoch: 0,
                state: nrc_mls_storage::groups::types::GroupState::Active,
                image_url: None,
                image_key: None,
                image_nonce: None,
            }),
            messages: vec![],
            members: vec![],
            input: String::new(),
            scroll_offset: 0,
            typing_members: vec![],
        };

        App::new(storage_arc, client, keys, key_storage, initial_page)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_flash_dismissal_on_first_keystroke() {
        let mut app = setup_test_app().await;

        // Set flash message
        app.flash = Some((
            "Test flash".to_string(),
            Instant::now() + Duration::from_secs(10),
        ));
        assert!(app.flash.is_some());

        // Any keystroke dismisses flash AND gets processed normally
        app.handle_event(AppEvent::KeyPress(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::empty(),
        )))
        .await
        .unwrap();
        assert!(app.flash.is_none(), "Flash should be dismissed");
        if let Page::Chat { input, .. } = &app.current_page {
            assert_eq!(
                input, "a",
                "Character should be added (flash dismissal is just a side-effect)"
            );
        }

        // Next keystroke works normally
        app.handle_event(AppEvent::KeyPress(KeyEvent::new(
            KeyCode::Char('b'),
            KeyModifiers::empty(),
        )))
        .await
        .unwrap();
        if let Page::Chat { input, .. } = &app.current_page {
            assert_eq!(input, "ab", "Second keystroke should work normally");
        }
    }

    #[tokio::test]
    async fn test_flash_dismissal_with_existing_input() {
        let mut app = setup_test_app().await;

        // Type some text first
        app.handle_event(AppEvent::KeyPress(KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::empty(),
        )))
        .await
        .unwrap();
        app.handle_event(AppEvent::KeyPress(KeyEvent::new(
            KeyCode::Char('i'),
            KeyModifiers::empty(),
        )))
        .await
        .unwrap();

        if let Page::Chat { input, .. } = &app.current_page {
            assert_eq!(input, "hi");
        }

        // Set flash message
        app.flash = Some((
            "Command executed".to_string(),
            Instant::now() + Duration::from_secs(5),
        ));

        // Next keystroke dismisses flash AND gets processed
        app.handle_event(AppEvent::KeyPress(KeyEvent::new(
            KeyCode::Char(' '),
            KeyModifiers::empty(),
        )))
        .await
        .unwrap();

        assert!(app.flash.is_none());
        if let Page::Chat { input, .. } = &app.current_page {
            assert_eq!(input, "hi ", "Space should be added after dismissing flash");
        }

        // Now typing continues normally
        app.handle_event(AppEvent::KeyPress(KeyEvent::new(
            KeyCode::Char('!'),
            KeyModifiers::empty(),
        )))
        .await
        .unwrap();
        if let Page::Chat { input, .. } = &app.current_page {
            assert_eq!(input, "hi !");
        }
    }
}
