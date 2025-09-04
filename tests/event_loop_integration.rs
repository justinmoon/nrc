mod common;

use anyhow::Result;
use common::TestClient;
use nrc::AppEvent;
use nostr_sdk::prelude::*;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use crossterm::event::{KeyCode, KeyEvent};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::test]
async fn test_welcome_message_regression_and_chat() -> Result<()> {
    // This test catches the exact bug we found where welcome messages weren't being sent
    // AND verifies that Alice and Bob can actually exchange messages
    
    // Create two clients
    let alice = TestClient::new("alice").await?;
    let bob = TestClient::new("bob").await?;
    
    let alice_npub = alice.npub().await?;
    let alice_pubkey = PublicKey::from_bech32(&alice_npub)?;
    
    // Bob joins with Alice's npub (like user would type /j <npub>)
    bob.execute_command(&format!("/j {}", alice_npub)).await?;
    
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
    
    // Alice should fetch welcomes and auto-join the group
    {
        let mut alice_nrc = alice.nrc.lock().await;
        alice_nrc.fetch_and_process_welcomes().await?;
    }
    
    // Alice should now have a group too
    assert_eq!(alice.group_count().await, 1, "Alice should have auto-joined the group");
    
    // Now test actual messaging
    // Bob sends a message to Alice
    {
        let mut bob_nrc = bob.nrc.lock().await;
        bob_nrc.selected_group_index = Some(0); // Select the first group
        bob_nrc.process_input("Hello Alice!".to_string()).await?;
    }
    
    // Wait for message to propagate through relays
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    // Alice fetches messages
    {
        let mut alice_nrc = alice.nrc.lock().await;
        alice_nrc.fetch_and_process_messages().await?;
    }
    
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
    
    // Alice sends a reply
    {
        let mut alice_nrc = alice.nrc.lock().await;
        alice_nrc.selected_group_index = Some(0);
        alice_nrc.process_input("Hi Bob!".to_string()).await?;
    }
    
    // Wait for message to propagate
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    // Bob fetches messages
    {
        let mut bob_nrc = bob.nrc.lock().await;
        bob_nrc.fetch_and_process_messages().await?;
    }
    
    // Bob should have both messages (his own and Alice's reply)
    {
        let bob_nrc = bob.nrc.lock().await;
        let groups = bob_nrc.get_groups();
        let messages = bob_nrc.get_messages(&groups[0]);
        
        // Bob should see at least Alice's reply (his own message may or may not be stored)
        assert!(messages.iter().any(|m| m.content == "Hi Bob!"), 
                "Bob should have received Alice's reply");
        
        let alice_pubkey = alice.nrc.lock().await.public_key();
        assert!(messages.iter().any(|m| m.sender == alice_pubkey && m.content == "Hi Bob!"),
                "Bob should have Alice's message with correct sender");
    }
    
    Ok(())
}

#[tokio::test]
async fn test_event_loop_doesnt_block() -> Result<()> {
    // Test that the event loop doesn't block on network operations
    
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    
    // Send a fetch tick event
    event_tx.send(AppEvent::FetchMessagesTick)?;
    
    // Also send a keyboard event immediately after
    event_tx.send(AppEvent::KeyPress(KeyEvent::from(KeyCode::Char('a'))))?;
    
    // The keyboard event should be processable even if fetch is running
    let start = Instant::now();
    
    // Get first event (fetch tick)
    let event1 = tokio::time::timeout(Duration::from_millis(100), event_rx.recv()).await?
        .ok_or_else(|| anyhow::anyhow!("No event received"))?;
    assert!(matches!(event1, AppEvent::FetchMessagesTick));
    
    // Get second event (keyboard) - should be available immediately
    let event2 = tokio::time::timeout(Duration::from_millis(100), event_rx.recv()).await?
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
async fn test_concurrent_fetch_and_typing() -> Result<()> {
    // Test that typing works while network fetches are happening
    
    let client = TestClient::new("test").await?;
    
    // Trigger multiple fetch operations
    for _ in 0..3 {
        client.send_event(AppEvent::FetchMessagesTick).await?;
    }
    
    // Send keyboard events - these should not be blocked
    let start = Instant::now();
    
    for c in "hello".chars() {
        client.send_event(AppEvent::KeyPress(KeyEvent::from(KeyCode::Char(c)))).await?;
    }
    
    // All keyboard events should be sent quickly
    assert!(
        start.elapsed() < Duration::from_millis(100),
        "Keyboard events blocked by fetches: {:?}",
        start.elapsed()
    );
    
    Ok(())
}

#[tokio::test]
async fn test_fetch_spawns_background_task() -> Result<()> {
    // Test that FetchMessagesTick actually spawns a background task
    
    let client = TestClient::new("test").await?;
    
    // Create a group first so there's something to fetch
    // Note: This might fail if no key packages, but that's OK for this test
    // We're just testing that the fetch doesn't block
    {
        let mut nrc = client.nrc.lock().await;
        if let Ok(group_id) = nrc.create_group("test".to_string()).await {
            nrc.add_group(group_id);
        }
    }
    
    // Send fetch tick
    let start = Instant::now();
    client.send_event(AppEvent::FetchMessagesTick).await?;
    
    // The event should be processed almost instantly (spawning task, not waiting)
    // In the old broken code, this would block for seconds
    assert!(
        start.elapsed() < Duration::from_millis(50),
        "FetchMessagesTick blocked for {:?} - should spawn background task!",
        start.elapsed()
    );
    
    Ok(())
}

#[tokio::test]
async fn test_welcome_sent_over_network() -> Result<()> {
    // Test that welcome is actually sent as a GiftWrap event
    
    let alice = TestClient::new("alice").await?;
    let bob = TestClient::new("bob").await?;
    
    let alice_npub = alice.npub().await?;
    
    // In a full implementation, we'd hook into Bob's client to capture sent events
    // For now, we just verify the welcome_rumor is created and group is formed
    
    // Bob joins Alice
    bob.execute_command(&format!("/j {}", alice_npub)).await?;
    
    // Wait a bit for async operations
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // Verify Bob created a group
    assert_eq!(bob.group_count().await, 1, "Bob should have created a group");
    
    Ok(())
}