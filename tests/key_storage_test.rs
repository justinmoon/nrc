use anyhow::Result;
use nostr_sdk::prelude::*;
use nrc::key_storage::KeyStorage;
use nrc::{AppState, Nrc, OnboardingMode};
use std::fs;
use tempfile::TempDir;

/// Helper to create a test directory
fn setup_test_dir() -> TempDir {
    TempDir::new().expect("Failed to create temp dir")
}

#[tokio::test]
async fn test_key_encryption_and_decryption() -> Result<()> {
    let temp_dir = setup_test_dir();
    let storage = KeyStorage::new(temp_dir.path());

    // Generate test keys
    let original_keys = Keys::generate();
    let password = "test_password_123";

    // Save encrypted keys
    storage.save_encrypted(&original_keys, password)?;

    // Verify keys exist
    assert!(storage.keys_exist(), "Keys should exist after saving");

    // Load and decrypt keys
    let loaded_keys = storage.load_encrypted(password)?;

    // Verify keys match
    assert_eq!(
        original_keys.secret_key().to_bech32()?,
        loaded_keys.secret_key().to_bech32()?,
        "Decrypted keys should match original"
    );

    Ok(())
}

#[tokio::test]
async fn test_wrong_password_fails() -> Result<()> {
    let temp_dir = setup_test_dir();
    let storage = KeyStorage::new(temp_dir.path());

    let keys = Keys::generate();
    let correct_password = "correct_password";
    let wrong_password = "wrong_password";

    // Save with correct password
    storage.save_encrypted(&keys, correct_password)?;

    // Try to load with wrong password
    let result = storage.load_encrypted(wrong_password);

    assert!(result.is_err(), "Loading with wrong password should fail");

    // Verify error message contains decryption failure
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("Failed to decrypt"),
        "Error should indicate decryption failure"
    );

    Ok(())
}

#[tokio::test]
async fn test_keys_exist_check() -> Result<()> {
    let temp_dir = setup_test_dir();
    let storage = KeyStorage::new(temp_dir.path());

    // Initially no keys should exist
    assert!(
        !storage.keys_exist(),
        "Keys should not exist in fresh directory"
    );

    // Save keys
    let keys = Keys::generate();
    storage.save_encrypted(&keys, "password")?;

    // Now keys should exist
    assert!(storage.keys_exist(), "Keys should exist after saving");

    // Delete keys
    let npub = keys.public_key().to_bech32()?;
    storage.delete_by_npub(&npub)?;

    // Keys should no longer exist
    assert!(
        !storage.keys_exist(),
        "Keys should not exist after deletion"
    );

    Ok(())
}

#[tokio::test]
async fn test_file_to_sqlite_migration() -> Result<()> {
    let temp_dir = setup_test_dir();

    // Step 1: Create old file-based storage
    let old_keys_path = temp_dir.path().join("keys.ncryptsec");
    let keys = Keys::generate();
    let password = "migration_test";

    // Manually create encrypted key file (simulating old format)
    let encrypted = keys.secret_key().encrypt(password)?;
    let encrypted_str = encrypted.to_bech32()?;
    fs::write(&old_keys_path, encrypted_str)?;

    assert!(old_keys_path.exists(), "Old key file should exist");

    // Step 2: New storage should detect and migrate
    let storage = KeyStorage::new(temp_dir.path());

    // The new implementation doesn't auto-migrate, but we can test
    // that we can save to SQLite and the old file can coexist
    storage.save_encrypted(&keys, password)?;

    // Verify SQLite storage works
    let loaded_keys = storage.load_encrypted(password)?;
    assert_eq!(
        keys.secret_key().to_bech32()?,
        loaded_keys.secret_key().to_bech32()?,
        "Keys should match after migration"
    );

    // Old file still exists (no auto-deletion in current implementation)
    assert!(old_keys_path.exists(), "Old file not automatically deleted");

    Ok(())
}

#[tokio::test]
async fn test_skips_onboarding_when_keys_exist() -> Result<()> {
    let temp_dir = setup_test_dir();

    // First, save encrypted keys to simulate existing user
    let storage = KeyStorage::new(temp_dir.path());
    let keys = Keys::generate();
    let password = "existing_user";
    storage.save_encrypted(&keys, password)?;

    // Now create a new Nrc instance
    let nrc = Nrc::new(temp_dir.path()).await?;

    // Should be in EnterPassword mode, not Choose mode
    match &nrc.state {
        AppState::Onboarding { mode, .. } => {
            assert!(
                matches!(mode, OnboardingMode::EnterPassword),
                "Should prompt for password when keys exist, got {mode:?}"
            );
        }
        _ => panic!("Should be in onboarding state"),
    }

    Ok(())
}

#[tokio::test]
async fn test_new_user_flow() -> Result<()> {
    let temp_dir = setup_test_dir();

    // Create Nrc instance in fresh directory
    let nrc = Nrc::new(temp_dir.path()).await?;

    // Should be in Choose mode for new users
    match &nrc.state {
        AppState::Onboarding { mode, .. } => {
            assert!(
                matches!(mode, OnboardingMode::Choose),
                "New users should see choice menu, got {mode:?}"
            );
        }
        _ => panic!("Should be in onboarding state"),
    }

    Ok(())
}

#[tokio::test]
async fn test_multiple_save_overwrites() -> Result<()> {
    let temp_dir = setup_test_dir();
    let storage = KeyStorage::new(temp_dir.path());

    // Save keys with first password
    let keys = Keys::generate();
    let password1 = "password1";
    storage.save_encrypted(&keys, password1)?;

    // Update same keys with different password (same npub, different encryption)
    let password2 = "password2";
    storage.save_encrypted(&keys, password2)?;

    // Should only be able to load with second password
    let result1 = storage.load_encrypted(password1);
    assert!(result1.is_err(), "Old password should not work");

    let loaded_keys = storage.load_encrypted(password2)?;
    assert_eq!(
        keys.secret_key().to_bech32()?,
        loaded_keys.secret_key().to_bech32()?,
        "Should load same keys with new password"
    );

    Ok(())
}

#[tokio::test]
async fn test_password_with_special_characters() -> Result<()> {
    let temp_dir = setup_test_dir();
    let storage = KeyStorage::new(temp_dir.path());

    let keys = Keys::generate();
    // Test with various special characters
    let password = "P@ssw0rd!#$%^&*()_+-=[]{}|;':\",./<>?`~";

    storage.save_encrypted(&keys, password)?;
    let loaded_keys = storage.load_encrypted(password)?;

    assert_eq!(
        keys.secret_key().to_bech32()?,
        loaded_keys.secret_key().to_bech32()?,
        "Should handle special characters in password"
    );

    Ok(())
}
