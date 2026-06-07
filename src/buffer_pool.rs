use crate::common::{ PageId, FrameId, PAGE_SIZE};
use crate::disk_manager::DiskManager;
use crate::disk_scheduler::DiskScheduler;
use crate::frame_header::FrameHeader;
use crate::guard::{WritePageGuard, ReadPageGuard};
use crate::arc_replacer::ArcReplacer;
use crate::disk_scheduler::{DiskRequest, DiskRequestType};

use std::sync::{Mutex, Arc};
use std::sync::atomic::{AtomicU32, Ordering};
use std::collections::{ HashMap };
use std::sync::mpsc;

pub struct BufferPoolManager {
    num_frames: usize,
    next_page_id: AtomicU32,
    arc_replacer: Arc<Mutex<ArcReplacer>>,
    disk_scheduler: Arc<DiskScheduler>,
    frames: Vec<Arc<FrameHeader>>,
    free_frames: Vec<FrameId>,
    latch: Arc<Mutex<()>>,
    page_table: HashMap<PageId, FrameId>,
}

impl BufferPoolManager {
    pub fn new(num_frames: usize, disk_manager: Arc<Mutex<DiskManager>>) -> Self {
        let latch = Arc::new(Mutex::new(()));
        let replacer = Arc::new(Mutex::new(ArcReplacer::new(num_frames)));
        let disk_scheduler = Arc::new(DiskScheduler::new(Arc::clone(&disk_manager)));

        let _lock = latch.lock();

        let next_page_id = AtomicU32::new(0);

        let mut frames = Vec::with_capacity(num_frames);
        let mut free_frames = Vec::with_capacity(num_frames);
        let page_table = HashMap::with_capacity(num_frames);

        for i in 0..num_frames {
            frames.push(Arc::new(FrameHeader::new(i as FrameId)));
            free_frames.push(i as FrameId);
        }

        Self {
            num_frames,
            next_page_id,
            arc_replacer: replacer,
            disk_scheduler,
            frames,
            free_frames,
            latch: Arc::new(Mutex::new(())),
            page_table,
        }
    }

    pub fn new_page(&self) -> PageId {
        let _latch = self.latch.lock().unwrap();
        let page_id = self.next_page_id.fetch_add(1, Ordering::SeqCst);

        let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
        self.disk_scheduler.schedule(DiskRequest {
            r#type: DiskRequestType::Write,
            page_id,
            data: vec![0u8; PAGE_SIZE as usize],
            promise: tx
        });
        rx.recv().unwrap();

        page_id as u32
    }
 
    pub fn delete_page(&mut self, page_id: PageId) { 
        let _latch = self.latch.lock().unwrap();
        if self.page_table.contains_key(&page_id) {
            let frame_id = self.page_table[&page_id] as usize;
            let frame = Arc::clone(&self.frames[frame_id]);

            if frame.pin_count.load(Ordering::SeqCst) > 0 {
                return;
            }

            self.page_table.remove(&page_id);
            self.arc_replacer.lock().unwrap().remove(frame_id as FrameId);
            frame.reset();
            self.free_frames.push(frame_id as FrameId);

            let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
            let data = vec![0u8; PAGE_SIZE];
            self.disk_scheduler.schedule(DiskRequest {
                r#type: DiskRequestType::Delete,
                page_id,
                data,
                promise: tx,
            });
            let _ = rx.recv().unwrap();
        }
    }
 
    pub fn check_write_page(&mut self, page_id: PageId) -> Option<WritePageGuard>{
        // TODO: check if we can not hold latch during async disk i/o operations
        let _latch = self.latch.lock().unwrap();
        // page is already in the buffer pool
        if self.page_table.contains_key(&page_id) {
            let frame_id = self.page_table[&page_id] as usize;
            let frame = Arc::clone(&self.frames[frame_id]);
            let write_guard = WritePageGuard::new(
                page_id, 
                frame,
                Arc::clone(&self.arc_replacer),
                Arc::clone(&self.latch),
                Arc::clone(&self.disk_scheduler),
            );

            return Some(write_guard);
        }

        // page not in buffer pool and there is avaialable memory
        if self.free_frames.len() > 0 {
            let frame_id = self.free_frames.pop().unwrap();

            let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
            let data = vec![0u8; PAGE_SIZE];
            self.disk_scheduler.schedule(DiskRequest {
                r#type: DiskRequestType::Read,
                page_id,
                data,
                promise: tx,
            });

            let data = rx.recv().unwrap();

            let frame = Arc::clone(&self.frames[frame_id as usize]);

            frame.data.write().unwrap().copy_from_slice(&data.unwrap());

            self.page_table.insert(page_id, frame_id);
            let write_guard = WritePageGuard::new(
                page_id, 
                frame,
                Arc::clone(&self.arc_replacer),
                Arc::clone(&self.latch),
                Arc::clone(&self.disk_scheduler),
            );

            return Some(write_guard);

        // page not in buffer pool and there is no available memory
        } else {
            // evict the old page
            {
                let frame_id = self.arc_replacer.lock().unwrap().evict().unwrap();
                let frame = Arc::clone(&self.frames[frame_id as usize]);
                
                self.free_frames.push(frame_id);
                let to_remove = self.page_table
                .iter()
                .find(move|&(_, &fid)| fid == frame_id)
                .map(|(page_id, _)| *page_id);

                let old_page_id = to_remove.expect("evicted frame must have a page id");
                self.page_table.remove(&old_page_id);

                let write_guard = WritePageGuard::new(
                    old_page_id, 
                    frame,
                    Arc::clone(&self.arc_replacer),
                    Arc::clone(&self.latch),
                    Arc::clone(&self.disk_scheduler),
                );

                write_guard.flush();
                let frame = Arc::clone(&self.frames[frame_id as usize]);
                frame.reset();
            }
            // add the new page
            let frame_id = self.free_frames.pop().unwrap();
            let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();

            let data = vec![0u8; PAGE_SIZE];

            self.disk_scheduler.schedule(DiskRequest {
                r#type: DiskRequestType::Read,
                page_id,
                data, 
                promise: tx,
            });

            let data = rx.recv().unwrap();
            let new_frame = Arc::clone(&self.frames[frame_id as usize]);

            new_frame.data.write().unwrap().copy_from_slice(&data.unwrap());

            self.page_table.insert(page_id, frame_id);

            let write_guard = WritePageGuard::new(
                page_id,
                new_frame,
                Arc::clone(&self.arc_replacer),
                Arc::clone(&self.latch),
                Arc::clone(&self.disk_scheduler),
            );

          

            return Some(write_guard);
        }
    }

    pub fn check_read_page(&mut self, page_id: PageId) -> Option<ReadPageGuard> {
        // TODO: check if we can not hold latch during async disk i/o operations
        let _latch = self.latch.lock().unwrap();

        // page is already in the buffer pool
        if self.page_table.contains_key(&page_id) {
            let frame_id = self.page_table[&page_id] as usize;
            let frame = Arc::clone(&self.frames[frame_id]);
            let read_guard = ReadPageGuard::new(
                page_id,
                frame,
                Arc::clone(&self.arc_replacer),
                Arc::clone(&self.latch),
                Arc::clone(&self.disk_scheduler),
            );

            return Some(read_guard);
        }

        // page not in buffer pool and there is available memory
        if self.free_frames.len() > 0 {
            let frame_id = self.free_frames.pop().unwrap();

            let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
            let data = vec![0u8; PAGE_SIZE];

            self.disk_scheduler.schedule(DiskRequest {
                r#type: DiskRequestType::Read,
                page_id,
                data,
                promise: tx,
            });

            let data = rx.recv().unwrap();

            let frame = Arc::clone(&self.frames[frame_id as usize]);
            frame.data.write().unwrap().copy_from_slice(&data.unwrap());
            self.page_table.insert(page_id, frame_id);
            let read_guard = ReadPageGuard::new(
                page_id,
                frame,
                Arc::clone(&self.arc_replacer),
                Arc::clone(&self.latch),
                Arc::clone(&self.disk_scheduler),
            );

            return Some(read_guard);

        // page not in buffer pool and there is no available memory
        } else {
            {
                let frame_id = self.arc_replacer.lock().unwrap().evict().unwrap();
                let frame = Arc::clone(&self.frames[frame_id as usize]);

                self.free_frames.push(frame_id);
                let to_remove = self.page_table
                .iter()
                .find(move|&(_, &fid)| fid == frame_id)
                .map(|(page_id, _)| *page_id);

                let old_page_id = to_remove.expect("evicted frame must have a page id");
                self.page_table.remove(&old_page_id);

                let read_guard = ReadPageGuard::new(
                    old_page_id,
                    frame,
                    Arc::clone(&self.arc_replacer),
                    Arc::clone(&self.latch),
                    Arc::clone(&self.disk_scheduler),
                );

                read_guard.flush();
                let frame = Arc::clone(&self.frames[frame_id as usize]);
                frame.reset();
            }

            let frame_id = self.free_frames.pop().unwrap();
            let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();

            let data = vec![0u8; PAGE_SIZE];

            self.disk_scheduler.schedule(DiskRequest {
                r#type: DiskRequestType::Read,
                page_id,
                data,
                promise: tx,
            });

            let data = rx.recv().unwrap();
            let new_frame = Arc::clone(&self.frames[frame_id as usize]);

            new_frame.data.write().unwrap().copy_from_slice(&data.unwrap());
            
            self.page_table.insert(page_id, frame_id);
            
            let read_guard = ReadPageGuard::new(
                page_id,
                new_frame,
                Arc::clone(&self.arc_replacer),
                Arc::clone(&self.latch),
                Arc::clone(&self.disk_scheduler),
            );

            return Some(read_guard);
        }
    }

    pub fn flush_page(&mut self, page_id: PageId) -> bool {
        let _latch = self.latch.lock().unwrap();

        if self.page_table.contains_key(&page_id) {
            let frame_id = self.page_table[&page_id] as usize;
            let frame = Arc::clone(&self.frames[frame_id]);
            
            let read_page_guard = ReadPageGuard::new(
                page_id,
                frame,
                Arc::clone(&self.arc_replacer),
                Arc::clone(&self.latch),
                Arc::clone(&self.disk_scheduler),
            );

            return read_page_guard.flush();
        }

        return false;
    }

    pub fn flush_all_pages(&mut self) -> bool {
        let _latch = self.latch.lock().unwrap();

        if self.page_table.is_empty() {
            return false;
        } 

        for (page_id, frame_id) in self.page_table.iter() {
            let frame = Arc::clone(&self.frames[*frame_id as usize]);
            let read_page_guard = ReadPageGuard::new(
                *page_id,
                frame,
                Arc::clone(&self.arc_replacer),
                Arc::clone(&self.latch),
                Arc::clone(&self.disk_scheduler)
            );

            read_page_guard.flush();
        }

        return true;
    }
}

mod buffer_pool_manager_tests {
    use super::*;

    #[test]
    fn test_basic_read_write_delete_operation() {
        let mut bpm = BufferPoolManager::new(10, Arc::new(Mutex::new(DiskManager::new("test.db"))));
        let page_id = bpm.new_page();

        let mut write_guard = bpm.check_write_page(page_id).unwrap();
        write_guard.data_mut().unwrap().fill(1);

        let read_guard = bpm.check_read_page(page_id).unwrap();
        let data = read_guard.data().unwrap();

        assert_eq!(data.as_slice(), &[1; PAGE_SIZE]);

        read_guard.frame.is_dirty.store(false, Ordering::SeqCst);
        
        assert_eq!(bpm.flush_page(page_id), false);

        read_guard.frame.is_dirty.store(true, Ordering::SeqCst);

        assert_eq!(bpm.flush_page(page_id), true);

        let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
        bpm.disk_scheduler.schedule(DiskRequest {
            r#type: DiskRequestType::Read,
            page_id: page_id,
            data: vec![0u8; PAGE_SIZE],
            promise: tx,
        });

        let data_from_disk = rx.recv().unwrap();
        assert_eq!(data_from_disk.unwrap().as_slice(), &[1u8; PAGE_SIZE]);
    }

    // TODO: test multiple threads read and write operations not now do it later i need to continue studying other database components
    #[test]
    fn test_multiple_threads_read_write_delete_operations() {
        let bpm = BufferPoolManager::new(10, Arc::new(Mutex::new(DiskManager::new("test.db"))));
        // create multiple pages 
        println!("bpm {:?}", bpm.page_table.len());
        // read and wirte delete operations on different threads
    }
}