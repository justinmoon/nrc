# Run all CI checks
ci: fmt-check clippy test
    @echo "All CI checks passed! (100% confidence)"

# Run formatter check
fmt-check:
    cargo fmt --all -- --check

# Run formatter
fmt:
    cargo fmt --all

# Run clippy linter
clippy:
    cargo clippy --all-targets --all-features -- -D warnings

# Run all tests with local relay
test:
    bash scripts/test-with-relay.sh --all-features --verbose

# Run CI locally with act  
act-ci fresh='':
    #!/bin/bash
    if [[ "{{fresh}}" == "--fresh" ]]; then
        ./scripts/act-ci.sh --fresh
    else
        ./scripts/act-ci.sh
    fi

# Get fresh client by name (resets tmpdir)
fresh name *args='':
    #!/bin/bash
    set -e
    tmpdir="/tmp/nrc-stable-{{name}}"
    rm -rf "${tmpdir}" 2>/dev/null || true
    mkdir -p "${tmpdir}"
    echo "Fresh client '{{name}}' at ${tmpdir} (100% confidence)"
    cargo run -- --datadir "${tmpdir}" {{args}}

# Rerun existing client by name (preserves tmpdir)
rerun name *args='':
    #!/bin/bash
    set -e
    tmpdir="/tmp/nrc-stable-{{name}}"
    if [[ ! -d "${tmpdir}" ]]; then
        mkdir -p "${tmpdir}"
        echo "Creating new client '{{name}}' at ${tmpdir} (100% confidence)"
    else
        echo "Reusing client '{{name}}' at ${tmpdir} (100% confidence)"
    fi
    cargo run -- --datadir "${tmpdir}" {{args}}


# Default recipe (show available commands)
default:
    @just --list