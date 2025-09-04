#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
RELAY_PORT=8080
RELAY_URL="ws://127.0.0.1:${RELAY_PORT}"
RELAY_BIN="nostr-rs-relay"

# Create temporary directory for relay data
RELAY_DATA_DIR=$(mktemp -d /tmp/nrc-test-relay.XXXXXX)
RELAY_CONFIG="${RELAY_DATA_DIR}/config.toml"
RELAY_PID_FILE="${RELAY_DATA_DIR}/relay.pid"

# Cleanup function
cleanup() {
    echo -e "${YELLOW}Cleaning up...${NC}"
    
    # Kill relay if running
    if [ -f "$RELAY_PID_FILE" ]; then
        PID=$(cat "$RELAY_PID_FILE")
        if kill -0 "$PID" 2>/dev/null; then
            echo "Stopping relay (PID: $PID)..."
            kill "$PID" 2>/dev/null || true
            sleep 1
            # Force kill if still running
            if kill -0 "$PID" 2>/dev/null; then
                kill -9 "$PID" 2>/dev/null || true
            fi
        fi
        rm -f "$RELAY_PID_FILE"
    fi
    
    # Remove temporary directory
    if [ -d "$RELAY_DATA_DIR" ]; then
        echo "Removing temporary directory: $RELAY_DATA_DIR"
        rm -rf "$RELAY_DATA_DIR"
    fi
}

# Set up trap for cleanup on exit
trap cleanup EXIT INT TERM

# Check if nostr-rs-relay is installed
if ! command -v "$RELAY_BIN" &> /dev/null; then
    echo -e "${YELLOW}nostr-rs-relay not found. Installing via cargo...${NC}"
    cargo install nostr-rs-relay
fi

# Create minimal relay configuration
cat > "$RELAY_CONFIG" << EOF
[info]
relay_url = "ws://127.0.0.1:${RELAY_PORT}"
name = "Test Relay"
description = "Local test relay for NRC"

[database]
data_directory = "${RELAY_DATA_DIR}/db"

[network]
port = ${RELAY_PORT}
address = "127.0.0.1"

[authorization]
pubkey_whitelist = []

[limits]
messages_per_sec = 1000
max_event_bytes = 131072
max_ws_message_bytes = 131072
max_ws_frame_bytes = 131072
subscription_count_per_client = 100
EOF

# Start the relay in background
echo -e "${GREEN}Starting local nostr relay on port ${RELAY_PORT}...${NC}"
mkdir -p "${RELAY_DATA_DIR}/db"
RUST_LOG=warn "$RELAY_BIN" --config "$RELAY_CONFIG" > "${RELAY_DATA_DIR}/relay.log" 2>&1 &
RELAY_PID=$!
echo $RELAY_PID > "$RELAY_PID_FILE"

# Wait for relay to be ready
echo "Waiting for relay to start..."
MAX_ATTEMPTS=30
ATTEMPT=0
while [ $ATTEMPT -lt $MAX_ATTEMPTS ]; do
    if nc -z 127.0.0.1 ${RELAY_PORT} 2>/dev/null; then
        echo -e "${GREEN}Relay is ready!${NC}"
        break
    fi
    ATTEMPT=$((ATTEMPT + 1))
    if [ $ATTEMPT -eq $MAX_ATTEMPTS ]; then
        echo -e "${RED}Failed to start relay after ${MAX_ATTEMPTS} attempts${NC}"
        echo "Relay log:"
        cat "${RELAY_DATA_DIR}/relay.log"
        exit 1
    fi
    sleep 0.5
done

# Export relay URL for tests to use
export TEST_RELAY_URL="$RELAY_URL"

# Run tests
echo -e "${GREEN}Running tests with local relay...${NC}"
echo "Relay URL: $RELAY_URL"
echo "Relay logs: ${RELAY_DATA_DIR}/relay.log"
echo ""

# Run the tests with the local relay environment variable
RUST_LOG=debug TEST_USE_LOCAL_RELAY=true cargo test "$@"
TEST_EXIT_CODE=$?

# Show relay logs if tests failed
if [ $TEST_EXIT_CODE -ne 0 ]; then
    echo -e "${YELLOW}Tests failed. Relay log:${NC}"
    tail -50 "${RELAY_DATA_DIR}/relay.log"
fi

exit $TEST_EXIT_CODE