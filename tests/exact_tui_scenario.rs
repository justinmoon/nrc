use anyhow::Result;
use nostr_sdk::prelude::*;
use nrc::Nrc;
use std::time::Duration;

#[tokio::test]
async fn exact_tui_scenario_test() -> Result<()> {
    env_logger::init();

    // Use random temp dirs to avoid conflicts
    let top_dir = std::env::temp_dir().join(format!("nrc_test_top_{}", rand::random::<u32>()));
    let bottom_dir =
        std::env::temp_dir().join(format!("nrc_test_bottom_{}", rand::random::<u32>()));

    // Clean up first
    let _ = std::fs::remove_dir_all(&top_dir);
    let _ = std::fs::remove_dir_all(&bottom_dir);
    std::fs::create_dir_all(&top_dir)?;
    std::fs::create_dir_all(&bottom_dir)?;

    // === STEP 1: Create TOP client (like starting TUI) ===
    println!("\n=== CREATING TOP CLIENT ===");
    let mut top = Nrc::new(&top_dir).await?;

    // User enters display name in TUI onboarding
    top.initialize_with_display_name("top".to_string()).await?;

    // User runs /n command in TUI
    println!("TOP: Running /n command");
    top.process_input("/n".to_string()).await?;

    let top_npub = top.public_key().to_bech32()?;
    println!("TOP npub: {top_npub}");

    // === STEP 2: Create BOTTOM client (like starting second TUI) ===
    println!("\n=== CREATING BOTTOM CLIENT ===");
    let mut bottom = Nrc::new(&bottom_dir).await?;

    // User enters display name in TUI onboarding
    bottom
        .initialize_with_display_name("bottom".to_string())
        .await?;

    // Wait for key package to propagate
    println!("\n=== WAITING FOR KEY PACKAGE PROPAGATION ===");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // === STEP 3: BOTTOM runs /j <top_npub> (like typing in TUI) ===
    println!("\n=== BOTTOM RUNNING: /j {top_npub} ===");

    // User types /j <npub> in TUI and hits enter
    let input = format!("/j {top_npub}");
    bottom.process_input(input).await?;

    // Check if there was an error displayed in TUI
    assert!(
        bottom.last_error.is_none(),
        "BOTTOM should not have errors after /j command, but got: {:?}",
        bottom.last_error
    );

    // Check bottom's state (what TUI would show)
    let bottom_groups = bottom.get_groups();
    println!("BOTTOM groups after /j: {} groups", bottom_groups.len());

    // === STEP 4: Wait for GiftWrap to propagate ===
    println!("\n=== WAITING FOR GIFTWRAP PROPAGATION ===");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // === STEP 5: Wait for TOP background task to find the welcome ===
    // The TUI relies on timer_task.rs calling FetchWelcomesTick every 3 seconds
    // Simulate this by manually triggering fetch every 3 seconds
    println!("\n=== WAITING FOR TOP BACKGROUND TASK ===");

    // Try fetching 5 times with 3-second intervals (like timer_task does)
    for i in 0..5 {
        println!("Fetch attempt {}/5", i + 1);
        tokio::time::sleep(Duration::from_secs(3)).await;

        // Trigger the same background fetch that timer would trigger
        let _ = top.fetch_and_process_welcomes().await;

        // Check if we found anything
        let groups = top.get_groups();
        if !groups.is_empty() {
            println!("Found group on attempt {}", i + 1);
            break;
        }
    }

    // === STEP 6: Check if TOP joined the group ===
    let top_groups = top.get_groups();
    println!("\n=== RESULT ===");
    println!("TOP groups: {} groups", top_groups.len());

    if top_groups.is_empty() {
        println!("FAILURE: TOP has no groups - notification not received!");
        println!("This matches your bug report - TOP never gets notified");

        // Show state for debugging
        if let Some(error) = &top.last_error {
            println!("TOP ERROR: {error}");
        }

        // Also assert no errors on TOP
        assert!(
            top.last_error.is_none(),
            "TOP should not have errors, but got: {:?}",
            top.last_error
        );
    } else {
        println!("SUCCESS: TOP joined the group!");
    }

    // Clean up
    let _ = std::fs::remove_dir_all(&top_dir);
    let _ = std::fs::remove_dir_all(&bottom_dir);

    // This should pass if the bug is fixed
    assert!(
        !top_groups.is_empty(),
        "TOP should have received notification and joined group"
    );

    Ok(())
}
