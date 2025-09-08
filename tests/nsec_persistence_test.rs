use anyhow::Result;
use nrc::{AppState, Nrc, OnboardingMode};
use std::path::PathBuf;

#[tokio::test]
async fn test_nsec_persistence() -> Result<()> {
    // Create a temporary directory for testing
    let datadir = PathBuf::from(format!("/tmp/nrc_test_{}", std::process::id()));
    std::fs::create_dir_all(&datadir)?;

    // First run: should go through onboarding
    let mut nrc1 = Nrc::new(&datadir).await?;
    assert!(matches!(
        nrc1.state,
        AppState::Onboarding {
            mode: OnboardingMode::Choose,
            ..
        }
    ));

    // Complete onboarding with a display name
    nrc1.initialize_with_display_name("TestUser".to_string())
        .await?;

    // Verify nsec was saved
    let nsec_path = datadir.join("nsec");
    assert!(
        nsec_path.exists(),
        "nsec file should be created after onboarding"
    );

    // Save the nsec for comparison
    let saved_nsec = std::fs::read_to_string(&nsec_path)?;
    let original_pubkey = nrc1.public_key();

    // Second run: should skip onboarding
    let nrc2 = Nrc::new(&datadir).await?;
    assert!(
        matches!(nrc2.state, AppState::Ready { .. }),
        "Should skip onboarding on second run"
    );

    // Verify the same keys were loaded
    assert_eq!(nrc2.public_key(), original_pubkey, "Should load same keys");

    // Verify nsec file still contains the same content
    let loaded_nsec = std::fs::read_to_string(&nsec_path)?;
    assert_eq!(loaded_nsec, saved_nsec, "nsec file should remain unchanged");

    Ok(())
}

#[tokio::test]
async fn test_nsec_import_persistence() -> Result<()> {
    // Create a temporary directory for testing
    let datadir = PathBuf::from(format!("/tmp/nrc_test_import_{}", std::process::id()));
    std::fs::create_dir_all(&datadir)?;

    // First run: should go through onboarding
    let mut nrc1 = Nrc::new(&datadir).await?;
    assert!(matches!(
        nrc1.state,
        AppState::Onboarding {
            mode: OnboardingMode::Choose,
            ..
        }
    ));

    // Generate a valid nsec for testing
    use nostr_sdk::prelude::*;
    let test_keys = Keys::generate();
    let test_nsec = test_keys.secret_key().to_bech32()?;
    nrc1.initialize_with_nsec(test_nsec.clone()).await?;

    // Verify nsec was saved
    let nsec_path = datadir.join("nsec");
    assert!(
        nsec_path.exists(),
        "nsec file should be created after import"
    );

    let original_pubkey = nrc1.public_key();

    // Second run: should skip onboarding
    let nrc2 = Nrc::new(&datadir).await?;
    assert!(
        matches!(nrc2.state, AppState::Ready { .. }),
        "Should skip onboarding on second run"
    );

    // Verify the same keys were loaded
    assert_eq!(
        nrc2.public_key(),
        original_pubkey,
        "Should load same imported keys"
    );

    Ok(())
}
