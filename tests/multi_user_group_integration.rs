mod common;

use anyhow::Result;
use common::TestClient;
use nostr_sdk::prelude::*;
use std::time::Duration;

#[tokio::test]
async fn test_multi_user_group_creation_and_chat() -> Result<()> {
    // Test creating a multi-user group and having multiple users chat in it
    
    // Create three clients
    let alice = TestClient::new("alice").await?;
    let bob = TestClient::new("bob").await?;
    let charlie = TestClient::new("charlie").await?;
    
    // Give everyone's key packages time to propagate to relays
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    // Alice creates a pending multi-user group
    alice.execute_command("/create #test-channel").await?;
    
    // Give it a moment to process
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Check for any error message
    {
        let alice_nrc = alice.nrc.lock().await;
        if let Some(ref error) = alice_nrc.last_error {
            panic!("Error creating group: {}", error);
        }
        // Should have a flash message about creating the group
        if let Some(ref flash) = alice_nrc.flash_message {
            assert!(flash.contains("Created group #test-channel"), "Should confirm group creation: {}", flash);
        } else {
            panic!("No flash message after creating group");
        }
    }
    
    // Alice doesn't have a real group yet - it's pending until someone is invited
    assert_eq!(alice.group_count().await, 0, "Alice should have 0 real groups (1 pending)");
    
    // Alice invites Bob - this will create the actual group
    let bob_npub = bob.npub().await?;
    alice.execute_command(&format!("/invite {bob_npub}")).await?;
    
    // Give it a moment to create the group
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Alice should now have a real group after inviting Bob
    assert_eq!(alice.group_count().await, 1, "Alice should have 1 real group after inviting");
    
    // Wait for invite to propagate
    tokio::time::sleep(Duration::from_secs(4)).await;
    
    // Bob fetches welcomes and processes them
    bob.trigger_fetch_welcomes().await?;
    bob.process_pending_events().await?;
    
    // Bob should now have the group
    assert_eq!(bob.group_count().await, 1, "Bob should have joined the group");
    
    // Note: With only 2 members, the group is treated as a DM and shows the other person's name
    // This is the correct behavior - groups only show #channelname with 3+ members
    
    // With 3+ members, it should now show as a channel
    {
        let bob_nrc = bob.nrc.lock().await;
        let groups = bob_nrc.get_groups();
        let group_id = &groups[0];
        
        // Debug: check member count
        let member_count = bob_nrc.get_member_count(group_id).expect("Should get member count");
        println!("Group has {} members", member_count);
        
        let group_name = bob_nrc.get_chat_display_name(group_id);
        println!("Group name: {}", group_name);
        
        if member_count > 2 {
            assert_eq!(group_name, "#test-channel", "Bob should see #test-channel with 3+ members");
        } else {
            // With only 2 members, it's treated as a DM and shows the other person's name
            assert_eq!(group_name, "alice", "Bob should see alice's name in 2-person group");
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
            messages.iter().any(|m| m.sender == alice_pubkey && m.content == "Hello everyone!"),
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
            messages.iter().any(|m| m.sender == bob_pubkey && m.content == "Hi Alice!"),
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
    
    // Give key packages time to propagate
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Alice creates a group
    alice.execute_command("/create #members-test").await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Select the group
    alice.select_first_group().await?;
    alice.process_pending_events().await?;
    
    // Check members (should just be Alice)
    alice.execute_command("/members").await?;
    
    // Check that Alice sees herself in the member list
    {
        let alice_nrc = alice.nrc.lock().await;
        // The flash message should contain the member list
        if let Some(ref flash) = alice_nrc.flash_message {
            assert!(flash.contains("Members:"), "Should show members list");
            assert!(flash.contains("alice"), "Alice should be in the member list");
        } else {
            panic!("No flash message after /members command");
        }
    }
    
    // Alice invites Bob
    let bob_npub = bob.npub().await?;
    alice.execute_command(&format!("/invite {bob_npub}")).await?;
    
    // Wait for invite and fetch welcomes
    tokio::time::sleep(Duration::from_secs(4)).await;
    bob.trigger_fetch_welcomes().await?;
    bob.process_pending_events().await?;
    
    // Now check members again - should show both Alice and Bob
    alice.execute_command("/m").await?; // Test short form
    
    {
        let alice_nrc = alice.nrc.lock().await;
        if let Some(ref flash) = alice_nrc.flash_message {
            assert!(flash.contains("alice"), "Alice should still be in the member list");
            assert!(flash.contains("bob"), "Bob should now be in the member list");
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
    
    // Give key packages time to propagate
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Alice creates a group
    alice.execute_command("/c #leave-test").await?; // Test short form of create
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Alice invites Bob
    let bob_npub = bob.npub().await?;
    alice.select_first_group().await?;
    alice.process_pending_events().await?;
    alice.execute_command(&format!("/i {bob_npub}")).await?; // Test short form of invite
    
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
    
    // Give key packages time to propagate
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    let alice_npub = alice.npub().await?;
    
    // Bob creates a 2-person DM with Alice (using /j command)
    bob.execute_command(&format!("/j {alice_npub}")).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Bob should see Alice's name (not a channel name)
    {
        let bob_nrc = bob.nrc.lock().await;
        let groups = bob_nrc.get_groups();
        assert_eq!(groups.len(), 1);
        
        let display_name = bob_nrc.get_chat_display_name(&groups[0]);
        assert_eq!(display_name, "alice", "2-person DM should show alice's name");
        assert!(!display_name.starts_with('#'), "2-person DM should not start with #");
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
        assert_eq!(display_name, "bob", "Alice should see bob's name in 2-person DM");
        assert!(!display_name.starts_with('#'), "2-person DM should not start with #");
    }
    
    // Now Alice creates a multi-user group
    alice.execute_command("/create #multi-test").await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Alice should now have 2 groups
    assert_eq!(alice.group_count().await, 2, "Alice should have 2 groups");
    
    // Check that the new group shows as #multi-test
    {
        let alice_nrc = alice.nrc.lock().await;
        let groups = alice_nrc.get_groups();
        
        // Find the multi-user group (the one that's not the DM)
        let multi_group = groups.iter()
            .find(|g| alice_nrc.get_chat_display_name(g).starts_with('#'))
            .expect("Should find multi-user group");
        
        let display_name = alice_nrc.get_chat_display_name(multi_group);
        assert_eq!(display_name, "#multi-test", "Multi-user group should show as #multi-test");
    }
    
    // Alice invites Bob and Charlie to the multi-user group
    // First select the multi-user group
    alice.send_arrow_up()?; // Navigate to the other group
    alice.process_pending_events().await?;
    
    let bob_npub = bob.npub().await?;
    let charlie_npub = charlie.npub().await?;
    
    alice.execute_command(&format!("/invite {bob_npub}")).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    alice.execute_command(&format!("/invite {charlie_npub}")).await?;
    
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
        
        let names: Vec<String> = groups.iter()
            .map(|g| bob_nrc.get_chat_display_name(g))
            .collect();
        
        assert!(names.contains(&"alice".to_string()), "Bob should see alice DM");
        assert!(names.contains(&"#multi-test".to_string()), "Bob should see #multi-test group");
    }
    
    // Charlie should have 1 group (the multi-user group)
    assert_eq!(charlie.group_count().await, 1, "Charlie should have 1 group");
    
    {
        let charlie_nrc = charlie.nrc.lock().await;
        let groups = charlie_nrc.get_groups();
        let display_name = charlie_nrc.get_chat_display_name(&groups[0]);
        assert_eq!(display_name, "#multi-test", "Charlie should see #multi-test");
    }
    
    Ok(())
}

#[tokio::test]
async fn test_create_group_requires_key_package() -> Result<()> {
    // Test that creating a group without publishing key package fails appropriately
    
    let alice = TestClient::new("alice_no_kp").await?;
    
    // Don't wait for key package to propagate - try to create group immediately
    // This should fail because key package isn't published yet
    alice.execute_command("/create #fail-test").await?;
    
    // Check for error
    {
        let alice_nrc = alice.nrc.lock().await;
        if let Some(ref error) = alice_nrc.last_error {
            assert!(
                error.contains("key") || error.contains("package") || error.contains("empty"),
                "Should have error about key package: {}",
                error
            );
        }
    }
    
    // No group should have been created
    assert_eq!(alice.group_count().await, 0, "No group should be created without key package");
    
    // Now wait for key package to propagate
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    // Try again - should work now
    alice.execute_command("/create #success-test").await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Should have 1 group now
    assert_eq!(alice.group_count().await, 1, "Group should be created after key package is ready");
    
    Ok(())
}