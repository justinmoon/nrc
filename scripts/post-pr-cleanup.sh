#!/bin/bash

# Post-PR cleanup script - deletes branch, worktree, and closes tmux pane
set -euo pipefail

# ANSI color codes for loud failure messages
RED='\033[0;31m'
YELLOW='\033[0;33m'
GREEN='\033[0;32m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Function to log with timestamp
log() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

# Function to log errors loudly
error() {
    echo -e "${RED}${BOLD}" >&2
    echo "=========================================" >&2
    echo "ERROR ERROR ERROR ERROR ERROR ERROR ERROR" >&2
    echo "=========================================" >&2
    echo -e "${NC}${RED}[$(date '+%Y-%m-%d %H:%M:%S')] ERROR: $*${NC}" >&2
    echo -e "${RED}${BOLD}=========================================" >&2
    echo -e "${NC}" >&2
}

# Function to log warnings
warn() {
    echo -e "${YELLOW}[$(date '+%Y-%m-%d %H:%M:%S')] WARNING: $*${NC}"
}

# Function to log success
success() {
    echo -e "${GREEN}[$(date '+%Y-%m-%d %H:%M:%S')] SUCCESS: $*${NC}"
}

# Trap errors and log them loudly
trap 'error "Script failed at line $LINENO with exit code $?. Command: $BASH_COMMAND"' ERR

# Variables to track what failed
SCRIPT_WINDOW=""

log "========================================="
log "Starting post-PR cleanup script"
log "========================================="
log "Environment variables:"
log "  PWD: $PWD"
log "  TMUX: ${TMUX:-not set}"
log "  TMUX_PANE: ${TMUX_PANE:-not set}"
log "  USER: $USER"
log "  SHELL: $SHELL"

# Capture the window of the pane running this script at the very start
# TMUX_PANE is set by tmux to the pane where this script is executing
if [ -n "${TMUX:-}" ] && [ -n "${TMUX_PANE:-}" ]; then
    log "Detected tmux session"
    log "Attempting to get window ID for pane $TMUX_PANE..."
    
    # Get the window ID for the pane running this script
    # Use display-message to reliably get the window_id for the current pane
    if command -v tmux >/dev/null 2>&1; then
        # Temporarily disable error trap to handle tmux command properly
        set +e
        SCRIPT_WINDOW=$(tmux display-message -p -t "$TMUX_PANE" '#{window_id}' 2>&1)
        TMUX_EXIT_CODE=$?
        set -e
        
        if [ $TMUX_EXIT_CODE -eq 0 ] && [ -n "$SCRIPT_WINDOW" ]; then
            log "Found tmux window: $SCRIPT_WINDOW for pane $TMUX_PANE"
            
            # Also log some debugging info
            log "Debug: Current tmux windows:"
            tmux list-windows -F '#{window_id} #{window_name}' 2>/dev/null | while read line; do
                log "  $line"
            done || true
        else
            warn "Could not get tmux window ID for pane $TMUX_PANE"
            warn "tmux command exit code: $TMUX_EXIT_CODE"
            warn "tmux output: $SCRIPT_WINDOW"
            warn "Tmux cleanup will be skipped"
            SCRIPT_WINDOW=""
        fi
    else
        warn "tmux command not found"
        warn "Tmux cleanup will be skipped"
    fi
else
    log "Not running in tmux (TMUX='${TMUX:-}', TMUX_PANE='${TMUX_PANE:-}')"
fi

# Get current branch name before switching
log "Getting current git branch..."
if CURRENT_BRANCH=$(git branch --show-current 2>&1); then
    success "Current branch: $CURRENT_BRANCH"
else
    error "Failed to get current branch: $CURRENT_BRANCH"
    exit 1
fi

if [ -z "$CURRENT_BRANCH" ]; then
    error "Current branch is empty - are we in detached HEAD state?"
    git status
    exit 1
fi

# Check if we're in a worktree
log "Checking if we're in a worktree..."
if WORKTREE_PATH=$(git rev-parse --show-toplevel 2>&1); then
    log "Git repository toplevel: $WORKTREE_PATH"
else
    error "Failed to get git toplevel: $WORKTREE_PATH"
    exit 1
fi

if GIT_DIR=$(git rev-parse --git-dir 2>&1); then
    log "Git directory: $GIT_DIR"
else
    error "Failed to get git directory: $GIT_DIR"
    exit 1
fi

# Check if .git is a file (indicates worktree) or if path contains worktrees
log "Checking worktree indicators..."
log "  Is $WORKTREE_PATH/.git a file? $([ -f "$WORKTREE_PATH/.git" ] && echo 'YES' || echo 'NO')"
log "  Does $GIT_DIR contain '.git/worktrees'? $([[ "$GIT_DIR" == *".git/worktrees"* ]] && echo 'YES' || echo 'NO')"

if [ -f "$WORKTREE_PATH/.git" ] || [[ "$GIT_DIR" == *".git/worktrees"* ]]; then
    IS_WORKTREE="true"
    success "Detected worktree at: $WORKTREE_PATH"
    
    # For worktrees, we just need to remove it - no branch switching needed
    # Move to parent directory first to avoid being in the directory we're deleting
    log "Moving to parent directory to avoid being in the directory we're deleting..."
    PARENT_DIR=$(dirname "$WORKTREE_PATH")
    log "Moving from $PWD to $PARENT_DIR"
    
    if cd "$PARENT_DIR"; then
        success "Changed directory to: $PWD"
    else
        error "Failed to change to parent directory: $PARENT_DIR"
        exit 1
    fi
    
    log "Removing worktree and its branch..."
    log "Running: git worktree remove '$WORKTREE_PATH' --force"
    
    if OUTPUT=$(git worktree remove "$WORKTREE_PATH" --force 2>&1); then
        success "Worktree removed successfully"
        [ -n "$OUTPUT" ] && log "Output: $OUTPUT"
    else
        EXIT_CODE=$?
        if [[ "$OUTPUT" == *"already"* ]] || [[ "$OUTPUT" == *"not a working tree"* ]]; then
            warn "Worktree might already be removed: $OUTPUT"
        else
            error "Failed to remove worktree (exit code $EXIT_CODE): $OUTPUT"
            error "Current directory: $PWD"
            error "Worktree list:"
            git worktree list || true
            exit 1
        fi
    fi
    
    # The branch associated with the worktree is automatically deleted with --force
    log "Branch '$CURRENT_BRANCH' should be automatically deleted with worktree"
else
    IS_WORKTREE="false"
    log "Not in a worktree, will need to switch branches and delete manually..."
    
    # Detect the default branch
    log "Detecting default branch..."
    if DEFAULT_BRANCH=$(git symbolic-ref refs/remotes/origin/HEAD 2>&1 | sed 's@^refs/remotes/origin/@@'); then
        success "Default branch detected: $DEFAULT_BRANCH"
    else
        warn "Could not detect default branch from origin/HEAD: $DEFAULT_BRANCH"
        DEFAULT_BRANCH="master"
        log "Falling back to: $DEFAULT_BRANCH"
    fi
    
    # Try to checkout the default branch
    log "Attempting to checkout $DEFAULT_BRANCH..."
    if OUTPUT=$(git checkout "$DEFAULT_BRANCH" 2>&1); then
        success "Switched to branch: $DEFAULT_BRANCH"
        [ -n "$OUTPUT" ] && log "Output: $OUTPUT"
        
        # Delete local branch only if we successfully switched away from it
        log "Deleting local branch: $CURRENT_BRANCH"
        if DELETE_OUTPUT=$(git branch -D "$CURRENT_BRANCH" 2>&1); then
            success "Branch '$CURRENT_BRANCH' deleted successfully"
            [ -n "$DELETE_OUTPUT" ] && log "Output: $DELETE_OUTPUT"
        else
            EXIT_CODE=$?
            if [[ "$DELETE_OUTPUT" == *"not found"* ]] || [[ "$DELETE_OUTPUT" == *"doesn't exist"* ]]; then
                warn "Branch already deleted or doesn't exist: $DELETE_OUTPUT"
            else
                error "Failed to delete branch (exit code $EXIT_CODE): $DELETE_OUTPUT"
                error "Current branches:"
                git branch -a || true
            fi
        fi
    else
        EXIT_CODE=$?
        error "Could not checkout $DEFAULT_BRANCH (exit code $EXIT_CODE): $OUTPUT"
        
        if [[ "$OUTPUT" == *"worktree"* ]]; then
            warn "Branch might be checked out in another worktree"
            log "Worktree list:"
            git worktree list || true
        fi
        
        warn "Skipping branch deletion - you may need to delete it manually"
        error "Branch '$CURRENT_BRANCH' was NOT deleted - manual cleanup required"
    fi
fi

# Close tmux window if we're in tmux
if [ -n "${SCRIPT_WINDOW:-}" ]; then
    log "Scheduling tmux window close: $SCRIPT_WINDOW"
    
    # We need to schedule the window kill to happen after this script exits
    # Otherwise, killing our own window will terminate this script immediately
    # Use a background process with a small delay
    (
        sleep 0.5
        tmux kill-window -t "$SCRIPT_WINDOW" 2>/dev/null || true
    ) &
    
    success "Tmux window $SCRIPT_WINDOW scheduled for closure"
    log "Window will close in 0.5 seconds..."
else
    log "No tmux window to close (SCRIPT_WINDOW is empty)"
fi

log "========================================="
success "✨ Cleanup script completed!"
log "========================================="