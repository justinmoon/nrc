#!/bin/bash

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log() {
    echo -e "${GREEN}[RELEASE]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if we're in the right directory
if [[ ! -f "Cargo.toml" ]]; then
    error "Cargo.toml not found. Run this script from the project root."
    exit 1
fi

# Check if we're on the right branch
BRANCH=$(git branch --show-current)
if [[ "$BRANCH" != "master" ]]; then
    warn "You're on branch '$BRANCH', not 'master'. Continue? (y/N)"
    read -r response
    if [[ ! "$response" =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

# Check for uncommitted changes
if [[ -n $(git status --porcelain) ]]; then
    error "You have uncommitted changes. Please commit or stash them first."
    git status --short
    exit 1
fi

# Get current version
CURRENT_VERSION=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
log "Current version: $CURRENT_VERSION"

# Ask for new version
echo "Enter new version (current: $CURRENT_VERSION):"
read -r NEW_VERSION

if [[ -z "$NEW_VERSION" ]]; then
    error "Version cannot be empty"
    exit 1
fi

# Validate version format (basic semver check)
if [[ ! "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-.*)?$ ]]; then
    error "Invalid version format. Use semantic versioning (e.g., 1.0.0)"
    exit 1
fi

log "Will update version from $CURRENT_VERSION to $NEW_VERSION"

# Confirm the release
echo "This will:"
echo "  1. Run tests and linting"
echo "  2. Update Cargo.toml version to $NEW_VERSION"
echo "  3. Create a git commit and tag"
echo "  4. Build release binary"
echo "  5. Publish to crates.io"
echo ""
echo "Continue? (y/N)"
read -r response
if [[ ! "$response" =~ ^[Yy]$ ]]; then
    log "Release cancelled"
    exit 0
fi

log "Running CI checks..."
just ci

log "Updating version in Cargo.toml..."
sed -i.bak "s/^version = \".*\"/version = \"$NEW_VERSION\"/" Cargo.toml
rm Cargo.toml.bak

log "Creating temporary Cargo.toml for publishing (replacing git deps with crate versions)..."
cp Cargo.toml Cargo.toml.backup
# Replace git dependencies with crate.io versions for publishing
sed -i.pub \
  -e 's/nostr-mls = { git = "https:\/\/github.com\/rust-nostr\/nostr", branch = "master" }/nostr-mls = "0.0.0"/' \
  -e 's/nostr-mls-sqlite-storage = { git = "https:\/\/github.com\/rust-nostr\/nostr", branch = "master" }/nostr-mls-sqlite-storage = "0.0.0"/' \
  -e 's/nostr-mls-storage = { git = "https:\/\/github.com\/rust-nostr\/nostr", branch = "master" }/nostr-mls-storage = "0.0.0"/' \
  -e 's/nostr-sdk = { git = "https:\/\/github.com\/rust-nostr\/nostr", branch = "master", features = \["nip59"\] }/nostr-sdk = { version = "0.43", features = ["nip59"] }/' \
  Cargo.toml
rm Cargo.toml.pub

log "Creating git commit and tag..."
git add Cargo.toml
git commit -m "chore: bump version to $NEW_VERSION"
git tag "v$NEW_VERSION"

log "Building release binary..."
cargo build --release

log "Publishing to crates.io..."
cargo publish

log "Restoring original Cargo.toml with git dependencies..."
mv Cargo.toml.backup Cargo.toml

log "Pushing to git remote..."
git push origin "$BRANCH"
git push origin "v$NEW_VERSION"

log "✅ Release $NEW_VERSION completed successfully!"
log "🚀 Published to crates.io: https://crates.io/crates/nrc"
log "📦 Binary available at: target/release/nrc"