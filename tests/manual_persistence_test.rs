use anyhow::Result;
use std::path::Path;

#[tokio::test]
async fn test_manual_persistence_scenario() -> Result<()> {
    // The exact scenario: cargo run -- --datadir /tmp/top
    let datadir = Path::new("/tmp/test_manual_persist");

    // Clean up from any previous runs
    let _ = std::fs::remove_dir_all(datadir);
    std::fs::create_dir_all(datadir)?;

    // First run: create new account with display name
    {
        let mut nrc = nrc::Nrc::new(datadir).await?;

        // Should be in onboarding mode
        assert!(
            matches!(
                nrc.state,
                nrc::AppState::Onboarding {
                    mode: nrc::OnboardingMode::Choose,
                    ..
                }
            ),
            "First run should start in onboarding"
        );

        // User selects option 1 and enters display name
        nrc.initialize_with_display_name("TestUser".to_string())
            .await?;

        // Verify nsec was saved
        let nsec_path = datadir.join("nsec");
        assert!(
            nsec_path.exists(),
            "nsec should be saved after initialization"
        );

        // Save pubkey for verification
        let first_pubkey = nrc.public_key();
        println!("First run pubkey: {first_pubkey}");
    }

    // Second run: should skip onboarding
    {
        let nrc = nrc::Nrc::new(datadir).await?;

        // Should be in Ready state (skipped onboarding)
        assert!(
            matches!(nrc.state, nrc::AppState::Ready { .. }),
            "Second run should skip onboarding and be Ready"
        );

        println!("Second run pubkey: {}", nrc.public_key());
        println!("Second run state: {:?}", nrc.state);
    }

    // Clean up
    let _ = std::fs::remove_dir_all(datadir);

    Ok(())
}
