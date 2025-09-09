// Simple test to understand MLS key package flow
use anyhow::Result;
use nostr_sdk::prelude::*;
use nrc_mls::prelude::*;
use nrc_mls_sqlite_storage::NostrMlsSqliteStorage;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    println!("=== Testing MLS Key Package Flow ===\n");

    // Setup Alice
    let alice_keys = Keys::generate();
    println!("Alice's npub: {}", alice_keys.public_key().to_bech32()?);
    let alice_storage = NostrMlsSqliteStorage::new("alice_test.db")?;
    let alice_mls = NostrMls::new(alice_storage);

    // Setup Bob
    let bob_keys = Keys::generate();
    println!("Bob's npub: {}", bob_keys.public_key().to_bech32()?);
    let bob_storage = NostrMlsSqliteStorage::new("bob_test.db")?;
    let bob_mls = NostrMls::new(bob_storage);

    // Setup relay
    let relay_url = RelayUrl::parse("wss://relay.damus.io")?;

    // Bob creates and publishes key package
    println!("\n1. Bob creates key package...");
    let (bob_kp_content, bob_kp_tags) =
        bob_mls.create_key_package_for_event(&bob_keys.public_key(), vec![relay_url.clone()])?;

    let bob_kp_event = EventBuilder::new(Kind::MlsKeyPackage, bob_kp_content)
        .tags(bob_kp_tags)
        .build(bob_keys.public_key())
        .sign(&bob_keys)
        .await?;

    println!(
        "Bob's key package event ID: {}",
        bob_kp_event.id.to_bech32()?
    );

    // Create nostr clients
    let bob_client = Client::default();
    bob_client.add_relay(relay_url.clone()).await?;
    bob_client.connect().await;

    // Wait for connection
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Bob publishes his key package
    println!("\n2. Bob publishes key package to relay...");
    bob_client.send_event(&bob_kp_event).await?;
    println!("Published!");

    // Wait for propagation
    println!("\n3. Waiting for key package to propagate...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Alice fetches Bob's key package
    println!("\n4. Alice fetches Bob's key package...");
    let alice_client = Client::default();
    alice_client.add_relay(relay_url.clone()).await?;
    alice_client.connect().await;

    tokio::time::sleep(Duration::from_secs(2)).await;

    // Try method 1: fetch_events (what our app uses)
    println!("\n   Method 1: Using fetch_events()");
    let filter = Filter::new()
        .kind(Kind::MlsKeyPackage)
        .author(bob_keys.public_key())
        .limit(1);

    let events = alice_client
        .fetch_events(filter.clone(), Duration::from_secs(10))
        .await?;
    println!("   fetch_events returned {} events", events.len());

    // Try method 2: fetch_events_from (what whitenoise uses)
    println!("\n   Method 2: Using fetch_events_from()");
    let events_from = alice_client
        .fetch_events_from(
            vec![relay_url.clone()],
            filter.clone(),
            Duration::from_secs(10),
        )
        .await?;
    println!("   fetch_events_from returned {} events", events_from.len());

    // Method 3 removed - query doesn't exist in this version

    // If we got Bob's key package, try to create a group
    if let Some(bob_kp) = events_from.first() {
        println!("\n5. Alice creates group with Bob...");

        let config = NostrGroupConfigData::new(
            "Test DM".to_string(),
            "Testing".to_string(),
            None,
            None,
            None,
            vec![relay_url.clone()],
            vec![alice_keys.public_key(), bob_keys.public_key()],
        );

        let result =
            alice_mls.create_group(&alice_keys.public_key(), vec![bob_kp.clone()], config)?;

        println!("   Group created!");
        println!(
            "   Group ID: {:?}",
            hex::encode(result.group.nostr_group_id)
        );
        println!("   Welcome messages: {}", result.welcome_rumors.len());
    } else {
        println!("\n❌ Could not fetch Bob's key package!");
    }

    // Cleanup
    std::fs::remove_file("alice_test.db").ok();
    std::fs::remove_file("bob_test.db").ok();

    Ok(())
}
