//! File locking for daemon startup
//!
//! Prevents race condition where multiple clients try to start daemon simultaneously
//!
//! Strategy:
//! 1. Acquire exclusive lock on .daemon.lock file
//! 2. Double-check daemon doesn't exist (might have started while waiting for lock)
//! 3. Start daemon if needed
//! 4. Release lock

use anyhow::{Context, Result};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::fs::File;

/// File lock guard
pub struct DaemonLock {
    _file: File,
    lock_path: std::path::PathBuf,
}

impl DaemonLock {
    /// Try to acquire daemon lock
    ///
    /// Waits up to `timeout` for lock to become available.
    /// If timeout expires, assumes lock is stale and forces break.
    pub async fn acquire(daemon_dir: &Path, timeout: Duration) -> Result<Self> {
        let lock_path = daemon_dir.join(".daemon.lock");

        eprintln!("[daemon] Acquiring daemon lock: {}", lock_path.display());

        // Try to acquire lock with timeout
        let file = acquire_lock_with_timeout(&lock_path, timeout).await?;

        eprintln!("[daemon] Lock acquired");

        Ok(Self {
            _file: file,
            lock_path,
        })
    }

    /// Release the lock (happens automatically on drop, but can be explicit)
    pub async fn release(self) -> Result<()> {
        // Remove lock file
        if let Err(e) = tokio::fs::remove_file(&self.lock_path).await {
            eprintln!("[daemon] Warning: Failed to remove lock file: {}", e);
        }

        // File handle will be dropped automatically, releasing the lock
        Ok(())
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        // Lock file will be removed by file handle close
        // Best effort cleanup
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Acquire exclusive lock with timeout
async fn acquire_lock_with_timeout(lock_path: &Path, timeout: Duration) -> Result<File> {
    let start = Instant::now();

    loop {
        match try_acquire_lock(lock_path).await {
            Ok(file) => return Ok(file),

            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                // Lock held by another process (AlreadyExists from O_CREAT|O_EXCL)

                if start.elapsed() > timeout {
                    // Timeout - assume stale lock
                    eprintln!(
                        "[daemon] Warning: Lock timeout after {:?} - assuming stale lock, force breaking",
                        timeout
                    );

                    // Force break lock (dangerous but necessary)
                    force_break_lock(lock_path).await?;

                    // Try one more time
                    return try_acquire_lock(lock_path).await.with_context(|| {
                        format!(
                            "Failed to acquire lock after breaking: {}",
                            lock_path.display()
                        )
                    });
                }

                // Wait a bit and retry
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            Err(e) => {
                return Err(e).context("Failed to acquire lock");
            }
        }
    }
}

/// Try to acquire exclusive lock (non-blocking)
async fn try_acquire_lock(lock_path: &Path) -> std::io::Result<File> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;

    // Create parent directory if needed
    if let Some(parent) = lock_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Use O_CREAT | O_EXCL for atomic creation
    // This fails if file already exists
    let std_file = OpenOptions::new()
        .write(true)
        .create_new(true) // Atomic - fails if exists
        .mode(0o600) // Owner only
        .open(lock_path)?;

    // Convert to tokio File
    let file = File::from_std(std_file);

    Ok(file)
}

/// Force break a stale lock
async fn force_break_lock(lock_path: &Path) -> Result<()> {
    eprintln!(
        "[daemon] Warning: Force breaking lock: {}",
        lock_path.display()
    );

    // Remove the lock file
    if let Err(e) = tokio::fs::remove_file(lock_path).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(e).context("Failed to remove stale lock file");
        }
    }

    Ok(())
}

/// Alternative: Use flock-based locking (may be more robust on some systems)
#[cfg(unix)]
pub mod flock_based {
    use super::*;
    use std::fs::File as StdFile;
    use std::os::unix::io::AsRawFd;

    pub struct FlockLock {
        _file: StdFile,
        lock_path: std::path::PathBuf,
    }

    impl FlockLock {
        pub async fn acquire(daemon_dir: &Path, timeout: Duration) -> Result<Self> {
            let lock_path = daemon_dir.join(".daemon.lock");

            // Create lock file
            tokio::fs::create_dir_all(daemon_dir).await?;

            let file = StdFile::create(&lock_path)?;
            let fd = file.as_raw_fd();

            let start = Instant::now();

            loop {
                // Try non-blocking lock
                let ret = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };

                if ret == 0 {
                    // Lock acquired
                    return Ok(Self {
                        _file: file,
                        lock_path,
                    });
                }

                // Check for timeout
                if start.elapsed() > timeout {
                    eprintln!("[daemon] Warning: flock timeout - assuming stale lock");

                    // Try blocking lock with short timeout
                    // (If lock is truly stale, this will succeed quickly)
                    let ret = unsafe { libc::flock(fd, libc::LOCK_EX) };

                    if ret == 0 {
                        return Ok(Self {
                            _file: file,
                            lock_path,
                        });
                    } else {
                        anyhow::bail!("Failed to acquire flock");
                    }
                }

                // Wait and retry
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    impl Drop for FlockLock {
        fn drop(&mut self) {
            // flock is automatically released when file closes
            let _ = std::fs::remove_file(&self.lock_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_acquire_release() {
        let temp_dir = std::env::temp_dir().join("ovim-lock-test");
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let lock = DaemonLock::acquire(&temp_dir, Duration::from_secs(5))
            .await
            .unwrap();

        // Lock should exist
        assert!(temp_dir.join(".daemon.lock").exists());

        // Release
        lock.release().await.unwrap();

        // Lock should be gone
        assert!(!temp_dir.join(".daemon.lock").exists());

        // Cleanup
        tokio::fs::remove_dir_all(&temp_dir).await.unwrap();
    }

    #[tokio::test]
    async fn test_concurrent_lock() {
        let temp_dir = std::env::temp_dir().join("ovim-lock-test-concurrent");
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let temp_dir = Arc::new(temp_dir);

        // Start 10 concurrent lock attempts
        let mut tasks = vec![];

        for i in 0..10 {
            let dir = temp_dir.clone();
            tasks.push(tokio::spawn(async move {
                let lock = DaemonLock::acquire(&dir, Duration::from_secs(10))
                    .await
                    .unwrap();

                // Hold lock briefly
                tokio::time::sleep(Duration::from_millis(10)).await;

                drop(lock);

                i
            }));
        }

        // All should complete successfully (one at a time)
        for task in tasks {
            let result = task.await;
            assert!(result.is_ok());
        }

        // Cleanup
        tokio::fs::remove_dir_all(temp_dir.as_ref()).await.unwrap();
    }
}
