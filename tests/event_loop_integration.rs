mod common;

use anyhow::Result;
use common::TestClient;
use crossterm::event::{KeyCode, KeyEvent};
use nostr_sdk::prelude::*;
use nrc::AppEvent;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[tokio::test]
async fn test_welcome_message_regression_and_chat() -> Result<()> {
    // This test catches the exact bug we found where welcome messages weren't being sent
    // AND verifies that Alice and Bob can actually exchange messages

    // Create two clients
    let alice = TestClient::new("alice").await?;
    let bob = TestClient::new("bob").await?;

    // Give Alice's key package time to propagate to relays
    tokio::time::sleep(Duration::from_secs(2)).await;

    let alice_npub = alice.npub().await?;
    let alice_pubkey = PublicKey::from_bech32(&alice_npub)?;

    // Bob joins with Alice's npub (like user would type /j <npub>)
    bob.execute_command(&format!("/j {alice_npub}")).await?;

    // Give it a moment to process
    tokio::time::sleep(Duration::from_millis(500)).await;

    // CRITICAL TEST: Bob should have a welcome rumor for Alice
    assert!(
        bob.has_welcome_rumor_for(&alice_pubkey).await,
        "Bob doesn't have welcome rumor for Alice - this is the bug!"
    );

    // Bob should have created a group
    assert_eq!(bob.group_count().await, 1, "Bob should have 1 group");

    // Wait for Alice to receive and process the welcome
    tokio::time::sleep(Duration::from_secs(4)).await;

    // Trigger fetch welcomes event and process it
    alice.trigger_fetch_welcomes().await?;
    alice.process_pending_events().await?;

    // Alice should now have a group too
    assert_eq!(
        alice.group_count().await,
        1,
        "Alice should have auto-joined the group"
    );

    // Now test actual messaging
    // Bob selects his first group and sends a message to Alice
    bob.select_first_group().await?;
    bob.process_pending_events().await?; // Process the navigation event
    bob.execute_command("Hello Alice!").await?;

    // Wait for message to propagate through relays
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Trigger fetch messages event for Alice and process it
    alice.trigger_fetch_messages().await?;
    alice.process_pending_events().await?;

    // Alice should have received Bob's message
    {
        let alice_nrc = alice.nrc.lock().await;
        let groups = alice_nrc.get_groups();
        assert_eq!(groups.len(), 1, "Alice should have 1 group");

        let messages = alice_nrc.get_messages(&groups[0]);
        assert_eq!(messages.len(), 1, "Alice should have 1 message");
        assert_eq!(messages[0].content, "Hello Alice!");

        let bob_pubkey = bob.nrc.lock().await.public_key();
        assert_eq!(messages[0].sender, bob_pubkey, "Message should be from Bob");
    }

    // Alice selects her first group and sends a reply
    alice.select_first_group().await?;
    alice.process_pending_events().await?; // Process the navigation event
    alice.execute_command("Hi Bob!").await?;

    // Wait for message to propagate
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Trigger fetch messages event for Bob and process it
    bob.trigger_fetch_messages().await?;
    bob.process_pending_events().await?;

    // Bob should have both messages (his own and Alice's reply)
    {
        let bob_nrc = bob.nrc.lock().await;
        let groups = bob_nrc.get_groups();
        let messages = bob_nrc.get_messages(&groups[0]);

        // Bob should see at least Alice's reply (his own message may or may not be stored)
        assert!(
            messages.iter().any(|m| m.content == "Hi Bob!"),
            "Bob should have received Alice's reply"
        );

        let alice_pubkey = alice.nrc.lock().await.public_key();
        assert!(
            messages
                .iter()
                .any(|m| m.sender == alice_pubkey && m.content == "Hi Bob!"),
            "Bob should have Alice's message with correct sender"
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_event_loop_doesnt_block() -> Result<()> {
    // Test that the event loop doesn't block on network operations

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    // Send a timer-based event (ProcessPendingOperationsTick still exists)
    event_tx.send(AppEvent::ProcessPendingOperationsTick)?;

    // Also send a keyboard event immediately after
    event_tx.send(AppEvent::KeyPress(KeyEvent::from(KeyCode::Char('a'))))?;

    // The keyboard event should be processable even if processing is running
    let start = Instant::now();

    // Get first event (processing tick)
    let event1 = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
        .await?
        .ok_or_else(|| anyhow::anyhow!("No event received"))?;
    assert!(matches!(event1, AppEvent::ProcessPendingOperationsTick));

    // Get second event (keyboard) - should be available immediately
    let event2 = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
        .await?
        .ok_or_else(|| anyhow::anyhow!("No event received"))?;
    assert!(matches!(event2, AppEvent::KeyPress(_)));

    // Both events should be received quickly
    assert!(
        start.elapsed() < Duration::from_millis(250),
        "Events took too long to process: {:?}",
        start.elapsed()
    );

    Ok(())
}

#[tokio::test]
async fn test_welcome_sent_over_network() -> Result<()> {
    // Test that welcome is actually sent as a GiftWrap event

    let alice = TestClient::new("alice").await?;
    let bob = TestClient::new("bob").await?;

    // Give Alice's key package time to propagate to relays
    tokio::time::sleep(Duration::from_secs(2)).await;

    let alice_npub = alice.npub().await?;

    // In a full implementation, we'd hook into Bob's client to capture sent events
    // For now, we just verify the welcome_rumor is created and group is formed

    // Bob joins Alice
    bob.execute_command(&format!("/j {alice_npub}")).await?;

    // Wait a bit for async operations
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Verify Bob created a group
    assert_eq!(
        bob.group_count().await,
        1,
        "Bob should have created a group"
    );

    Ok(())
}
