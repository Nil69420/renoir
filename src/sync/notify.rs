//! Event notification system using eventfd and condition-based wakeups
//!
//! This module provides efficient notification primitives optimized for robotics
//! scenarios where writers need to notify multiple readers about data availability.
//! Uses eventfd on Linux for maximum efficiency.

use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};
use std::collections::HashMap;
use std::os::fd::{RawFd, AsRawFd};
use std::sync::Mutex;

use nix::{
    sys::eventfd::{eventfd, EfdFlags},
    unistd::{write, read},
    poll::{poll, PollFd, PollFlags},
    errno::Errno,
};

use super::{SyncResult, SyncError};

/// Condition for triggering notifications
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyCondition {
    /// Notify on every write
    Always,
    /// Notify when buffer reaches certain fill level
    BufferLevel(usize),
    /// Notify after certain number of writes
    WriteCount(u64),
    /// Notify after time interval (in messages)
    TimeInterval(u64),
    /// Custom condition based on data content
    Custom,
}

/// Event-based notifier using eventfd (Linux) or alternatives
#[derive(Debug)]
pub struct EventNotifier {
    /// Event file descriptor for Linux eventfd notifications
    #[cfg(target_os = "linux")]
    event_fd: Option<std::os::fd::OwnedFd>,
    /// Fallback condition variable for non-Linux systems
    #[cfg(not(target_os = "linux"))]
    condvar: Arc<(Mutex<bool>, std::sync::Condvar)>,
    /// Whether notifications are enabled
    enabled: AtomicBool,
    /// Statistics
    notify_count: AtomicU64,
    wait_count: AtomicU64,
}

impl EventNotifier {
    /// Create a new event notifier
    pub fn new() -> SyncResult<Self> {
        #[cfg(target_os = "linux")]
        let event_fd = Self::create_eventfd()?;
        
        #[cfg(not(target_os = "linux"))]
        let condvar = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        
        Ok(Self {
            #[cfg(target_os = "linux")]
            event_fd,
            #[cfg(not(target_os = "linux"))]
            condvar,
            enabled: AtomicBool::new(true),
            notify_count: AtomicU64::new(0),
            wait_count: AtomicU64::new(0),
        })
    }
    
    #[cfg(target_os = "linux")]
    fn create_eventfd() -> SyncResult<Option<std::os::fd::OwnedFd>> {
        let fd = eventfd(0, EfdFlags::EFD_CLOEXEC | EfdFlags::EFD_NONBLOCK)
            .map_err(|_| SyncError::NotificationFailed)?;
        Ok(Some(fd))
    }
    
    /// Notify waiting readers
    pub fn notify(&self) -> SyncResult<()> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }
        
        self.notify_count.fetch_add(1, Ordering::Relaxed);
        
        #[cfg(target_os = "linux")]
        {
            if let Some(ref owned_fd) = self.event_fd {
                let fd = owned_fd.as_raw_fd();
                let value: u64 = 1;
                let buf = value.to_ne_bytes();
                match write(fd, &buf) {
                    Ok(_) => {},
                    Err(Errno::EAGAIN) => {}, // OK for non-blocking eventfd
                    Err(_) => return Err(SyncError::NotificationFailed),
                }
            }
        }
        
        #[cfg(not(target_os = "linux"))]
        {
            let (mutex, condvar) = &*self.condvar;
            let mut notified = mutex.lock().unwrap();
            *notified = true;
            condvar.notify_all();
        }
        
        Ok(())
    }
    
    /// Wait for notification with optional timeout
    pub fn wait(&self, timeout_ms: Option<u64>) -> SyncResult<()> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Err(SyncError::NotificationFailed);
        }
        
        self.wait_count.fetch_add(1, Ordering::Relaxed);
        
        #[cfg(target_os = "linux")]
        {
            if let Some(ref owned_fd) = self.event_fd {
                let fd = owned_fd.as_raw_fd();
                return self.wait_on_eventfd(fd, timeout_ms);
            }
        }
        
        #[cfg(not(target_os = "linux"))]
        {
            let (mutex, condvar) = &*self.condvar;
            let mut notified = mutex.lock().unwrap();
            
            if let Some(timeout) = timeout_ms {
                let timeout_duration = Duration::from_millis(timeout);
                let (new_notified, _timeout_result) = condvar
                    .wait_timeout_while(notified, timeout_duration, |&mut n| !n)
                    .unwrap();
                notified = new_notified;
            } else {
                notified = condvar.wait_while(notified, |&mut n| !n).unwrap();
            }
            
            if *notified {
                *notified = false; // Reset for next wait
                Ok(())
            } else {
                Err(SyncError::NotificationFailed) // Timeout
            }
        }
        
        #[cfg(target_os = "linux")]
        Err(SyncError::NotificationFailed)
    }
    
    #[cfg(target_os = "linux")]
    fn wait_on_eventfd(&self, fd: RawFd, timeout_ms: Option<u64>) -> SyncResult<()> {
        use std::os::fd::BorrowedFd;
        let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };
        let mut fds = [PollFd::new(&borrowed_fd, PollFlags::POLLIN)];
        
        let timeout = timeout_ms.map(|ms| ms as i32).unwrap_or(-1);
        
        match poll(&mut fds, timeout) {
            Ok(0) => Err(SyncError::NotificationFailed), // Timeout
            Ok(_) => {
                // Clear the eventfd by reading from it
                let mut buf = [0u8; 8];
                let _ = read(fd, &mut buf); // Ignore result, just clearing
                Ok(())
            }
            Err(_) => Err(SyncError::NotificationFailed),
        }
    }
    
    /// Get the file descriptor for external polling (Linux only)
    #[cfg(target_os = "linux")]
    pub fn event_fd(&self) -> Option<RawFd> {
        self.event_fd.as_ref().map(|fd| fd.as_raw_fd())
    }
    
    /// Enable or disable notifications
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }
    
    /// Check if notifications are enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
    
    /// Get notification statistics
    pub fn stats(&self) -> NotificationStats {
        NotificationStats {
            notify_count: self.notify_count.load(Ordering::Relaxed),
            wait_count: self.wait_count.load(Ordering::Relaxed),
            enabled: self.is_enabled(),
        }
    }
}

impl Drop for EventNotifier {
    fn drop(&mut self) {
        // OwnedFd automatically closes the file descriptor when dropped
        // No manual cleanup needed
    }
}

/// Statistics for event notifications
#[derive(Debug, Clone)]
pub struct NotificationStats {
    /// Number of notifications sent
    pub notify_count: u64,
    /// Number of waits performed
    pub wait_count: u64,
    /// Whether notifications are currently enabled
    pub enabled: bool,
}

/// Group of notifiers for managing multiple notification channels
#[derive(Debug)]
pub struct NotificationGroup {
    /// Map of channel names to notifiers
    notifiers: Mutex<HashMap<String, Arc<EventNotifier>>>,
    /// Default notifier for convenience
    default_notifier: Arc<EventNotifier>,
}

impl NotificationGroup {
    /// Create a new notification group
    pub fn new() -> SyncResult<Self> {
        Ok(Self {
            notifiers: Mutex::new(HashMap::new()),
            default_notifier: Arc::new(EventNotifier::new()?),
        })
    }
    
    /// Add a named notifier channel
    pub fn add_channel(&self, name: &str) -> SyncResult<Arc<EventNotifier>> {
        let notifier = Arc::new(EventNotifier::new()?);
        let mut notifiers = self.notifiers.lock().unwrap();
        notifiers.insert(name.to_string(), notifier.clone());
        Ok(notifier)
    }
    
    /// Get a notifier by channel name
    pub fn get_channel(&self, name: &str) -> Option<Arc<EventNotifier>> {
        let notifiers = self.notifiers.lock().unwrap();
        notifiers.get(name).cloned()
    }
    
    /// Get the default notifier
    pub fn default(&self) -> Arc<EventNotifier> {
        self.default_notifier.clone()
    }
    
    /// Notify all channels
    pub fn notify_all(&self) -> SyncResult<()> {
        // Notify default
        self.default_notifier.notify()?;
        
        // Notify all named channels
        let notifiers = self.notifiers.lock().unwrap();
        for notifier in notifiers.values() {
            notifier.notify()?;
        }
        
        Ok(())
    }
    
    /// Notify specific channels
    pub fn notify_channels(&self, channel_names: &[&str]) -> SyncResult<()> {
        let notifiers = self.notifiers.lock().unwrap();
        for name in channel_names {
            if let Some(notifier) = notifiers.get(*name) {
                notifier.notify()?;
            }
        }
        Ok(())
    }
    
    /// Remove a channel
    pub fn remove_channel(&self, name: &str) -> bool {
        let mut notifiers = self.notifiers.lock().unwrap();
        notifiers.remove(name).is_some()
    }
    
    /// Get list of all channel names
    pub fn channel_names(&self) -> Vec<String> {
        let notifiers = self.notifiers.lock().unwrap();
        notifiers.keys().cloned().collect()
    }
    
    /// Get aggregated statistics for all channels
    pub fn aggregate_stats(&self) -> AggregateNotificationStats {
        let mut total_notifies = 0;
        let mut total_waits = 0;
        let mut enabled_channels = 0;
        let mut total_channels = 1; // Include default
        
        // Add default stats
        let default_stats = self.default_notifier.stats();
        total_notifies += default_stats.notify_count;
        total_waits += default_stats.wait_count;
        if default_stats.enabled {
            enabled_channels += 1;
        }
        
        // Add named channel stats
        let notifiers = self.notifiers.lock().unwrap();
        total_channels += notifiers.len();
        
        for notifier in notifiers.values() {
            let stats = notifier.stats();
            total_notifies += stats.notify_count;
            total_waits += stats.wait_count;
            if stats.enabled {
                enabled_channels += 1;
            }
        }
        
        AggregateNotificationStats {
            total_notify_count: total_notifies,
            total_wait_count: total_waits,
            total_channels,
            enabled_channels,
        }
    }
}

impl Default for NotificationGroup {
    fn default() -> Self {
        Self::new().expect("Failed to create default notification group")
    }
}

/// Aggregate statistics across all notification channels
#[derive(Debug, Clone)]
pub struct AggregateNotificationStats {
    /// Total notifications across all channels
    pub total_notify_count: u64,
    /// Total waits across all channels
    pub total_wait_count: u64,
    /// Number of total channels
    pub total_channels: usize,
    /// Number of enabled channels
    pub enabled_channels: usize,
}

/// Batched notification manager for high-frequency scenarios
#[derive(Debug)]
pub struct BatchNotifier {
    /// Underlying notifier
    notifier: Arc<EventNotifier>,
    /// Batch size threshold
    batch_size: AtomicU64,
    /// Current batch count
    current_batch: AtomicU64,
    /// Condition for triggering notification
    condition: NotifyCondition,
    /// Write count for condition checking
    write_count: AtomicU64,
}

impl BatchNotifier {
    /// Create a new batch notifier
    pub fn new(condition: NotifyCondition) -> SyncResult<Self> {
        let batch_size = match condition {
            NotifyCondition::WriteCount(n) => n,
            NotifyCondition::BufferLevel(n) => n as u64,
            NotifyCondition::TimeInterval(n) => n,
            _ => 1,
        };
        
        Ok(Self {
            notifier: Arc::new(EventNotifier::new()?),
            batch_size: AtomicU64::new(batch_size),
            current_batch: AtomicU64::new(0),
            condition,
            write_count: AtomicU64::new(0),
        })
    }
    
    /// Record a write and maybe trigger notification
    pub fn record_write(&self, buffer_level: Option<usize>) -> SyncResult<bool> {
        let write_count = self.write_count.fetch_add(1, Ordering::Relaxed) + 1;
        
        let should_notify = match self.condition {
            NotifyCondition::Always => true,
            NotifyCondition::WriteCount(n) => write_count % n == 0,
            NotifyCondition::BufferLevel(threshold) => {
                buffer_level.map(|level| level >= threshold).unwrap_or(false)
            }
            NotifyCondition::TimeInterval(n) => {
                let batch = self.current_batch.fetch_add(1, Ordering::Relaxed) + 1;
                if batch >= n {
                    self.current_batch.store(0, Ordering::Relaxed);
                    true
                } else {
                    false
                }
            }
            NotifyCondition::Custom => false, // Let caller decide
        };
        
        if should_notify {
            self.notifier.notify()?;
        }
        
        Ok(should_notify)
    }
    
    /// Force a notification regardless of condition
    pub fn force_notify(&self) -> SyncResult<()> {
        self.notifier.notify()
    }
    
    /// Wait for notification
    pub fn wait(&self, timeout_ms: Option<u64>) -> SyncResult<()> {
        self.notifier.wait(timeout_ms)
    }
    
    /// Get the underlying notifier for direct access
    pub fn notifier(&self) -> Arc<EventNotifier> {
        self.notifier.clone()
    }
    
    /// Update the batch condition
    pub fn set_condition(&mut self, condition: NotifyCondition) {
        self.condition = condition;
        let new_batch_size = match condition {
            NotifyCondition::WriteCount(n) => n,
            NotifyCondition::BufferLevel(n) => n as u64,
            NotifyCondition::TimeInterval(n) => n,
            _ => 1,
        };
        self.batch_size.store(new_batch_size, Ordering::Relaxed);
    }
    
    /// Get current batch statistics
    pub fn batch_stats(&self) -> BatchStats {
        BatchStats {
            condition: self.condition,
            batch_size: self.batch_size.load(Ordering::Relaxed),
            current_batch: self.current_batch.load(Ordering::Relaxed),
            total_writes: self.write_count.load(Ordering::Relaxed),
            notification_stats: self.notifier.stats(),
        }
    }
}

/// Statistics for batch notification
#[derive(Debug, Clone)]
pub struct BatchStats {
    /// Current notification condition
    pub condition: NotifyCondition,
    /// Batch size threshold
    pub batch_size: u64,
    /// Current batch count
    pub current_batch: u64,
    /// Total writes recorded
    pub total_writes: u64,
    /// Underlying notification stats
    pub notification_stats: NotificationStats,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::sync::{Arc, Barrier};
    use std::time::Duration;
    
    #[test]
    fn test_event_notifier_basic() {
        let notifier = EventNotifier::new().unwrap();
        
        // Should start enabled
        assert!(notifier.is_enabled());
        
        // Basic notify should not fail
        notifier.notify().unwrap();
        
        let stats = notifier.stats();
        assert_eq!(stats.notify_count, 1);
    }
    
    #[test]
    fn test_notifier_enable_disable() {
        let notifier = EventNotifier::new().unwrap();
        
        notifier.set_enabled(false);
        assert!(!notifier.is_enabled());
        
        // Notify should still succeed but do nothing
        notifier.notify().unwrap();
        
        notifier.set_enabled(true);
        assert!(notifier.is_enabled());
    }
    
    #[test]
    fn test_notification_group() {
        let group = NotificationGroup::new().unwrap();
        
        // Add some channels
        let _channel1 = group.add_channel("sensor1").unwrap();
        let _channel2 = group.add_channel("sensor2").unwrap();
        
        let names = group.channel_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"sensor1".to_string()));
        assert!(names.contains(&"sensor2".to_string()));
        
        // Notify all should not fail
        group.notify_all().unwrap();
        
        // Notify specific channels
        group.notify_channels(&["sensor1"]).unwrap();
    }
    
    #[test]
    fn test_batch_notifier() {
        let mut batch_notifier = BatchNotifier::new(NotifyCondition::WriteCount(3)).unwrap();
        
        // First two writes should not trigger notification
        assert!(!batch_notifier.record_write(None).unwrap());
        assert!(!batch_notifier.record_write(None).unwrap());
        
        // Third write should trigger
        assert!(batch_notifier.record_write(None).unwrap());
        
        // Reset and test buffer level condition
        batch_notifier.set_condition(NotifyCondition::BufferLevel(5));
        
        assert!(!batch_notifier.record_write(Some(3)).unwrap());
        assert!(batch_notifier.record_write(Some(6)).unwrap());
    }
    
    #[test]
    fn test_concurrent_notifications() {
        let notifier = Arc::new(EventNotifier::new().unwrap());
        let barrier = Arc::new(Barrier::new(3)); // 2 threads + main
        
        let notify_handles: Vec<_> = (0..2).map(|_| {
            let notifier = notifier.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                for _ in 0..10 {
                    notifier.notify().unwrap();
                    thread::sleep(Duration::from_millis(1));
                }
            })
        }).collect();
        
        barrier.wait();
        
        // Let them run for a bit
        thread::sleep(Duration::from_millis(50));
        
        for handle in notify_handles {
            handle.join().unwrap();
        }
        
        let stats = notifier.stats();
        assert_eq!(stats.notify_count, 20); // 2 threads * 10 notifications each
    }
    
    #[cfg(target_os = "linux")]
    #[test]
    fn test_eventfd_wait() {
        let notifier = Arc::new(EventNotifier::new().unwrap());
        let notifier_clone = notifier.clone();
        
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            notifier_clone.notify().unwrap();
        });
        
        // This should succeed when the other thread notifies
        let result = notifier.wait(Some(1000));
        assert!(result.is_ok());
        
        handle.join().unwrap();
    }
    
    #[test]
    fn test_batch_time_interval() {
        let batch_notifier = BatchNotifier::new(NotifyCondition::TimeInterval(3)).unwrap();
        
        // First two should not notify
        assert!(!batch_notifier.record_write(None).unwrap());
        assert!(!batch_notifier.record_write(None).unwrap());
        
        // Third should notify and reset
        assert!(batch_notifier.record_write(None).unwrap());
        
        // Should start over
        assert!(!batch_notifier.record_write(None).unwrap());
    }
}