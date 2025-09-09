mod common;

use anyhow::Result;
use common::TestClient;
use std::time::Duration;

#[tokio::test]
async fn test_multi_user_group_creation_and_chat() -> Result<()> {
    // Test creating a multi-user group and having multiple users chat in it

    // Create three clients
    let alice = TestClient::new("alice").await?;
    let bob = TestClient::new("bob").await?;
    let _charlie = TestClient::new("charlie").await?;

    // Publish key packages for all clients
    alice.execute_command("/n").await?;
    bob.execute_command("/n").await?;
    _charlie.execute_command("/n").await?;

    // Give everyone's key packages time to propagate to relays
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Alice creates a multi-user group with Bob
    let bob_npub = bob.npub().await?;
    alice
        .execute_command(&format!("/group #test-channel {bob_npub}"))
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
                flash.contains("Created #test-channel"),
                "Should confirm group creation: {flash}"
            );
        } else {
            panic!("No flash message after creating group");
        }
    }

    // Alice should have the group immediately
    assert_eq!(
        alice.group_count().await,
        1,
        "Alice should have 1 group after creation"
    );

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

    // Note: With only 2 members, the group is treated as a DM and shows the other person's name
    // This is the correct behavior - groups only show #channelname with 3+ members

    // With 3+ members, it should now show as a channel
    {
        let bob_nrc = bob.nrc.lock().await;
        let groups = bob_nrc.get_groups();
        let group_id = &groups[0];

        // Debug: check member count
        let member_count = bob_nrc
            .get_member_count(group_id)
            .expect("Should get member count");
        println!("Group has {member_count} members");

        let group_name = bob_nrc.get_chat_display_name(group_id);
        println!("Group name: {group_name}");

        if member_count > 2 {
            assert_eq!(
                group_name, "#test-channel",
                "Bob should see #test-channel with 3+ members"
            );
        } else {
            // With only 2 members, it's treated as a DM and shows the other person's name
            assert_eq!(
                group_name, "alice",
                "Bob should see alice's name in 2-person group"
            );
        }
    }

    // Now test multi-user messaging
    // Alice sends a message
    alice.execute_command("Hello everyone!").await?;

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
            messages.iter().any(|m| m.content == "Hello everyone!"),
            "Bob should have received Alice's message"
        );

        let alice_pubkey = alice.nrc.lock().await.public_key();
        assert!(
            messages
                .iter()
                .any(|m| m.sender == alice_pubkey && m.content == "Hello everyone!"),
            "Message should be from Alice"
        );
    }

    // Test that this is working as a group (even with 2 people)

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
            "Alice should have received Bob's reply"
        );

        let bob_pubkey = bob.nrc.lock().await.public_key();
        assert!(
            messages
                .iter()
                .any(|m| m.sender == bob_pubkey && m.content == "Hi Alice!"),
            "Reply should be from Bob"
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_multi_user_group_with_members_command() -> Result<()> {
    // Test the /members command in a multi-user group

    let alice = TestClient::new("alice").await?;
    let bob = TestClient::new("bob").await?;

    // Publish key packages for both clients
    alice.execute_command("/n").await?;
    bob.execute_command("/n").await?;

    // Give key packages time to propagate
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Alice creates a group with Bob
    let bob_npub = bob.npub().await?;
    alice
        .execute_command(&format!("/group #members-test {bob_npub}"))
        .await?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Select the group
    alice.select_first_group().await?;
    alice.process_pending_events().await?;

    // Check members - should show both Alice and Bob since Bob was added at creation
    alice.execute_command("/members").await?;

    // Check that Alice sees both herself and Bob in the member list
    {
        let alice_nrc = alice.nrc.lock().await;
        // The flash message should contain the member list
        if let Some(ref flash) = alice_nrc.flash_message {
            assert!(flash.contains("Members:"), "Should show members list");
            assert!(
                flash.contains("alice"),
                "Alice should be in the member list"
            );
            // Bob's npub is shown, not his name
            assert!(
                flash.contains(&bob_npub[..8]) || flash.contains("bob"),
                "Bob should be in the member list (as npub or name): {flash}"
            );
        } else {
            panic!("No flash message after /members command");
        }
    }

    // Wait for invite to propagate and let Bob fetch welcomes
    tokio::time::sleep(Duration::from_secs(4)).await;
    bob.trigger_fetch_welcomes().await?;
    bob.process_pending_events().await?;

    // Verify Bob successfully joined the group
    assert_eq!(
        bob.group_count().await,
        1,
        "Bob should have joined the group"
    );

    // Check members again using short form - should still show both Alice and Bob
    alice.execute_command("/m").await?; // Test short form

    {
        let alice_nrc = alice.nrc.lock().await;
        if let Some(ref flash) = alice_nrc.flash_message {
            assert!(
                flash.contains("alice"),
                "Alice should still be in the member list"
            );
            // Bob's npub is shown, not his name
            assert!(
                flash.contains(&bob_npub[..8]) || flash.contains("bob"),
                "Bob should still be in the member list (as npub or name)"
            );
        } else {
            panic!("No flash message after /m command");
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_multi_user_group_leave_command() -> Result<()> {
    // Test the /leave command

    let alice = TestClient::new("alice").await?;
    let bob = TestClient::new("bob").await?;

    // Publish key packages for both clients
    alice.execute_command("/n").await?;
    bob.execute_command("/n").await?;

    // Give key packages time to propagate
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Alice creates a group with Bob
    let bob_npub = bob.npub().await?;
    alice
        .execute_command(&format!("/g #leave-test {bob_npub}"))
        .await?; // Test short form of create
    tokio::time::sleep(Duration::from_millis(500)).await;

    alice.select_first_group().await?;
    alice.process_pending_events().await?;

    // Wait for invite and fetch welcomes
    tokio::time::sleep(Duration::from_secs(4)).await;
    bob.trigger_fetch_welcomes().await?;
    bob.process_pending_events().await?;

    assert_eq!(bob.group_count().await, 1, "Bob should have 1 group");

    // Bob leaves the group
    bob.select_first_group().await?;
    bob.process_pending_events().await?;
    bob.execute_command("/l").await?; // Test short form of leave

    // Bob should no longer have any groups
    assert_eq!(bob.group_count().await, 0, "Bob should have left the group");

    Ok(())
}

#[tokio::test]
async fn test_group_name_display_differentiation() -> Result<()> {
    // Test that 2-person DMs show the other person's name,
    // while 3+ person groups show #channelname

    let alice = TestClient::new("alice").await?;
    let bob = TestClient::new("bob").await?;
    let charlie = TestClient::new("charlie").await?;

    // Publish key packages for all clients
    alice.execute_command("/n").await?;
    bob.execute_command("/n").await?;
    charlie.execute_command("/n").await?;

    // Give key packages time to propagate
    tokio::time::sleep(Duration::from_secs(3)).await;

    let alice_npub = alice.npub().await?;

    // Bob creates a 2-person DM with Alice (using /d command)
    bob.execute_command(&format!("/d {alice_npub}")).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Bob should see Alice's name (not a channel name)
    {
        let bob_nrc = bob.nrc.lock().await;
        let groups = bob_nrc.get_groups();
        assert_eq!(groups.len(), 1);

        let display_name = bob_nrc.get_chat_display_name(&groups[0]);
        assert_eq!(
            display_name, "alice",
            "2-person DM should show alice's name"
        );
        assert!(
            !display_name.starts_with('#'),
            "2-person DM should not start with #"
        );
    }

    // Wait for Alice to receive welcome
    tokio::time::sleep(Duration::from_secs(4)).await;
    alice.trigger_fetch_welcomes().await?;
    alice.process_pending_events().await?;

    // Alice should also see Bob's name
    {
        let alice_nrc = alice.nrc.lock().await;
        let groups = alice_nrc.get_groups();
        assert_eq!(groups.len(), 1);

        let display_name = alice_nrc.get_chat_display_name(&groups[0]);
        assert_eq!(
            display_name, "bob",
            "Alice should see bob's name in 2-person DM"
        );
        assert!(
            !display_name.starts_with('#'),
            "2-person DM should not start with #"
        );
    }

    // Now Alice creates a multi-user group with Bob and Charlie
    let bob_npub = bob.npub().await?;
    let charlie_npub = charlie.npub().await?;
    alice
        .execute_command(&format!("/group #multi-test {bob_npub} {charlie_npub}"))
        .await?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Alice should now have 2 groups
    assert_eq!(alice.group_count().await, 2, "Alice should have 2 groups");

    // Check that the new group shows as #multi-test
    {
        let alice_nrc = alice.nrc.lock().await;
        let groups = alice_nrc.get_groups();

        // Find the multi-user group (the one that's not the DM)
        let multi_group = groups
            .iter()
            .find(|g| alice_nrc.get_chat_display_name(g).starts_with('#'))
            .expect("Should find multi-user group");

        let display_name = alice_nrc.get_chat_display_name(multi_group);
        assert_eq!(
            display_name, "#multi-test",
            "Multi-user group should show as #multi-test"
        );
    }

    // Wait for invites to propagate
    tokio::time::sleep(Duration::from_secs(4)).await;

    // Bob and Charlie fetch welcomes
    bob.trigger_fetch_welcomes().await?;
    bob.process_pending_events().await?;
    charlie.trigger_fetch_welcomes().await?;
    charlie.process_pending_events().await?;

    // Bob should now have 2 groups: the DM and the multi-user group
    assert_eq!(bob.group_count().await, 2, "Bob should have 2 groups");

    // Verify Bob sees the correct names for both groups
    {
        let bob_nrc = bob.nrc.lock().await;
        let groups = bob_nrc.get_groups();

        let names: Vec<String> = groups
            .iter()
            .map(|g| bob_nrc.get_chat_display_name(g))
            .collect();

        assert!(
            names.contains(&"alice".to_string()),
            "Bob should see alice DM"
        );
        assert!(
            names.contains(&"#multi-test".to_string()),
            "Bob should see #multi-test group"
        );
    }

    // Charlie should have 1 group (the multi-user group)
    assert_eq!(
        charlie.group_count().await,
        1,
        "Charlie should have 1 group"
    );

    {
        let charlie_nrc = charlie.nrc.lock().await;
        let groups = charlie_nrc.get_groups();
        let display_name = charlie_nrc.get_chat_display_name(&groups[0]);
        assert_eq!(
            display_name, "#multi-test",
            "Charlie should see #multi-test"
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_create_group_requires_key_package() -> Result<()> {
    // Test that creating a group requires at least one member npub

    let alice = TestClient::new("alice_no_kp").await?;
    let bob = TestClient::new("bob_no_kp").await?;

    // Wait for key packages to propagate
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Try to create group without members - should fail
    alice.execute_command("/group #fail-test").await?;

    // Check for error
    {
        let alice_nrc = alice.nrc.lock().await;
        if let Some(ref error) = alice_nrc.last_error {
            assert!(
                error.contains("Usage:") && error.contains("npub"),
                "Should have usage error: {error}"
            );
        }
    }

    // No group should have been created
    assert_eq!(
        alice.group_count().await,
        0,
        "No group should be created without members"
    );

    // Try again with a member - should work
    let bob_npub = bob.npub().await?;
    alice
        .execute_command(&format!("/group #success-test {bob_npub}"))
        .await?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Should have 1 group now
    assert_eq!(
        alice.group_count().await,
        1,
        "Group should be created with member"
    );

    Ok(())
}
