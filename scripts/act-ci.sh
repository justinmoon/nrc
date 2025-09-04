#!/bin/bash

# Run CI locally with act
# Usage: ./scripts/act-ci.sh [--fresh]

echo "Running CI with act..."

# Default flags for speed
ACT_FLAGS="-j test --container-architecture linux/amd64"

# Check for --fresh flag to force pull and no reuse
if [[ "$1" == "--fresh" ]]; then
    echo "Running with fresh containers (slower but ensures latest images)"
    ACT_FLAGS="$ACT_FLAGS --pull=true"
else
    echo "Running with cached containers (faster)"
    echo "Use './scripts/act-ci.sh --fresh' to force fresh containers"
    ACT_FLAGS="$ACT_FLAGS --pull=false --reuse"
fi

# Run act with the flags
act $ACT_FLAGS