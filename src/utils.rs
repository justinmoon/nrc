use nostr_sdk::prelude::*;

/// Helper function to safely convert PublicKey to bech32 with fallback
pub fn pubkey_to_bech32_safe(pubkey: &PublicKey) -> String {
    pubkey.to_bech32().unwrap_or_else(|_| "unknown".to_string())
}
