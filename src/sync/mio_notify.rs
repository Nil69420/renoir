//! Mio-based asynchronous event notification system
//!
//! This module provides async event handling using mio's cross-platform event loop.
//! It replaces manual poll() calls with mio's efficient event polling system.

use std::{
    sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}, Mutex},
    collections::HashMap,
    time::Duration,
    os::fd::{RawFd, AsRawFd, OwnedFd},
};

use mio::{
    Events, Interest, Poll, Token,
    unix::SourceFd,
};

use crate::sync::{SyncResult, SyncError};
use super::NotifyCondition;

/// Token for eventfd events in mio
const EVENTFD_TOKEN: Token = Token(0);

/// Mio-based async event notification system
pub struct MioEventNotification {
    /// Whether notifications are enabled
    enabled: AtomicBool,
    /// Event file descriptor (Linux eventfd)
    event_fd: Option<std::os::fd::OwnedFd>,
    /// Mio poll instance for async operations (protected by mutex for mutable access)
    poll: Option<Mutex<Poll>>,
    /// Count of notifications sent
    notify_count: AtomicU64,
    /// Count of wait operations
    wait_count: AtomicU64,
}

impl MioEventNotification {
    /// Create new mio-based notification system
    pub fn new() -> SyncResult<Arc<Self>> {
        let event_fd = Self::create_eventfd()?;
        let poll = Self::create_poll_instance(&event_fd)?;

        Ok(Arc::new(Self {
            enabled: AtomicBool::new(true),
            event_fd,
            poll,
            notify_count: AtomicU64::new(0),
            wait_count: AtomicU64::new(0),
        }))
    }

    /// Create eventfd for notifications
    #[cfg(target_os = "linux")]
    fn create_eventfd() -> SyncResult<Option<std::os::fd::OwnedFd>> {
        use nix::sys::eventfd::{eventfd, EfdFlags};
        
        let fd = eventfd(0, EfdFlags::EFD_CLOEXEC | EfdFlags::EFD_NONBLOCK)
            .map_err(|_| SyncError::NotificationFailed)?;
        Ok(Some(fd))
    }

    #[cfg(not(target_os = "linux"))]
    fn create_eventfd() -> SyncResult<Option<std::os::fd::OwnedFd>> {
        Ok(None) // Fallback for non-Linux platforms
    }

    /// Create mio Poll instance and register eventfd
    fn create_poll_instance(event_fd: &Option<OwnedFd>) -> SyncResult<Option<Mutex<Poll>>> {
        let poll = Poll::new().map_err(|_| SyncError::NotificationFailed)?;
        
        if let Some(owned_fd) = event_fd {
            let fd = owned_fd.as_raw_fd();
            let mut source = SourceFd(&fd);
            poll.registry()
                .register(&mut source, EVENTFD_TOKEN, Interest::READABLE)
                .map_err(|_| SyncError::NotificationFailed)?;
        }

        Ok(Some(Mutex::new(poll)))
    }

    /// Notify waiting readers using mio
    pub fn notify(&self) -> SyncResult<()> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }
        
        self.notify_count.fetch_add(1, Ordering::Relaxed);
        
        #[cfg(target_os = "linux")]
        {
            if let Some(ref owned_fd) = self.event_fd {
                use nix::unistd::write;
                let fd = owned_fd.as_raw_fd();
                let value: u64 = 1;
                let buf = value.to_ne_bytes();
                match write(fd, &buf) {
                    Ok(_) => {},
                    Err(nix::errno::Errno::EAGAIN) => {}, // OK for non-blocking eventfd
                    Err(_) => return Err(SyncError::NotificationFailed),
                }
            }
        }
        
        Ok(())
    }

    /// Wait for notification using mio's async polling
    pub fn wait_async(&self, timeout_ms: Option<u64>) -> SyncResult<()> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }
        
        self.wait_count.fetch_add(1, Ordering::Relaxed);
        
        #[cfg(target_os = "linux")]
        {
            if let (Some(poll_mutex), Some(ref owned_fd)) = (&self.poll, &self.event_fd) {
                let mut poll = poll_mutex.lock().map_err(|_| SyncError::NotificationFailed)?;
                let fd = owned_fd.as_raw_fd();
                return self.wait_on_mio_poll(&mut *poll, fd, timeout_ms);
            }
        }
        
        // Fallback for non-Linux or if mio setup failed
        std::thread::sleep(Duration::from_millis(1));
        Ok(())
    }

    /// Wait using mio's efficient event polling
    #[cfg(target_os = "linux")]
    fn wait_on_mio_poll(&self, poll: &mut Poll, _fd: RawFd, timeout_ms: Option<u64>) -> SyncResult<()> {
        let mut events = Events::with_capacity(1);
        let timeout = timeout_ms.map(Duration::from_millis);
        
        match poll.poll(&mut events, timeout) {
            Ok(()) => {
                for event in events.iter() {
                    if event.token() == EVENTFD_TOKEN && event.is_readable() {
                        // Clear the eventfd by reading from it
                        self.clear_eventfd()?;
                        return Ok(());
                    }
                }
                // No events or timeout
                Ok(())
            }
            Err(_) => Err(SyncError::NotificationFailed),
        }
    }

    /// Clear eventfd after notification received
    #[cfg(target_os = "linux")]
    fn clear_eventfd(&self) -> SyncResult<()> {
        if let Some(ref owned_fd) = self.event_fd {
            use nix::unistd::read;
            let fd = owned_fd.as_raw_fd();
            let mut buf = [0u8; 8];
            let _ = read(fd, &mut buf); // Ignore result, just clearing
        }
        Ok(())
    }

    /// Get notification statistics
    pub fn get_stats(&self) -> (u64, u64) {
        (
            self.notify_count.load(Ordering::Relaxed),
            self.wait_count.load(Ordering::Relaxed)
        )
    }

    /// Enable or disable notifications
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    /// Check if notifications are enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Get the file descriptor for external integration
    #[cfg(target_os = "linux")]
    pub fn event_fd(&self) -> Option<RawFd> {
        self.event_fd.as_ref().map(|fd| fd.as_raw_fd())
    }
}

/// Mio-based condition notification manager
pub struct MioConditionNotifier {
    notifications: HashMap<String, Arc<MioEventNotification>>,
}

impl MioConditionNotifier {
    /// Create new condition notifier using mio
    pub fn new() -> Self {
        Self {
            notifications: HashMap::new(),
        }
    }

    /// Add a notification condition
    pub fn add_condition(&mut self, name: String, _condition: NotifyCondition) -> SyncResult<()> {
        let notification = MioEventNotification::new()?;
        self.notifications.insert(name, notification);
        Ok(())
    }

    /// Notify specific condition
    pub fn notify_condition(&self, name: &str) -> SyncResult<()> {
        if let Some(notification) = self.notifications.get(name) {
            notification.notify()
        } else {
            Err(SyncError::NotificationFailed)
        }
    }

    /// Wait for any condition with async polling
    pub fn wait_any_async(&self, timeout_ms: Option<u64>) -> SyncResult<Vec<String>> {
        let mut triggered = Vec::new();
        
        // This is a simplified implementation - a full async version would use
        // a single mio Poll instance to wait for multiple eventfds
        for (name, notification) in &self.notifications {
            match notification.wait_async(Some(1)) { // Very short timeout for each
                Ok(()) => {
                    triggered.push(name.clone());
                }
                Err(_) => continue,
            }
        }
        
        if triggered.is_empty() && timeout_ms.is_some() {
            std::thread::sleep(Duration::from_millis(timeout_ms.unwrap_or(0).min(100)));
        }
        
        Ok(triggered)
    }

    /// Get statistics for all notifications
    pub fn get_all_stats(&self) -> HashMap<String, (u64, u64)> {
        self.notifications
            .iter()
            .map(|(name, notification)| (name.clone(), notification.get_stats()))
            .collect()
    }
}

impl Default for MioConditionNotifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mio_notification_creation() {
        let notification = MioEventNotification::new();
        assert!(notification.is_ok());
        
        if let Ok(notif) = notification {
            assert!(notif.is_enabled());
            let (notify_count, wait_count) = notif.get_stats();
            assert_eq!(notify_count, 0);
            assert_eq!(wait_count, 0);
        }
    }

    #[test]
    fn test_mio_condition_notifier() {
        let mut notifier = MioConditionNotifier::new();
        
        let result = notifier.add_condition(
            "test_condition".to_string(), 
            NotifyCondition::Always
        );
        assert!(result.is_ok());
        
        let notify_result = notifier.notify_condition("test_condition");
        assert!(notify_result.is_ok());
    }

    #[test]
    fn test_mio_notification_enable_disable() {
        let notification = MioEventNotification::new().unwrap();
        
        assert!(notification.is_enabled());
        
        notification.set_enabled(false);
        assert!(!notification.is_enabled());
        
        notification.set_enabled(true);
        assert!(notification.is_enabled());
    }
}