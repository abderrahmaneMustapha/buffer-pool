

// ============================================================================
// BUFFER POOL MANAGER
// Manages the buffer pool and the ARC replacer
// ============================================================================

use crate::common::{ PageId, FrameId, PAGE_SIZE };
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use std::sync::RwLock;

pub struct FrameHeader {
    pub frame_id: FrameId,
    pub pin_count: AtomicUsize,
    pub is_dirty: AtomicBool, // if false means no thread is currently using this frame its safe to evict or reuse
    pub data: RwLock<Vec<u8>>,
}

impl FrameHeader {
    pub fn new(frame_id: FrameId) -> Self {
        Self {
            frame_id,
            pin_count: AtomicUsize::new(0),
            is_dirty: AtomicBool::new(false),
            data: RwLock::new(vec![0u8; PAGE_SIZE]),
        }
    }

    pub fn reset(& self) {
        self.data.write().unwrap().fill(0);
        // TODO: check if we can use Relaxed here and if it will affect the correctness of the program
        self.pin_count.store(0, Ordering::SeqCst);
        self.is_dirty.store(false, Ordering::SeqCst);
    }
}