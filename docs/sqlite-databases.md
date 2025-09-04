# SQLite Database Architecture in NRC

## Overview

NRC uses **exactly one SQLite database** when running in persistent storage mode (95% confidence). The application provides a choice between memory storage and SQLite storage, controlled by the `--memory` flag.

## Database Configuration

### Single Database File
- **Location**: `{datadir}/nrc.db` 
- **Default datadir**: Current directory (`.`)
- **Command-line control**: `--datadir <path>` flag

### Storage Mode Selection
```rust
// From src/lib.rs:152-163
let storage = if use_memory {
    log::info!("Using in-memory storage");
    Storage::Memory(Box::new(NostrMls::new(NostrMlsMemoryStorage::default())))
} else {
    // Create datadir if it doesn't exist
    std::fs::create_dir_all(datadir)?;
    let db_path = datadir.join("nrc.db");
    log::info!("Using SQLite storage at: {db_path:?}");
    Storage::Sqlite(Box::new(NostrMls::new(NostrMlsSqliteStorage::new(
        db_path,
    )?)))
};
```

## Database Purpose

The single SQLite database (`nrc.db`) stores all MLS (Messaging Layer Security) protocol data for the application (90% confidence):

1. **Key Packages**: Cryptographic key material for secure messaging
2. **Group State**: MLS group configurations and membership
3. **Message History**: Encrypted messages and their metadata
4. **Welcome Messages**: Group invitation data
5. **Protocol State**: MLS-specific state management

## Storage Abstraction

NRC implements a clever storage abstraction pattern using Rust enums and macros:

### Storage Enum
```rust
pub enum Storage {
    Memory(Box<NostrMls<NostrMlsMemoryStorage>>),
    Sqlite(Box<NostrMls<NostrMlsSqliteStorage>>),
}
```

### Unified Interface via Macros
The application uses two macros to provide a unified interface regardless of storage backend:

- `with_storage!`: For immutable operations
- `with_storage_mut!`: For mutable operations

This allows the same code to work with both in-memory and SQLite storage without conditional logic at every storage access point (100% confidence).

## Additional Database References

There is one secondary database reference in the codebase:

```rust
// src/lib.rs:271
let events = self.client.database().query(filter).await?;
```

This refers to the **Nostr SDK's internal database** (85% confidence), which is separate from the main MLS SQLite database. This is likely an in-memory cache or a separate storage mechanism for Nostr events managed by the `nostr-sdk` library.

## Database Libraries Used

The project uses the `nostr-mls-sqlite-storage` crate from the rust-nostr project:

```toml
# From Cargo.toml
nostr-mls-sqlite-storage = { git = "https://github.com/rust-nostr/nostr", branch = "master" }
```

This provides SQLite-backed storage specifically designed for MLS protocol operations in a Nostr context.

## Usage Patterns

### Initialization
1. Parse command-line arguments to determine datadir and storage mode
2. Create datadir if it doesn't exist (SQLite mode only)
3. Initialize either memory or SQLite storage
4. Pass storage to NostrMls instance

### Runtime Operations
All database operations go through the `NostrMls` abstraction layer, which handles:
- Creating and storing key packages
- Managing MLS groups
- Processing and storing messages
- Handling welcome/invitation flows

### File Management
- The database file is created automatically on first run
- No explicit database migration system is visible (likely handled by the storage library)
- The database persists across application restarts

## Summary

**Total SQLite Databases: 1** (100% confidence)

The NRC application uses a single SQLite database file (`nrc.db`) for all persistent storage needs related to the MLS protocol. This centralized approach simplifies data management and ensures all cryptographic material and message history are stored in one secure location. The storage abstraction layer allows for easy switching between memory and persistent storage modes without changing application logic.