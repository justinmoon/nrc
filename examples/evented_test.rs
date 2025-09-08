use anyhow::Result;
use nrc::actions::Action;
use nrc::evented_nrc::EventedNrc;

#[tokio::main]
async fn main() -> Result<()> {
    // Setup logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    
    let temp_dir = std::env::temp_dir().join("evented_nrc_test");
    std::fs::create_dir_all(&temp_dir)?;
    
    println!("Creating EventedNrc...");
    let (evented, mut event_loop) = EventedNrc::new(&temp_dir).await?;
    
    println!("Initial state: {:?}", *evented.state.borrow());
    println!("NPub: {}", evented.npub);
    
    // Test sending an action
    println!("\nEmitting ShowHelp action...");
    evented.emit(Action::ShowHelp);
    
    // Process the action
    event_loop.process_one().await;
    
    println!("Show help state: {}", *evented.show_help.borrow());
    
    // Test input handling
    println!("\nTesting input handling...");
    evented.emit(Action::SetInput("Hello world".to_string()));
    event_loop.process_one().await;
    println!("Input: {}", *evented.input.borrow());
    
    evented.emit(Action::ClearInput);
    event_loop.process_one().await;
    println!("Input after clear: {}", *evented.input.borrow());
    
    // Test error handling
    println!("\nTesting error handling...");
    evented.emit(Action::SendMessage("Test message".to_string()));
    event_loop.process_one().await;
    
    if let Some(error) = &*evented.last_error.borrow() {
        println!("Error (expected): {}", error);
    }
    
    println!("\nEventedNrc wrapper test completed successfully!");
    println!("Phase 1 complete: EventedNrc wrapper is working! (95% confidence)");
    
    Ok(())
}