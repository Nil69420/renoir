//! Epoch-based reclamation for variable-sized message memory management
//! 
//! Provides efficient garbage collection for large payloads using epoch-based
//! reclamation with per-reader watermarks.

use std::sync::{Arc, Mutex, RwLock, atomic::{AtomicU64, AtomicUsize, Ordering}};
use std::collections::HashMap;
use std::time::{SystemTime, Duration};
use crate::error::{RenoirError, Result};
use crate::shared_pools::SharedBufferPoolManager;
use super::blob::BlobDescriptor;

/// Unique identifier for readers in the epoch reclamation system
pub type ReaderId = u64;

/// Statistics for reclamation operations
#[derive(Debug, Default)]
pub struct ReclamationStats {
    /// Number of readers registered
    pub readers_registered: AtomicUsize,
    /// Number of readers unregistered
    pub readers_unregistered: AtomicUsize,
    /// Number of items marked for reclamation
    pub items_marked_for_reclamation: AtomicUsize,
    /// Number of items successfully reclaimed
    pub items_reclaimed: AtomicUsize,
    /// Number of items force-reclaimed
    pub items_force_reclaimed: AtomicUsize,
    /// Number of reclamation errors
    pub reclamation_errors: AtomicUsize,
    /// Number of epochs advanced
    pub epochs_advanced: AtomicUsize,
    /// Number of stale readers cleaned up
    pub stale_readers_cleaned: AtomicUsize,
}

impl Clone for ReclamationStats {
    fn clone(&self) -> Self {
        Self {
            readers_registered: AtomicUsize::new(self.readers_registered.load(std::sync::atomic::Ordering::Relaxed)),
            readers_unregistered: AtomicUsize::new(self.readers_unregistered.load(std::sync::atomic::Ordering::Relaxed)),
            items_marked_for_reclamation: AtomicUsize::new(self.items_marked_for_reclamation.load(std::sync::atomic::Ordering::Relaxed)),
            items_reclaimed: AtomicUsize::new(self.items_reclaimed.load(std::sync::atomic::Ordering::Relaxed)),
            items_force_reclaimed: AtomicUsize::new(self.items_force_reclaimed.load(std::sync::atomic::Ordering::Relaxed)),
            reclamation_errors: AtomicUsize::new(self.reclamation_errors.load(std::sync::atomic::Ordering::Relaxed)),
            epochs_advanced: AtomicUsize::new(self.epochs_advanced.load(std::sync::atomic::Ordering::Relaxed)),
            stale_readers_cleaned: AtomicUsize::new(self.stale_readers_cleaned.load(std::sync::atomic::Ordering::Relaxed)),
        }
    }
}

/// Epoch value for tracking reclamation points
pub type Epoch = u64;

/// Reclamation policy configuration
#[derive(Debug, Clone)]
pub struct ReclamationPolicy {
    /// Maximum age before forced reclamation
    pub max_age: Duration,
    /// Maximum number of pending reclamations
    pub max_pending_reclamations: usize,
    /// Epoch advancement frequency
    pub epoch_advance_interval: Duration,
    /// Enable aggressive reclamation under memory pressure
    pub aggressive_reclamation: bool,
}

impl Default for ReclamationPolicy {
    fn default() -> Self {
        Self {
            max_age: Duration::from_secs(60), // 1 minute
            max_pending_reclamations: 10000,
            epoch_advance_interval: Duration::from_millis(100), // 100ms
            aggressive_reclamation: true,
        }
    }
}

/// Pending reclamation entry
#[derive(Debug)]
struct PendingReclamation {
    /// Buffer descriptor to reclaim
    descriptor: BlobDescriptor,
    /// Epoch when it became reclaimable
    #[allow(dead_code)]
    reclaim_epoch: Epoch,
    /// Timestamp when it was marked for reclamation
    marked_at: SystemTime,
}

/// Reader state for epoch tracking
#[derive(Debug)]
struct ReaderState {
    /// Current epoch for this reader
    current_epoch: AtomicU64,
    /// Last activity timestamp
    last_activity: Mutex<SystemTime>,
    /// Whether reader is active
    is_active: AtomicU64, // 1 = active, 0 = inactive
}

impl ReaderState {
    fn new() -> Self {
        Self {
            current_epoch: AtomicU64::new(0),
            last_activity: Mutex::new(SystemTime::now()),
            is_active: AtomicU64::new(1),
        }
    }
    
    fn update_activity(&self) {
        let mut last_activity = self.last_activity.lock().unwrap();
        *last_activity = SystemTime::now();
    }
    
    fn is_stale(&self, timeout: Duration) -> bool {
        let last_activity = self.last_activity.lock().unwrap();
        last_activity.elapsed().unwrap_or(timeout) >= timeout
    }
}

/// Epoch-based reclaimer for variable-sized messages
#[derive(Debug)]
pub struct EpochReclaimer {
    /// Buffer pool manager
    pool_manager: Arc<SharedBufferPoolManager>,
    /// Reclamation policy
    policy: ReclamationPolicy,
    /// Current global epoch
    global_epoch: AtomicU64,
    /// Reader states
    readers: RwLock<HashMap<ReaderId, Arc<ReaderState>>>,
    /// Next reader ID
    next_reader_id: AtomicU64,
    /// Pending reclamations by epoch
    pending_reclamations: Mutex<HashMap<Epoch, Vec<PendingReclamation>>>,
    /// Last epoch advance time
    last_epoch_advance: Mutex<SystemTime>,
    /// Statistics
    stats: ReclamationStats,
}

impl EpochReclaimer {
    /// Create a new epoch reclaimer
    pub fn new(pool_manager: Arc<SharedBufferPoolManager>, policy: ReclamationPolicy) -> Self {
        Self {
            pool_manager,
            policy,
            global_epoch: AtomicU64::new(1),
            readers: RwLock::new(HashMap::new()),
            next_reader_id: AtomicU64::new(1),
            pending_reclamations: Mutex::new(HashMap::new()),
            last_epoch_advance: Mutex::new(SystemTime::now()),
            stats: ReclamationStats::default(),
        }
    }
    
    /// Register a new reader
    pub fn register_reader(&self) -> ReaderId {
        let reader_id = self.next_reader_id.fetch_add(1, Ordering::SeqCst);
        let reader_state = Arc::new(ReaderState::new());
        
        // Set initial epoch to current global epoch
        let current_epoch = self.global_epoch.load(Ordering::SeqCst);
        reader_state.current_epoch.store(current_epoch, Ordering::SeqCst);
        
        let mut readers = self.readers.write().unwrap();
        readers.insert(reader_id, reader_state);
        
        self.stats.readers_registered.fetch_add(1, Ordering::Relaxed);
        reader_id
    }
    
    /// Unregister a reader
    pub fn unregister_reader(&self, reader_id: ReaderId) -> Result<()> {
        let mut readers = self.readers.write().unwrap();
        if readers.remove(&reader_id).is_some() {
            self.stats.readers_unregistered.fetch_add(1, Ordering::Relaxed);
            Ok(())
        } else {
            Err(RenoirError::invalid_parameter("reader_id", "Reader not found"))
        }
    }
    
    /// Update reader epoch (call when reader accesses data)
    pub fn update_reader_epoch(&self, reader_id: ReaderId) -> Result<Epoch> {
        let readers = self.readers.read().unwrap();
        if let Some(reader_state) = readers.get(&reader_id) {
            reader_state.update_activity();
            let current_epoch = self.global_epoch.load(Ordering::SeqCst);
            reader_state.current_epoch.store(current_epoch, Ordering::SeqCst);
            Ok(current_epoch)
        } else {
            Err(RenoirError::invalid_parameter("reader_id", "Reader not found"))
        }
    }
    
    /// Mark blob descriptor for reclamation
    pub fn mark_for_reclamation(&self, descriptor: BlobDescriptor) -> Result<()> {
        if !descriptor.release() {
            // Still has references, don't reclaim yet
            return Ok(());
        }
        
        let current_epoch = self.global_epoch.load(Ordering::SeqCst);
        let pending = PendingReclamation {
            descriptor,
            reclaim_epoch: current_epoch,
            marked_at: SystemTime::now(),
        };
        
        let mut reclamations = self.pending_reclamations.lock().unwrap();
        reclamations.entry(current_epoch).or_insert_with(Vec::new).push(pending);
        
        self.stats.items_marked_for_reclamation.fetch_add(1, Ordering::Relaxed);
        
        // Check if we need to trigger reclamation
        let total_pending: usize = reclamations.values().map(|v| v.len()).sum();
        if total_pending > self.policy.max_pending_reclamations {
            drop(reclamations); // Release lock before triggering reclamation
            self.try_advance_epoch()?;
            self.reclaim_eligible()?;
        }
        
        Ok(())
    }
    
    /// Try to advance the global epoch
    pub fn try_advance_epoch(&self) -> Result<bool> {
        let mut last_advance = self.last_epoch_advance.lock().unwrap();
        let should_advance = last_advance.elapsed().unwrap_or_default() >= self.policy.epoch_advance_interval;
        
        if !should_advance {
            return Ok(false);
        }
        
        *last_advance = SystemTime::now();
        drop(last_advance);
        
        // Advance epoch
        let _new_epoch = self.global_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        
        // Clean up stale readers
        self.cleanup_stale_readers()?;
        
        self.stats.epochs_advanced.fetch_add(1, Ordering::Relaxed);
        
        Ok(true)
    }
    
    /// Reclaim eligible items based on reader watermarks
    pub fn reclaim_eligible(&self) -> Result<usize> {
        let min_reader_epoch = self.get_min_reader_epoch();
        let mut reclaimed_count = 0;
        
        let mut reclamations = self.pending_reclamations.lock().unwrap();
        let mut epochs_to_remove = Vec::new();
        
        for (&epoch, pending_list) in reclamations.iter() {
            if epoch < min_reader_epoch {
                // This epoch is safe to reclaim
                for pending in pending_list {
                    match self.reclaim_buffer(&pending.descriptor) {
                        Ok(()) => {
                            reclaimed_count += 1;
                            self.stats.items_reclaimed.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            // Log error but continue with other reclamations
                            eprintln!("Failed to reclaim buffer: {}", e);
                            self.stats.reclamation_errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                epochs_to_remove.push(epoch);
            }
        }
        
        // Remove reclaimed epochs
        for epoch in epochs_to_remove {
            reclamations.remove(&epoch);
        }
        
        Ok(reclaimed_count)
    }
    
    /// Force reclamation of old items (emergency cleanup)
    pub fn force_reclaim_old(&self) -> Result<usize> {
        let mut reclaimed_count = 0;
        let cutoff_time = SystemTime::now() - self.policy.max_age;
        
        let mut reclamations = self.pending_reclamations.lock().unwrap();
        let mut epochs_to_remove = Vec::new();
        
        for (&epoch, pending_list) in reclamations.iter_mut() {
            pending_list.retain(|pending| {
                if pending.marked_at < cutoff_time {
                    // Force reclaim this item
                    if let Ok(()) = self.reclaim_buffer(&pending.descriptor) {
                        reclaimed_count += 1;
                        self.stats.items_force_reclaimed.fetch_add(1, Ordering::Relaxed);
                    }
                    false // Remove from pending list
                } else {
                    true // Keep in pending list
                }
            });
            
            if pending_list.is_empty() {
                epochs_to_remove.push(epoch);
            }
        }
        
        // Remove empty epochs
        for epoch in epochs_to_remove {
            reclamations.remove(&epoch);
        }
        
        Ok(reclaimed_count)
    }
    
    /// Get minimum reader epoch (watermark)
    fn get_min_reader_epoch(&self) -> Epoch {
        let readers = self.readers.read().unwrap();
        readers.values()
            .filter(|state| state.is_active.load(Ordering::SeqCst) == 1)
            .map(|state| state.current_epoch.load(Ordering::SeqCst))
            .min()
            .unwrap_or_else(|| self.global_epoch.load(Ordering::SeqCst))
    }
    
    /// Clean up stale readers
    fn cleanup_stale_readers(&self) -> Result<()> {
        let reader_timeout = self.policy.max_age;
        let mut readers = self.readers.write().unwrap();
        let initial_count = readers.len();
        
        readers.retain(|_, state| {
            if state.is_stale(reader_timeout) {
                state.is_active.store(0, Ordering::SeqCst);
                false
            } else {
                true
            }
        });
        
        let cleaned_count = initial_count - readers.len();
        self.stats.stale_readers_cleaned.fetch_add(cleaned_count, Ordering::Relaxed);
        
        Ok(())
    }
    
    /// Actually reclaim a buffer
    fn reclaim_buffer(&self, descriptor: &BlobDescriptor) -> Result<()> {
        let message_desc = crate::topic::MessageDescriptor::new(
            descriptor.pool_id,
            descriptor.buffer_handle,
            descriptor.total_size as u32,
        );
        
        self.pool_manager.return_buffer(&message_desc)
    }
    
    /// Get reclamation statistics
    pub fn get_statistics(&self) -> ReclamationStats {
        self.stats.clone()
    }
    
    /// Perform maintenance (advance epoch, reclaim eligible, cleanup)
    pub fn maintenance(&self) -> Result<MaintenanceResult> {
        let epoch_advanced = self.try_advance_epoch()?;
        let reclaimed = self.reclaim_eligible()?;
        let force_reclaimed = if self.policy.aggressive_reclamation {
            self.force_reclaim_old()?
        } else {
            0
        };
        
        Ok(MaintenanceResult {
            epoch_advanced,
            items_reclaimed: reclaimed,
            items_force_reclaimed: force_reclaimed,
        })
    }
}

/// Result of maintenance operation
#[derive(Debug)]
pub struct MaintenanceResult {
    /// Whether epoch was advanced
    pub epoch_advanced: bool,
    /// Number of items reclaimed normally
    pub items_reclaimed: usize,
    /// Number of items force reclaimed
    pub items_force_reclaimed: usize,
}