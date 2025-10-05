//! Task supervisor for managing background tasks with automatic restart
//!
//! Tracks all spawned tasks and can:
//! - Monitor task health
//! - Restart failed tasks automatically
//! - Shutdown all tasks gracefully
//! - Prevent resource leaks from forgotten tasks

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// Task restart policy
#[derive(Debug, Clone)]
pub enum RestartPolicy {
    /// Never restart tasks
    Never,

    /// Always restart, even on successful completion
    Always {
        max_retries: u32,
        initial_backoff: Duration,
    },

    /// Only restart on failure
    OnFailure {
        max_retries: u32,
        initial_backoff: Duration,
    },
}

impl Default for RestartPolicy {
    fn default() -> Self {
        RestartPolicy::OnFailure {
            max_retries: 3,
            initial_backoff: Duration::from_secs(1),
        }
    }
}

/// Status of a supervised task
#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Running,
    Completed,
    Failed(String),
    Restarting,
    MaxRetriesExceeded,
}

/// Metadata about a supervised task
pub struct TaskHandle {
    /// Task name for identification
    name: String,

    /// Join handle for the task
    handle: JoinHandle<()>,

    /// When the task was started
    started_at: Instant,

    /// How many times this task has been restarted
    restarts: u32,

    /// Current status
    status: TaskStatus,
}

/// Supervises background tasks with automatic restart
pub struct TaskSupervisor {
    /// Active tasks being supervised
    tasks: Arc<Mutex<HashMap<String, TaskHandle>>>,

    /// Default restart policy for new tasks
    default_policy: RestartPolicy,
}

impl TaskSupervisor {
    /// Creates a new task supervisor with default restart policy
    pub fn new(policy: RestartPolicy) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            default_policy: policy,
        }
    }

    /// Spawns a supervised task that will be automatically restarted on failure
    ///
    /// The factory function is called each time the task needs to start/restart.
    /// This allows stateless restart - the factory creates a fresh future each time.
    pub async fn spawn_supervised<F, Fut>(
        &self,
        name: String,
        factory: F,
    ) -> Result<()>
    where
        F: Fn() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.spawn_supervised_with_policy(name, factory, self.default_policy.clone())
            .await
    }

    /// Spawns a supervised task with a specific restart policy
    pub async fn spawn_supervised_with_policy<F, Fut>(
        &self,
        name: String,
        factory: F,
        policy: RestartPolicy,
    ) -> Result<()>
    where
        F: Fn() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        let tasks = self.tasks.clone();
        let name_clone = name.clone();

        // Wrap the factory in restart logic
        let handle = tokio::spawn(async move {
            let mut restarts = 0u32;

            loop {
                let start = Instant::now();

                // Run the task
                let result = factory().await;
                let uptime = start.elapsed();

                match result {
                    Ok(()) => {
                        // Check if we should restart on success
                        match policy {
                            RestartPolicy::Always { .. } => {
                                // Restart on success
                            }
                            _ => {
                                break; // Normal completion, don't restart
                            }
                        }
                    }
                    Err(e) => {
                        // Only log actual errors
                        eprintln!(
                            "[Supervisor] Task '{}' failed after {:?}: {}",
                            name_clone, uptime, e
                        );

                        // Check restart policy
                        let (max_retries, initial_backoff) = match policy {
                            RestartPolicy::Never => {
                                break;
                            }
                            RestartPolicy::Always { max_retries, initial_backoff } |
                            RestartPolicy::OnFailure { max_retries, initial_backoff } => {
                                (max_retries, initial_backoff)
                            }
                        };

                        // Check retry limit
                        if restarts >= max_retries {
                            eprintln!(
                                "[Supervisor] Task '{}' exceeded max retries ({}/{})",
                                name_clone, restarts, max_retries
                            );

                            // Update status to MaxRetriesExceeded
                            let mut tasks_lock = tasks.lock().await;
                            if let Some(task) = tasks_lock.get_mut(&name_clone) {
                                task.status = TaskStatus::MaxRetriesExceeded;
                            }
                            break;
                        }

                        // Calculate exponential backoff
                        restarts += 1;
                        let backoff = initial_backoff * restarts;

                        // Update status to Restarting
                        {
                            let mut tasks_lock = tasks.lock().await;
                            if let Some(task) = tasks_lock.get_mut(&name_clone) {
                                task.status = TaskStatus::Restarting;
                                task.restarts = restarts;
                            }
                        }

                        tokio::time::sleep(backoff).await;
                    }
                }

                // Update status back to Running before restart
                {
                    let mut tasks_lock = tasks.lock().await;
                    if let Some(task) = tasks_lock.get_mut(&name_clone) {
                        task.status = TaskStatus::Running;
                        task.started_at = Instant::now();
                    }
                }
            }
        });

        // Register the task
        let mut tasks = self.tasks.lock().await;
        tasks.insert(
            name.clone(),
            TaskHandle {
                name: name.clone(),
                handle,
                started_at: Instant::now(),
                restarts: 0,
                status: TaskStatus::Running,
            },
        );

        Ok(())
    }

    /// Gets the status of a specific task
    pub async fn task_status(&self, name: &str) -> Option<TaskStatus> {
        let tasks = self.tasks.lock().await;
        tasks.get(name).map(|t| t.status.clone())
    }

    /// Gets health information for all tasks
    pub async fn health_check(&self) -> Vec<TaskHealth> {
        let tasks = self.tasks.lock().await;
        tasks
            .values()
            .map(|task| TaskHealth {
                name: task.name.clone(),
                status: task.status.clone(),
                uptime: task.started_at.elapsed(),
                restarts: task.restarts,
                is_alive: !task.handle.is_finished(),
            })
            .collect()
    }

    /// Counts tasks by status
    pub async fn count_by_status(&self) -> HashMap<String, usize> {
        let health = self.health_check().await;
        let mut counts = HashMap::new();

        for task in health {
            let status_str = match task.status {
                TaskStatus::Running => "running",
                TaskStatus::Completed => "completed",
                TaskStatus::Failed(_) => "failed",
                TaskStatus::Restarting => "restarting",
                TaskStatus::MaxRetriesExceeded => "max_retries",
            };
            *counts.entry(status_str.to_string()).or_insert(0) += 1;
        }

        counts
    }

    /// Shuts down all supervised tasks gracefully
    pub async fn shutdown_all(&self) -> Result<()> {
        let mut tasks = self.tasks.lock().await;

        for (_name, task) in tasks.drain() {
            task.handle.abort();
        }

        Ok(())
    }

    /// Removes completed or failed tasks from tracking
    pub async fn cleanup_finished(&self) -> usize {
        let mut tasks = self.tasks.lock().await;
        let before = tasks.len();

        tasks.retain(|_name, task| {
            !task.handle.is_finished()
        });

        before - tasks.len()
    }
}

/// Health information for a supervised task
#[derive(Debug, Clone)]
pub struct TaskHealth {
    pub name: String,
    pub status: TaskStatus,
    pub uptime: Duration,
    pub restarts: u32,
    pub is_alive: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_supervisor_basic() {
        let supervisor = TaskSupervisor::new(RestartPolicy::Never);

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        supervisor
            .spawn_supervised("test_task".to_string(), move || {
                let counter = counter_clone.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_supervisor_restart_on_failure() {
        let supervisor = TaskSupervisor::new(RestartPolicy::OnFailure {
            max_retries: 2,
            initial_backoff: Duration::from_millis(10),
        });

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        supervisor
            .spawn_supervised("failing_task".to_string(), move || {
                let counter = counter_clone.clone();
                async move {
                    let count = counter.fetch_add(1, Ordering::SeqCst);
                    if count < 2 {
                        Err(anyhow!("Simulated failure"))
                    } else {
                        Ok(())
                    }
                }
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;

        // Should run 3 times total: initial + 2 retries
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_supervisor_health_check() {
        let supervisor = TaskSupervisor::new(RestartPolicy::Never);

        supervisor
            .spawn_supervised("health_task".to_string(), || async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok(())
            })
            .await
            .unwrap();

        let health = supervisor.health_check().await;
        assert_eq!(health.len(), 1);
        assert_eq!(health[0].name, "health_task");
        assert_eq!(health[0].status, TaskStatus::Running);
        assert!(health[0].is_alive);
    }
}
