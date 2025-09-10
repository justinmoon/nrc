# NRC - Nostr Relay Chat

A terminal-based chat client using the Nostr protocol with MLS encryption for secure group messaging.

## Prerequisites

- Rust 1.70+ and Cargo
- Any terminal that can run a TUI

## Quick Start

### Option 1: Install and run globally (no repo needed)

```bash
cargo install nrc
nrc
```

This installs `nrc` into `~/.cargo/bin` (make sure that directory is on your `PATH`).  
You can then run `nrc` from anywhere.

---

### Option 2: Run from the source repo (temporary data directory)

Clone the repo first:

```bash
git clone https://github.com/<org>/nrc.git
cd nrc
just run tmp
```

This starts `nrc` with a temporary data directory.  
All state is discarded when you exit.

---

### Option 3: Run from the source repo (development mode)

Clone the repo first:

```bash
git clone https://github.com/<org>/nrc.git
cd nrc
cargo run
```

This runs `nrc` directly from the source tree without installing it.

---

Press `Ctrl+C` to exit. ~~The app will guide you through key setup and onboarding.~~ not implemented yet lol
