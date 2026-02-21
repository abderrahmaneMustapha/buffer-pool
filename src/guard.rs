use crate::common::{ PageId };
use crate::frame_header::FrameHeader;
use crate::arc_replacer::ArcReplacer;
use crate::disk_scheduler::DiskScheduler;
use crate::disk_scheduler::{DiskRequest, DiskRequestType};
use std::sync::{Mutex, Arc};
use std::sync::atomic::{Ordering};
use std::sync::mpsc;
use std::sync::{RwLockReadGuard, RwLockWriteGuard, LockResult};


pub struct WritePageGuard {
    page_id: PageId,
    frame: Arc<FrameHeader>,
    arc_replacer: Arc<Mutex<ArcReplacer>>,
    bpm_latch: Arc<Mutex<()>>,
    disk_scheduler: Arc<DiskScheduler>,
    is_valid: bool,
}

impl WritePageGuard {
    pub fn new(
        page_id: PageId, 
        frame: Arc<FrameHeader>, 
        arc_replacer: Arc<Mutex<ArcReplacer>>, 
        bpm_latch: Arc<Mutex<()>>, 
        disk_scheduler: Arc<DiskScheduler>
    ) -> Self {
        frame.pin_count.fetch_add(1, Ordering::SeqCst);
        arc_replacer.lock().unwrap().record_access(frame.frame_id, page_id);

        Self {
            page_id, 
            frame,
            arc_replacer, 
            bpm_latch,
            disk_scheduler,
            is_valid: true,
        }
    }

    pub fn flush(&self) -> bool {
        if !self.is_valid || !self.frame.is_dirty.load(Ordering::SeqCst) {
            return false;
        }

        let data = self.frame.data.read().unwrap().clone();

        let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();

        self.disk_scheduler.schedule(DiskRequest {
            r#type: DiskRequestType::Write,
            page_id: self.page_id,
            data,
            promise: tx,
        });

        rx.recv().unwrap();

        self.frame.is_dirty.store(false, Ordering::SeqCst);

        return true;
    }

    pub fn data_mut(&mut self) -> LockResult<RwLockWriteGuard<'_, Vec<u8>>> {
        self.frame.data.write()
    }

    pub fn data(&self) -> LockResult<RwLockReadGuard<'_, Vec<u8>>> {
        self.frame.data.read()
    }
} 

impl Drop for WritePageGuard {
    fn drop(&mut self) {
        if !self.is_valid {
            return;
        }

        self.frame.pin_count.fetch_sub(1, Ordering::SeqCst);
        if  self.frame.pin_count.load(Ordering::SeqCst) == 0 {
            self.arc_replacer.lock().unwrap().set_evictable(self.frame.frame_id);
        }

        self.is_valid = false;
    }
}

pub struct ReadPageGuard {
    page_id: PageId,
    frame: Arc<FrameHeader>,
    arc_replacer: Arc<Mutex<ArcReplacer>>,
    bpm_latch: Arc<Mutex<()>>,
    disk_scheduler: Arc<DiskScheduler>,
    is_valid: bool,
}

impl ReadPageGuard {

    pub fn new(
        page_id: PageId, 
        frame: Arc<FrameHeader>, 
        arc_replacer: Arc<Mutex<ArcReplacer>>, 
        bpm_latch: Arc<Mutex<()>>, 
        disk_scheduler: Arc<DiskScheduler>
    ) -> Self {
        frame.pin_count.fetch_add(1, Ordering::SeqCst);
        arc_replacer.lock().unwrap().record_access(frame.frame_id, page_id);

        Self {
            page_id, 
            frame,
            arc_replacer,
            bpm_latch,
            disk_scheduler,
            is_valid: true,
        }
    }

    pub fn data(&self) -> LockResult<RwLockReadGuard<'_, Vec<u8>>> {
        return self.frame.data.read()
    }

    pub fn flush(&self) -> bool {
        if !self.is_valid || !self.frame.is_dirty.load(Ordering::SeqCst) {
            return false;
        }

        let data = self.frame.data.read().unwrap().clone();

        let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();

        self.disk_scheduler.schedule(DiskRequest {
            r#type: DiskRequestType::Write,
            page_id: self.page_id,
            data,
            promise: tx,
        });

        rx.recv().unwrap();

        self.frame.is_dirty.store(false, Ordering::SeqCst);
        return true;
    }
}

impl Drop for ReadPageGuard {

    fn drop(&mut self) {
        if !self.is_valid {
            return;
        }

        self.frame.pin_count.fetch_sub(1, Ordering::SeqCst);
        if self.frame.pin_count.load(Ordering::SeqCst) == 0 {
            self.arc_replacer.lock().unwrap().set_evictable(self.frame.frame_id);
        }

        self.is_valid = false;
    }
}