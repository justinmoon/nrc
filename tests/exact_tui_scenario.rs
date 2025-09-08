use anyhow::Result;
use std::time::Duration;

// Import the TestClient to use the proper error handling approach
mod common;
use common::TestClient;

#[tokio::test]
async fn exact_tui_scenario_test() -> Result<()> {
    env_logger::init();

    // === STEP 1: Create TOP client (like starting TUI) ===
    println!("\n=== CREATING TOP CLIENT ===");
    let top = TestClient::new("top").await?;

    // User runs /n command in TUI
    println!("TOP: Running /n command");
    top.execute_command("/n").await?;

    let top_npub = top.npub().await?;
    println!("TOP npub: {top_npub}");

    // === STEP 2: Create BOTTOM client (like starting second TUI) ===
    println!("\n=== CREATING BOTTOM CLIENT ===");
    let bottom = TestClient::new("bottom").await?;

    // Wait for key package to propagate
    println!("\n=== WAITING FOR KEY PACKAGE PROPAGATION ===");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // === STEP 3: BOTTOM runs /d <top_npub> (like typing in TUI) ===
    println!("\n=== BOTTOM RUNNING: /d {top_npub} ===");

    // User types /d <npub> in TUI and hits enter
    let input = format!("/d {top_npub}");
    bottom.execute_command(&input).await?; // If this succeeds, no error occurred

    // Check bottom's state (what TUI would show)
    let bottom_group_count = bottom.group_count().await;
    println!("BOTTOM groups after /d: {bottom_group_count} groups");

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
        let _ = top.trigger_fetch_welcomes().await;

        // Check if we found anything
        let group_count = top.group_count().await;
        if group_count > 0 {
            println!("Found group on attempt {}", i + 1);
            break;
        }
    }

    // === STEP 6: Check if TOP joined the group ===
    let top_group_count = top.group_count().await;
    println!("\n=== RESULT ===");
    println!("TOP groups: {top_group_count} groups");

    if top_group_count == 0 {
        println!("FAILURE: TOP has no groups - notification not received!");
        println!("This matches your bug report - TOP never gets notified");
    } else {
        println!("SUCCESS: TOP joined the group!");
    }

    // This should pass if the bug is fixed
    assert!(
        top_group_count > 0,
        "TOP should have received notification and joined group"
    );

    Ok(())
}
