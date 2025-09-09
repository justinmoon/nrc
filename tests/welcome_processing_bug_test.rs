use anyhow::Result;
use std::time::Duration;

mod common;
use common::TestClient;

/// Test to replicate the DM welcome processing bug
/// This test should fail until we fix the welcome processing issue
#[tokio::test]
async fn test_dm_welcome_processing_bug() -> Result<()> {
    let _ = env_logger::try_init(); // Use try_init to avoid double init

    // Create two clients like in manual testing
    let client_a = TestClient::new("alice").await?;
    let client_b = TestClient::new("bob").await?;

    // Client A publishes key package (like running /n)
    client_a.execute_command("/n").await?;

    // Wait for key package to propagate
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Get client A's npub
    let alice_npub = client_a.npub().await?;

    // Client B creates DM to Alice (like running /dm <alice_npub>)
    let dm_command = format!("/dm {alice_npub}");
    client_b.execute_command(&dm_command).await?;

    // Check that Client B has created the group
    let bob_groups = client_b.group_count().await;
    assert!(bob_groups > 0, "Client B should have created a group");

    // Wait for welcome message to propagate
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Explicitly trigger welcome fetch for Alice
    client_a.trigger_fetch_welcomes().await?;

    // Process the events that were generated (including RawWelcomesReceived)
    client_a.process_pending_events().await?;

    // THE BUG: Client A should have received the welcome and joined the group
    // but currently this fails
    let alice_groups = client_a.group_count().await;

    println!("Alice groups: {alice_groups}, Bob groups: {bob_groups}");

    assert!(
        alice_groups > 0,
        "BUG: Alice should have received the welcome and joined the group, but has {alice_groups} groups"
    );

    Ok(())
}

/// Test individual welcome processing components to isolate the issue
#[tokio::test]
async fn test_welcome_processing_components() -> Result<()> {
    let _ = env_logger::try_init(); // Use try_init to avoid double init

    // This test will help us figure out where exactly the issue is:
    // 1. Is the welcome message being sent?
    // 2. Is the notification handler receiving GiftWrap events?
    // 3. Is the main loop processing RawWelcomesReceived?
    // 4. Is process_welcome_event being called?

    let client_a = TestClient::new("receiver").await?;
    let client_b = TestClient::new("sender").await?;

    // Set up like manual test
    client_a.execute_command("/n").await?;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let receiver_npub = client_a.npub().await?;
    let dm_command = format!("/dm {receiver_npub}");

    // Send DM invitation
    client_b.execute_command(&dm_command).await?;

    // Give some time for processing
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Explicitly trigger welcome fetch for receiver
    client_a.trigger_fetch_welcomes().await?;

    // Process the events that were generated (including RawWelcomesReceived)
    client_a.process_pending_events().await?;

    // Check results
    let sender_groups = client_b.group_count().await;
    let receiver_groups = client_a.group_count().await;

    println!("Sender has {sender_groups} groups, Receiver has {receiver_groups} groups");

    // For debugging: the test will help us see where the issue is
    if receiver_groups == 0 {
        println!("ISSUE CONFIRMED: Receiver did not process welcome message");
    }

    // For now, let's not assert - just observe the behavior
    Ok(())
}
