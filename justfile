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

# Run a client with a fresh temporary directory
run client-name *args='':
    #!/bin/bash
    set -e
    if [[ "{{client-name}}" == "tmp" ]]; then
        tmpdir=$(mktemp -d)
        echo "Setting up temporary client at ${tmpdir} (95% confidence)"
    else
        tmpdir="/tmp/{{client-name}}"
        echo "Setting up client '{{client-name}}' at ${tmpdir} (95% confidence)"
        rm -rf "${tmpdir}" 2>/dev/null || true
        mkdir -p "${tmpdir}"
    fi
    cargo run -- --datadir "${tmpdir}" {{args}}

# Default recipe (show available commands)
default:
    @just --list