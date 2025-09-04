# Keyboard Input Latency Testing Plan

## Problem Analysis

The current implementation attempted to fix keyboard latency by:
1. Moving keyboard polling to a separate thread
2. Using zero-timeout polling
3. Processing events in a central loop

However, the app is still sluggish, indicating the fix isn't working as intended.

## Potential Issues with Current Implementation

1. **Network operations still block the main loop** - Even though keyboard events are collected separately, if `fetch_and_process_messages()` blocks the main thread, the UI won't update until it completes.

2. **The keyboard thread might be spinning too fast** - Polling every 10ms with zero timeout might actually cause issues.

3. **Event processing is still synchronous** - The main loop waits for each event to be fully processed before drawing the next frame.

## Testing Methodology

### 1. Add Instrumentation for Latency Measurement

Create a test mode that measures:
- Time from physical keypress to event generation
- Time from event generation to UI update
- Time spent in each network operation

```rust
// Add to src/lib.rs
pub struct LatencyMetrics {
    pub last_keypress_time: Option<Instant>,
    pub last_draw_time: Option<Instant>,
    pub network_op_times: Vec<(String, Duration)>,
}

// Log timing for each keypress
AppEvent::KeyPress(key) => {
    let latency = metrics.last_keypress_time.map(|t| t.elapsed());
    log::info!("Keyboard latency: {:?}", latency);
    metrics.last_keypress_time = Some(Instant::now());
}
```

### 2. Create Synthetic Network Delays

Add a debug mode that simulates slow network operations:

```rust
// In fetch_and_process_messages
if std::env::var("NRC_SIMULATE_SLOW_NETWORK").is_ok() {
    tokio::time::sleep(Duration::from_secs(3)).await;
}
```

### 3. Automated Latency Test

Create a test that:
1. Spawns the app with simulated delays
2. Sends synthetic keyboard events
3. Measures time until UI updates
4. Fails if latency > 50ms

```rust
#[test]
fn test_keyboard_latency_under_network_load() {
    // Start app with slow network simulation
    // Send 100 keypresses while network ops are running
    // Assert 95th percentile latency < 50ms
}
```

### 4. Manual Testing Protocol

#### Test Case 1: Rapid Typing During Network Fetch
1. Set `NRC_SIMULATE_SLOW_NETWORK=1`
2. Run the app: `NRC_SIMULATE_SLOW_NETWORK=1 cargo run --release`
3. Type "abcdefghijklmnopqrstuvwxyz" as fast as possible
4. **Expected**: All characters appear instantly
5. **Current**: Characters likely appear in bursts after network operations complete

#### Test Case 2: Measure Actual Latency
1. Add timestamp logging to keyboard events
2. Run: `RUST_LOG=debug cargo run --release 2>&1 | grep "Keyboard latency"`
3. Type steadily for 30 seconds
4. Analyze the latency distribution

#### Test Case 3: Profile CPU Usage
1. Run the app with profiling: `cargo flamegraph --release`
2. Type continuously for 60 seconds
3. Check if keyboard thread is consuming excessive CPU
4. Check if main thread is blocked on network I/O

### 5. Real Fix Verification

The actual fix likely needs:

1. **Move ALL network operations to async tasks**
   ```rust
   AppEvent::FetchMessagesTick => {
       let tx = event_tx.clone();
       let nrc_clone = /* need to share state safely */;
       tokio::spawn(async move {
           let messages = fetch_messages_async().await;
           tx.send(AppEvent::MessagesReceived(messages));
       });
   }
   ```

2. **Never call async functions from the main loop**
   - Main loop should ONLY: receive events, update state, draw UI
   - All I/O should happen in spawned tasks

3. **Use proper async channels**
   - Replace blocking operations with async alternatives
   - Ensure the main loop never waits on network

## Success Criteria

1. **Latency**: 95th percentile keyboard-to-screen latency < 20ms
2. **Consistency**: No variance > 50ms in latency during network operations  
3. **CPU**: Keyboard thread uses < 1% CPU when idle
4. **Throughput**: Can handle 20 keypresses/second without lag

## Implementation Steps

1. Add latency instrumentation to current code
2. Run manual tests to establish baseline metrics
3. Identify specific blocking points with profiling
4. Refactor to true async architecture if needed
5. Re-run tests to verify improvement

## Quick Debug Commands

```bash
# Test with simulated slow network
NRC_SIMULATE_SLOW_NETWORK=1 RUST_LOG=debug cargo run --release 2>&1 | grep -E "(Keyboard|latency)"

# Profile CPU usage
cargo install flamegraph
cargo flamegraph --release

# Trace system calls to find blocking operations
strace -tt -T -f cargo run --release 2>&1 | grep -E "(poll|read|write|epoll)"

# Monitor event loop performance
RUST_LOG=trace cargo run --release 2>&1 | ts '[%Y-%m-%d %H:%M:%.S]' | grep -E "(event|draw|fetch)"
```

## Root Cause Hypothesis

Based on the symptoms, the most likely issue is that `fetch_and_process_messages()` and `fetch_and_process_welcomes()` are still being called directly from the main event loop in a blocking manner. Even though keyboard events are collected separately, they can't be processed until these network operations complete.

The fix is incomplete because we didn't actually move the network operations to background tasks - we just moved the keyboard reading to a background thread, which doesn't solve the fundamental problem of the main loop being blocked.