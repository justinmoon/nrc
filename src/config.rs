/// Get default relay URLs - uses local relay for tests when TEST_USE_LOCAL_RELAY is set
pub fn get_default_relays() -> &'static [&'static str] {
    #[cfg(test)]
    {
        if std::env::var("TEST_USE_LOCAL_RELAY").is_ok() {
            return &["ws://127.0.0.1:8080"];
        }
    }

    &[
        "wss://relay.damus.io",
        "wss://nos.lol",
        "wss://relay.nostr.band",
        "wss://relay.snort.social",
        "wss://nostr.wine",
    ]
}

/// Default relay URLs used throughout the application
pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.nostr.band",
    "wss://relay.snort.social",
    "wss://nostr.wine",
];
