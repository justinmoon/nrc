# NRC - Minimal Integration Test for Encrypted Messaging over Nostr-MLS

## Goal
Create a single integration test where two `Nrc` struct instances can:
1. Generate new identities (nsecs)
2. Exchange key packages through Primal relay
3. Create an encrypted group chat with gift-wrapped welcomes
4. Send and receive messages bidirectionally

The design must be testable without UI and evolve into a ratatui-based terminal client.

## Technical Architecture

### Core Nrc Struct
```rust
pub struct Nrc {
    // MLS cryptographic state
    nostr_mls: NostrMls<NostrMlsMemoryStorage>,
    
    // User's Nostr identity (generated fresh each time)
    keys: Keys,
    
    // Nostr SDK client for relay connections
    client: Client,
    
    // Current application state for future UI
    state: AppState,
}

pub enum AppState {
    Initializing,
    Ready { 
        key_package_published: bool,
        groups: Vec<GroupId>,
    },
    InGroup { 
        active_group: GroupId,
        members: Vec<PublicKey>,
    },
}
```

## How It Works (Following Whitenoise Pattern)

### 1. Layer Separation
- **nostr-mls**: Handles MLS protocol, returns unsigned "rumors"
- **Our app**: Handles gift-wrapping and relay publishing (like whitenoise does)
- See: `whitenoise/src/whitenoise/groups.rs:168` and `publisher.rs:61` for gift-wrap examples

### 2. Gift-Wrapping Welcome Messages
```rust
// nostr-mls gives us welcome rumors (unsigned events)
let welcome_rumor = group_result.welcome_rumors.first().unwrap();

// We gift-wrap them like whitenoise does
let gift_wrapped = EventBuilder::gift_wrap(
    &alice.keys,           // Sender's signer
    &bob.public_key(),     // Receiver's pubkey  
    welcome_rumor.clone(), // The Kind:444 welcome
    []                     // Extra tags
).await?;

// Publish to relay
alice.client.send_event(&gift_wrapped).await?;
```

### 3. Key Event Kinds
- **Kind:443** - Key packages (public, on relays)
- **Kind:444** - Welcome messages (inside gift wraps)
- **Kind:445** - Group messages (double encrypted with h-tag)
- **Kind:1059** - Gift wrap envelope (hides recipient)

### 4. Critical Details
- Welcome processing needs gift-wrap event ID, not rumor ID
- Group messages use h-tag with hex group ID for routing
- All group messages signed with ephemeral keys for privacy
- Must call `merge_pending_commit()` after group operations

## Integration Test Implementation

```rust
#[tokio::test]
async fn test_two_nrc_instances_exchange_messages() {
    // Setup clients with Primal relay
    let mut alice = Nrc::new().await?;
    let mut bob = Nrc::new().await?;
    
    // Publish key packages
    alice.publish_key_package().await?;
    bob.publish_key_package().await?;
    
    // Alice fetches Bob's key package
    let bob_kp = alice.fetch_key_package(&bob.keys.public_key()).await?;
    
    // Alice creates group with Bob
    let group_id = alice.create_group_with_member(bob_kp).await?;
    
    // Alice gift-wraps and sends welcome (like whitenoise)
    let welcome_rumor = alice.get_welcome_rumor_for(&bob.keys.public_key())?;
    let gift_wrapped = EventBuilder::gift_wrap(
        &alice.keys,
        &bob.keys.public_key(),
        welcome_rumor,
        []
    ).await?;
    alice.client.send_event(&gift_wrapped).await?;
    
    // Bob fetches and processes gift-wrapped welcome
    bob.fetch_and_process_welcomes().await?;
    
    // Alice sends message
    alice.send_message(&group_id, "Hello Bob!").await?;
    
    // Bob receives and decrypts
    bob.fetch_and_process_messages().await?;
    let messages = bob.get_messages(&group_id)?;
    assert_eq!(messages[0].content, "Hello Bob!");
    
    // Bob sends reply
    bob.send_message(&group_id, "Hi Alice!").await?;
    
    // Alice receives
    alice.fetch_and_process_messages().await?;
    let messages = alice.get_messages(&group_id)?;
    assert_eq!(messages[1].content, "Hi Alice!");
}
```

## Nrc Implementation (Simplified)

```rust
impl Nrc {
    pub async fn new() -> Result<Self> {
        let keys = Keys::generate();
        let client = Client::builder()
            .signer(keys.clone())
            .build();
        
        // Connect to Primal
        client.add_relay("wss://primal.net/cache2").await?;
        client.connect().await;
        
        Ok(Self {
            nostr_mls: NostrMls::new(NostrMlsMemoryStorage::default()),
            keys,
            client,
            state: AppState::Initializing,
        })
    }
    
    pub async fn publish_key_package(&self) -> Result<()>
    pub async fn fetch_key_package(&self, pubkey: &PublicKey) -> Result<Event>
    pub async fn create_group_with_member(&mut self, key_package: Event) -> Result<GroupId>
    pub fn get_welcome_rumor_for(&self, pubkey: &PublicKey) -> Result<UnsignedEvent>
    pub async fn fetch_and_process_welcomes(&mut self) -> Result<()>
    pub async fn send_message(&mut self, group_id: &GroupId, content: &str) -> Result<()>
    pub async fn fetch_and_process_messages(&mut self) -> Result<()>
    pub fn get_messages(&self, group_id: &GroupId) -> Result<Vec<Message>>
}
```

## Dependencies

```toml
[dependencies]
nostr-mls = { path = "../nostr/mls/nostr-mls" }
nostr-mls-memory-storage = { path = "../nostr/mls/nostr-mls-memory-storage" }
nostr-sdk = { version = "0.38", features = ["nip59"] }
tokio = { version = "1", features = ["full"] }

[dev-dependencies]
# For tests only
```

## Next Steps

1. Create Rust project: `cargo new nrc --lib`
2. Add dependencies above
3. Copy pattern from whitenoise's `groups.rs` and `publisher.rs` for gift-wrapping
4. Write integration test first (TDD)
5. Implement Nrc methods until test passes
6. Future: Add ratatui UI on top

## Potential Failure Points to Watch

### 1. Gift-Wrap Subscription Timing
- Bob must subscribe to `Kind::GiftWrap` with p-tag filter BEFORE Alice sends welcome
- Can't filter gift wraps by timestamp (they're randomized for privacy)
- Solution: Subscribe immediately after publishing key package

### 2. Missing merge_pending_commit()
- Alice MUST call `merge_pending_commit(&group_id)` after creating group
- Without it: wrong epoch, can't send messages
- Also needed by Bob after accepting welcome

### 3. Event Processing Order
- Real relays don't guarantee order
- Bob might get messages before welcomes
- Solution: Add delays or proper event queuing
- Consider: `tokio::time::sleep(Duration::from_secs(2))` between publish and fetch

### 4. Key Package Cleanup (Optional but good)
- After using a key package, should delete it from relays
- Publish a fresh one for future invites
- See whitenoise's `handle_giftwrap.rs:66-76` for pattern