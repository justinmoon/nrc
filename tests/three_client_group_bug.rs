mod common;

use anyhow::Result;
use common::TestClient;
use nostr_sdk::ToBech32;
use std::time::Duration;

#[tokio::test]
async fn test_three_client_group_message_visibility() -> Result<()> {
    // Test scenario:
    // 1. A creates group with B
    // 2. A and B exchange messages
    // 3. A invites C
    // 4. C sends a message
    // 5. Verify A can still see and send messages

    // Create three clients
    let alice = TestClient::new("alice").await?;
    let bob = TestClient::new("bob").await?;
    let charlie = TestClient::new("charlie").await?;

    // Give everyone's key packages time to propagate to relays
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Alice creates a group with Bob
    let bob_npub = bob.npub().await?;
    alice
        .execute_command(&format!("/group #test {bob_npub}"))
        .await?;

    // Give it a moment to process
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check for any error message
    {
        let alice_nrc = alice.nrc.lock().await;
        if let Some(ref error) = alice_nrc.last_error {
            panic!("Error creating group: {error}");
        }
        // Should have a flash message about creating the group
        if let Some(ref flash) = alice_nrc.flash_message {
            assert!(
                flash.contains("Created #test"),
                "Should confirm group creation: {flash}"
            );
        } else {
            panic!("No flash message after creating group");
        }
    }

    // Alice should have the group
    assert_eq!(alice.group_count().await, 1, "Alice should have 1 group");

    // Wait for invite to propagate
    tokio::time::sleep(Duration::from_secs(4)).await;

    // Bob fetches welcomes and processes them
    bob.trigger_fetch_welcomes().await?;
    bob.process_pending_events().await?;

    // Bob should now have the group
    assert_eq!(
        bob.group_count().await,
        1,
        "Bob should have joined the group"
    );

    // Alice sends a message
    alice.select_first_group().await?;
    alice.process_pending_events().await?;
    alice.execute_command("Hello Bob!").await?;

    // Wait for message to propagate
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Bob fetches messages
    bob.trigger_fetch_messages().await?;
    bob.process_pending_events().await?;

    // Bob should have received Alice's message
    {
        let bob_nrc = bob.nrc.lock().await;
        let groups = bob_nrc.get_groups();
        let messages = bob_nrc.get_messages(&groups[0]);

        assert!(
            messages.iter().any(|m| m.content == "Hello Bob!"),
            "Bob should have received Alice's message"
        );
    }

    // Bob sends a reply
    bob.select_first_group().await?;
    bob.process_pending_events().await?;
    bob.execute_command("Hi Alice!").await?;

    // Wait for message to propagate
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Alice fetches messages
    alice.trigger_fetch_messages().await?;
    alice.process_pending_events().await?;

    // Alice should see Bob's message
    {
        let alice_nrc = alice.nrc.lock().await;
        let groups = alice_nrc.get_groups();
        let messages = alice_nrc.get_messages(&groups[0]);

        assert!(
            messages.iter().any(|m| m.content == "Hi Alice!"),
            "Alice should have received Bob's reply before inviting Charlie"
        );
    }

    println!("✓ Alice and Bob can exchange messages before Charlie joins");

    // Now Alice invites Charlie
    let charlie_npub = charlie.npub().await?;
    alice
        .execute_command(&format!("/invite {charlie_npub}"))
        .await?;

    // Wait for invite to propagate
    tokio::time::sleep(Duration::from_secs(4)).await;

    // Charlie fetches welcomes and processes them
    charlie.trigger_fetch_welcomes().await?;
    charlie.process_pending_events().await?;

    // Charlie should now have the group
    assert_eq!(
        charlie.group_count().await,
        1,
        "Charlie should have joined the group"
    );

    // Check who Charlie thinks is in the group
    {
        let charlie_nrc = charlie.nrc.lock().await;
        let groups = charlie_nrc.get_groups();
        let member_count = charlie_nrc
            .get_member_count(&groups[0])
            .expect("Should get member count");
        println!("Charlie sees {member_count} members in the group");

        // Note: We can't easily get the full member list from the test,
        // but the member count tells us if Charlie knows about all members
    }

    println!("✓ Charlie successfully joined the group");

    // Charlie sends a message
    charlie.select_first_group().await?;
    charlie.process_pending_events().await?;
    charlie.execute_command("Hello everyone!").await?;

    // Verify Charlie's message was actually sent
    {
        let charlie_nrc = charlie.nrc.lock().await;
        let groups = charlie_nrc.get_groups();
        let messages = charlie_nrc.get_messages(&groups[0]);

        println!("Charlie's messages after sending:");
        for msg in &messages {
            println!(
                "  - {}: {}",
                msg.sender
                    .to_bech32()
                    .unwrap_or_else(|_| "unknown".to_string()),
                msg.content
            );
        }

        assert!(
            messages.iter().any(|m| m.content == "Hello everyone!"),
            "Charlie should see his own message"
        );
    }

    // Wait for message to propagate
    tokio::time::sleep(Duration::from_secs(3)).await;

    // All clients fetch messages
    alice.trigger_fetch_messages().await?;
    alice.process_pending_events().await?;
    bob.trigger_fetch_messages().await?;
    bob.process_pending_events().await?;

    // Check Bob's view of the group members after fetching
    {
        let bob_nrc = bob.nrc.lock().await;
        let groups = bob_nrc.get_groups();
        let member_count = bob_nrc
            .get_member_count(&groups[0])
            .expect("Should get member count");
        println!("Bob sees {member_count} members in the group after Charlie joins");
    }

    // Check that Bob can see Charlie's message
    {
        let bob_nrc = bob.nrc.lock().await;
        let groups = bob_nrc.get_groups();
        let messages = bob_nrc.get_messages(&groups[0]);

        println!("Bob's messages after Charlie sends:");
        for msg in &messages {
            println!(
                "  - {}: {}",
                msg.sender
                    .to_bech32()
                    .unwrap_or_else(|_| "unknown".to_string()),
                msg.content
            );
        }

        assert!(
            messages.iter().any(|m| m.content == "Hello everyone!"),
            "Bob should see Charlie's message"
        );
    }

    println!("✓ Bob can see Charlie's message");

    // CRITICAL CHECK: Alice should still be able to see Charlie's message
    {
        let alice_nrc = alice.nrc.lock().await;
        let groups = alice_nrc.get_groups();
        let messages = alice_nrc.get_messages(&groups[0]);

        // Print all messages for debugging
        println!("Alice's messages after Charlie joins:");
        for msg in &messages {
            println!(
                "  - {}: {}",
                msg.sender
                    .to_bech32()
                    .unwrap_or_else(|_| "unknown".to_string()),
                msg.content
            );
        }

        assert!(
            messages.iter().any(|m| m.content == "Hello everyone!"),
            "Alice should see Charlie's message (BUG: Alice loses message visibility after C joins)"
        );
    }

    println!("✓ Alice can see Charlie's message");

    // Alice sends another message to verify she can still send
    alice.execute_command("Welcome Charlie!").await?;

    // Wait for message to propagate
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Bob and Charlie fetch messages
    bob.trigger_fetch_messages().await?;
    bob.process_pending_events().await?;
    charlie.trigger_fetch_messages().await?;
    charlie.process_pending_events().await?;

    // Bob should see Alice's new message
    {
        let bob_nrc = bob.nrc.lock().await;
        let groups = bob_nrc.get_groups();
        let messages = bob_nrc.get_messages(&groups[0]);

        assert!(
            messages.iter().any(|m| m.content == "Welcome Charlie!"),
            "Bob should see Alice's welcome message"
        );
    }

    // Charlie should see Alice's new message
    {
        let charlie_nrc = charlie.nrc.lock().await;
        let groups = charlie_nrc.get_groups();
        let messages = charlie_nrc.get_messages(&groups[0]);

        assert!(
            messages.iter().any(|m| m.content == "Welcome Charlie!"),
            "Charlie should see Alice's welcome message"
        );
    }

    println!("✓ Alice can still send messages after Charlie joins");

    // Final check: Alice should see her own message
    alice.trigger_fetch_messages().await?;
    alice.process_pending_events().await?;

    {
        let alice_nrc = alice.nrc.lock().await;
        let groups = alice_nrc.get_groups();
        let messages = alice_nrc.get_messages(&groups[0]);

        // Print final message state for debugging
        println!("Alice's final messages:");
        for msg in &messages {
            println!(
                "  - {}: {}",
                msg.sender
                    .to_bech32()
                    .unwrap_or_else(|_| "unknown".to_string()),
                msg.content
            );
        }

        assert!(
            messages.iter().any(|m| m.content == "Welcome Charlie!"),
            "Alice should see her own message after Charlie joins"
        );
    }

    println!("✓ Test passed: All clients can send and receive messages correctly");

    Ok(())
}
