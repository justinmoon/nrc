mod common;

use anyhow::Result;
use common::TestClient;
use std::time::Duration;

#[tokio::test]
async fn test_giftwrap_welcome_delivery() -> Result<()> {
    // This test catches the GiftWrap welcome delivery bug:
    // When Bob runs `/j <alice_npub>`, Alice should receive the welcome message

    // Create Alice and Bob using TestClient
    let alice = TestClient::new("alice").await?;
    let bob = TestClient::new("bob").await?;

    // Get Alice's npub
    let alice_npub = alice.npub().await?;

    // Wait for propagation
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Bob runs /j <alice_npub>
    bob.execute_command(&format!("/j {alice_npub}")).await?;

    // Wait for GiftWrap to propagate
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Alice fetches welcomes using event-driven approach
    alice.trigger_fetch_welcomes().await?;
    alice.process_pending_events().await?;

    // Alice should have joined the group
    let alice_groups = {
        let alice_nrc = alice.nrc.lock().await;
        alice_nrc.get_groups()
    };
    assert_eq!(
        alice_groups.len(),
        1,
        "Alice should have received welcome and joined group"
    );

    // TestClient handles cleanup automatically

    Ok(())
}
