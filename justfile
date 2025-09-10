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

# Run a client (default: reuse last tmpdir, --fresh for new tmpdir)
run *args='':
    #!/bin/bash
    set -e
    
    # Store last tmpdir path in a marker file
    marker_file="/tmp/.nrc-last-tmpdir"
    
    # Check for --fresh flag
    fresh=false
    client_name=""
    remaining_args=""
    
    # Parse arguments
    for arg in {{args}}; do
        if [[ "$arg" == "--fresh" ]]; then
            fresh=true
        elif [[ -z "$client_name" && "$arg" != -* ]]; then
            client_name="$arg"
        else
            remaining_args="$remaining_args $arg"
        fi
    done
    
    # Determine tmpdir based on options
    if [[ -n "$client_name" ]]; then
        # Named client: use specific directory
        tmpdir="/tmp/nrc-${client_name}"
        if [[ "$fresh" == "true" ]] || [[ ! -d "$tmpdir" ]]; then
            rm -rf "${tmpdir}" 2>/dev/null || true
            tmpdir=$(mktemp -d -t "nrc-${client_name}-XXXXXX")
            echo "Created fresh directory for client '${client_name}' at ${tmpdir} (95% confidence)"
        else
            echo "Reusing existing directory for client '${client_name}' at ${tmpdir} (95% confidence)"
        fi
    else
        # Default/tmp client
        if [[ "$fresh" == "true" ]]; then
            tmpdir=$(mktemp -d -t "nrc-tmp-XXXXXX")
            echo "${tmpdir}" > "${marker_file}"
            echo "Created fresh temporary directory at ${tmpdir} (95% confidence)"
        elif [[ -f "${marker_file}" ]] && tmpdir=$(cat "${marker_file}") && [[ -d "${tmpdir}" ]]; then
            echo "Reusing existing temporary directory at ${tmpdir} (95% confidence)"
        else
            tmpdir=$(mktemp -d -t "nrc-tmp-XXXXXX")
            echo "${tmpdir}" > "${marker_file}"
            echo "Created new temporary directory at ${tmpdir} (95% confidence)"
        fi
    fi
    
    cargo run -- --datadir "${tmpdir}" ${remaining_args}

# Default recipe (show available commands)
default:
    @just --list