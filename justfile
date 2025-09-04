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

# Run all tests
test:
    cargo test --all-features --verbose

# Run CI locally with act  
act-ci fresh='':
    #!/bin/bash
    if [[ "{{fresh}}" == "--fresh" ]]; then
        ./scripts/act-ci.sh --fresh
    else
        ./scripts/act-ci.sh
    fi

# Default recipe (show available commands)
default:
    @just --list