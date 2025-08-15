use super::{CleanupConfig, DisconnectTask, worker::CleanupWorker};
use crate::adapter::connection_manager::ConnectionManager;
use crate::app::manager::AppManager;
use crate::channel::manager::ChannelManager;
use crate::webhook::integration::WebhookIntegration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{error, info, warn};

/// Multi-worker cleanup system that distributes work across multiple worker threads
pub struct MultiWorkerCleanupSystem {
    senders: Vec<mpsc::UnboundedSender<DisconnectTask>>,
    worker_handles: Vec<tokio::task::JoinHandle<()>>,
    round_robin_counter: Arc<AtomicUsize>,
    config: CleanupConfig,
}

impl MultiWorkerCleanupSystem {
    pub fn new(
        connection_manager: Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        channel_manager: Arc<RwLock<ChannelManager>>,
        app_manager: Arc<dyn AppManager + Send + Sync>,
        webhook_integration: Option<Arc<WebhookIntegration>>,
        config: CleanupConfig,
    ) -> Self {
        let num_workers = config.worker_threads.resolve();

        info!(
            "Initializing multi-worker cleanup system with {} workers",
            num_workers
        );

        let mut senders = Vec::with_capacity(num_workers);
        let mut worker_handles = Vec::with_capacity(num_workers);

        // Create individual workers with their own channels
        for worker_id in 0..num_workers {
            let (sender, receiver) = mpsc::unbounded_channel();

            // Use per-worker configuration directly (no division across workers)
            let worker_config = config.clone();

            let worker = CleanupWorker::new(
                connection_manager.clone(),
                channel_manager.clone(),
                app_manager.clone(),
                webhook_integration.clone(),
                worker_config.clone(),
            );

            // Spawn worker task
            let handle = tokio::spawn(async move {
                info!("Cleanup worker {} starting", worker_id);
                worker.run(receiver).await;
                info!("Cleanup worker {} stopped", worker_id);
            });

            senders.push(sender);
            worker_handles.push(handle);
        }

        info!(
            "Multi-worker cleanup system initialized with {} workers, batch_size={} per worker",
            num_workers, config.batch_size
        );

        Self {
            senders,
            worker_handles,
            round_robin_counter: Arc::new(AtomicUsize::new(0)),
            config,
        }
    }

    /// Get the main sender for sending tasks - this will distribute work across workers
    pub fn get_sender(&self) -> MultiWorkerSender {
        MultiWorkerSender {
            senders: self.senders.clone(),
            round_robin_counter: self.round_robin_counter.clone(),
        }
    }

    /// Get a direct sender for single worker optimization (avoids wrapper overhead)
    pub fn get_direct_sender(
        &self,
    ) -> Option<tokio::sync::mpsc::UnboundedSender<crate::cleanup::DisconnectTask>> {
        if self.senders.len() == 1 {
            self.senders.get(0).cloned()
        } else {
            None
        }
    }

    /// Get worker handles for shutdown
    pub fn get_worker_handles(self) -> Vec<tokio::task::JoinHandle<()>> {
        self.worker_handles
    }

    /// Get configuration
    pub fn get_config(&self) -> &CleanupConfig {
        &self.config
    }

    /// Shutdown all workers gracefully
    pub async fn shutdown(self) -> Result<(), String> {
        info!("Shutting down multi-worker cleanup system...");

        // Drop all senders to signal workers to finish processing and exit
        drop(self.senders);

        // Wait for all workers to complete
        let mut shutdown_errors = Vec::new();
        for (i, handle) in self.worker_handles.into_iter().enumerate() {
            if let Err(e) = handle.await {
                let error_msg = format!("Worker {} shutdown error: {}", i, e);
                error!("{}", error_msg);
                shutdown_errors.push(error_msg);
            }
        }

        if shutdown_errors.is_empty() {
            info!("Multi-worker cleanup system shutdown complete");
            Ok(())
        } else {
            Err(format!(
                "Shutdown completed with {} errors: {:?}",
                shutdown_errors.len(),
                shutdown_errors
            ))
        }
    }
}

/// Sender wrapper that distributes tasks across multiple worker threads
pub struct MultiWorkerSender {
    senders: Vec<mpsc::UnboundedSender<DisconnectTask>>,
    round_robin_counter: Arc<AtomicUsize>,
}

impl MultiWorkerSender {
    /// Send a disconnect task to the next worker in round-robin fashion
    pub fn send(&self, task: DisconnectTask) -> Result<(), mpsc::error::SendError<DisconnectTask>> {
        if self.senders.is_empty() {
            return Err(mpsc::error::SendError(task));
        }

        // Use round-robin distribution
        let worker_index =
            self.round_robin_counter.fetch_add(1, Ordering::Relaxed) % self.senders.len();

        match self.senders[worker_index].send(task) {
            Ok(()) => Ok(()),
            Err(send_error) => {
                // If the selected worker's channel is closed, try the next available one
                warn!("Worker {} channel closed, trying next worker", worker_index);

                // Try all other workers in sequence
                for offset in 1..self.senders.len() {
                    let next_index = (worker_index + offset) % self.senders.len();
                    if let Ok(()) = self.senders[next_index].send(send_error.0.clone()) {
                        return Ok(());
                    }
                }

                // All workers are unavailable
                error!("All cleanup workers are unavailable");
                Err(send_error)
            }
        }
    }

    /// Send a disconnect task with fallback to other workers if one fails
    /// This is essentially the same as send() but with explicit error handling
    pub fn send_with_fallback(
        &self,
        task: DisconnectTask,
    ) -> Result<(), mpsc::error::SendError<DisconnectTask>> {
        // This is the same implementation as send() - UnboundedSender doesn't block
        self.send(task)
    }

    /// Check if any worker is available
    pub fn is_available(&self) -> bool {
        self.senders.iter().any(|sender| !sender.is_closed())
    }

    /// Get the number of workers
    pub fn worker_count(&self) -> usize {
        self.senders.len()
    }

    /// Get statistics about worker availability
    pub fn get_worker_stats(&self) -> WorkerStats {
        let total = self.senders.len();
        let available = self
            .senders
            .iter()
            .filter(|sender| !sender.is_closed())
            .count();
        let closed = total - available;

        WorkerStats {
            total_workers: total,
            available_workers: available,
            closed_workers: closed,
        }
    }
}

/// Statistics about worker availability
#[derive(Debug, Clone)]
pub struct WorkerStats {
    pub total_workers: usize,
    pub available_workers: usize,
    pub closed_workers: usize,
}

impl Clone for MultiWorkerSender {
    fn clone(&self) -> Self {
        Self {
            senders: self.senders.clone(),
            round_robin_counter: self.round_robin_counter.clone(),
        }
    }
}

// Make the sender safe to send across threads
unsafe impl Send for MultiWorkerSender {}
unsafe impl Sync for MultiWorkerSender {}
