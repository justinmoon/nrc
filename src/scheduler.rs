use anyhow::Result;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, Instant};
use uuid::Uuid;

use crate::command_executor::{CommandExecutor, CommandResult};
use crate::network_task::NetworkState;
use crate::outbox::{CommandStatus, PersistentCommand};

pub struct BackgroundScheduler {
    executor: CommandExecutor,
    shutdown_rx: Option<mpsc::Receiver<()>>,
    metrics: SchedulerMetrics,
}

#[derive(Debug, Clone)]
pub struct SchedulerMetrics {
    pub commands_processed: u64,
    pub commands_succeeded: u64,
    pub commands_failed: u64,
    pub commands_retried: u64,
    pub last_run_time: Option<Instant>,
    pub processing_duration_ms: u64,
}

impl Default for SchedulerMetrics {
    fn default() -> Self {
        Self {
            commands_processed: 0,
            commands_succeeded: 0,
            commands_failed: 0,
            commands_retried: 0,
            last_run_time: None,
            processing_duration_ms: 0,
        }
    }
}

impl BackgroundScheduler {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            shutdown_rx: None,
            metrics: SchedulerMetrics::default(),
        }
    }

    pub fn with_shutdown_channel(mut self, shutdown_rx: mpsc::Receiver<()>) -> Self {
        self.shutdown_rx = Some(shutdown_rx);
        self
    }

    /// Run the scheduler background task
    /// Processes commands every 5 seconds and handles retries
    pub async fn run(&mut self, state: &mut NetworkState) -> Result<()> {
        let mut ticker = interval(Duration::from_secs(5));
        let mut shutdown_rx = self.shutdown_rx.take();

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    self.process_pending_commands(state).await?;
                }
                _ = async {
                    match &mut shutdown_rx {
                        Some(rx) => rx.recv().await.map(|_| ()),
                        None => std::future::pending().await,
                    }
                } => {
                    log::info!("Background scheduler shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Process a single batch of pending commands
    pub async fn process_pending_commands(&mut self, state: &mut NetworkState) -> Result<()> {
        let start_time = Instant::now();
        self.metrics.last_run_time = Some(start_time);

        let commands = self.executor.get_commands_ready_for_execution()?;
        
        if commands.is_empty() {
            self.metrics.processing_duration_ms = start_time.elapsed().as_millis() as u64;
            return Ok(());
        }

        log::debug!("Processing {} pending commands", commands.len());

        for command in commands {
            // Check if dependencies are satisfied
            if !self.dependencies_satisfied(&command)? {
                log::debug!("Command {} has unsatisfied dependencies, skipping", command.id);
                continue;
            }

            // Check timeout
            if command.metadata.is_timed_out() {
                log::warn!("Command {} timed out, marking as failed", command.id);
                self.handle_command_timeout(command).await?;
                continue;
            }

            // Execute the command
            match self.executor.execute_command(&command, state).await {
                Ok(CommandResult::Success { .. }) => {
                    self.metrics.commands_succeeded += 1;
                    log::debug!("Command {} completed successfully", command.id);
                }
                Ok(CommandResult::Retry { delay_secs, error }) => {
                    self.metrics.commands_retried += 1;
                    log::debug!("Command {} scheduled for retry in {}s: {}", 
                              command.id, delay_secs, error);
                }
                Err(e) => {
                    self.metrics.commands_failed += 1;
                    log::error!("Command {} failed permanently: {}", command.id, e);
                }
            }

            self.metrics.commands_processed += 1;
        }

        // Cleanup old completed tasks periodically
        if self.metrics.commands_processed % 100 == 0 {
            match self.executor.cleanup_old_completed_tasks() {
                Ok(cleaned) if cleaned > 0 => {
                    log::info!("Cleaned up {} old completed tasks", cleaned);
                }
                Err(e) => {
                    log::warn!("Failed to cleanup old tasks: {}", e);
                }
                _ => {}
            }
        }

        self.metrics.processing_duration_ms = start_time.elapsed().as_millis() as u64;
        Ok(())
    }

    /// Check if all dependencies for a command are satisfied (completed successfully)
    fn dependencies_satisfied(&self, command: &PersistentCommand) -> Result<bool> {
        for dep_id in &command.dependencies {
            match self.executor.get_command(dep_id)? {
                Some(dep_command) => {
                    match dep_command.status {
                        CommandStatus::Completed => continue,
                        CommandStatus::Failed => {
                            log::debug!("Command {} has failed dependency {}", command.id, dep_id);
                            return Ok(false);
                        }
                        _ => {
                            log::debug!("Command {} waiting for dependency {} (status: {:?})", 
                                      command.id, dep_id, dep_command.status);
                            return Ok(false);
                        }
                    }
                }
                None => {
                    log::warn!("Command {} has missing dependency {}", command.id, dep_id);
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    async fn handle_command_timeout(&mut self, mut command: PersistentCommand) -> Result<()> {
        command.status = CommandStatus::Failed;
        command.metadata.mark_failed("Command timed out".to_string());
        self.executor.store_command(&command)?;
        
        let event = crate::outbox::TaskEvent::new(
            command.id,
            "timed_out".to_string(),
            None,
        );
        self.executor.add_event(&event)?;
        
        Ok(())
    }

    pub fn get_metrics(&self) -> &SchedulerMetrics {
        &self.metrics
    }

    /// Submit a new command to be processed
    pub fn submit_command(&mut self, command: PersistentCommand) -> Result<Uuid> {
        let id = command.id;
        self.executor.store_command(&command)?;
        
        let event = crate::outbox::TaskEvent::new(
            command.id,
            "submitted".to_string(),
            None,
        );
        self.executor.add_event(&event)?;
        
        log::debug!("Submitted command {} for processing", id);
        Ok(id)
    }

    /// Get the status of a specific command
    pub fn get_command_status(&self, id: &Uuid) -> Result<Option<CommandStatus>> {
        match self.executor.get_command(id)? {
            Some(command) => Ok(Some(command.status)),
            None => Ok(None),
        }
    }

    /// Get all failed commands for debugging
    pub fn get_failed_commands(&self) -> Result<Vec<PersistentCommand>> {
        self.executor.get_failed_commands()
    }

    /// Retry a specific failed command
    pub fn retry_command(&mut self, id: &Uuid) -> Result<()> {
        self.executor.retry_failed_command(id)
    }

    /// Get events for a specific command (useful for debugging)
    pub fn get_command_events(&self, id: &Uuid) -> Result<Vec<crate::outbox::TaskEvent>> {
        self.executor.get_events_for_task(id)
    }
}