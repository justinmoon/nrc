use anyhow::Result;
use nostr_sdk::prelude::*;
use nrc::Nrc;
use std::time::Duration;

#[tokio::test]
async fn test_giftwrap_welcome_delivery() -> Result<()> {
    // This test catches the GiftWrap welcome delivery bug:
    // When Bob runs `/j <alice_npub>`, Alice should receive the welcome message

    let alice_dir = std::env::temp_dir().join("test_alice_gw");
    let bob_dir = std::env::temp_dir().join("test_bob_gw");

    let _ = std::fs::remove_dir_all(&alice_dir);
    let _ = std::fs::remove_dir_all(&bob_dir);
    std::fs::create_dir_all(&alice_dir)?;
    std::fs::create_dir_all(&bob_dir)?;

    // Create Alice and Bob
    let mut alice = Nrc::new(&alice_dir, false).await?;
    alice
        .initialize_with_display_name("alice".to_string())
        .await?;

    let mut bob = Nrc::new(&bob_dir, false).await?;
    bob.initialize_with_display_name("bob".to_string()).await?;

    // Get Alice's npub
    let alice_npub = alice.public_key().to_bech32()?;

    // Wait for propagation
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Bob runs /j <alice_npub>
    bob.process_input(format!("/j {}", alice_npub)).await?;

    // Wait for GiftWrap to propagate
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Alice fetches welcomes (this should work with fixed filter)
    alice.fetch_and_process_welcomes().await?;

    // Alice should have joined the group
    let alice_groups = alice.get_groups();
    assert_eq!(
        alice_groups.len(),
        1,
        "Alice should have received welcome and joined group"
    );

    // Clean up
    let _ = std::fs::remove_dir_all(&alice_dir);
    let _ = std::fs::remove_dir_all(&bob_dir);

    Ok(())
}
