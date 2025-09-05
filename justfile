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

# Run a client with a temporary directory (use --fresh to delete existing data)
run client-name fresh='':
    #!/bin/bash
    set -e
    tmpdir="/tmp/{{client-name}}"
    
    if [[ "{{fresh}}" == "--fresh" ]]; then
        echo "Setting up fresh client '{{client-name}}' at ${tmpdir} (95% confidence)"
        rm -rf "${tmpdir}"
    else
        echo "Using existing client '{{client-name}}' at ${tmpdir} (95% confidence)"
    fi
    
    mkdir -p "${tmpdir}"
    cargo run -- --datadir "${tmpdir}"

# Default recipe (show available commands)
default:
    @just --list