use std::collections::{ HashMap, HashSet, VecDeque };
use std::time::SystemTime;
use std::fs::{remove_file, OpenOptions, File};
use std::sync::{Mutex, Arc, mpsc, RwLock, RwLockReadGuard, RwLockWriteGuard, LockResult};
use std::io::{self, Read, Write, SeekFrom, Seek };
use std::thread;
use std::sync::atomic::{AtomicUsize, AtomicU32, Ordering, AtomicBool};

type PageId = u32;
type FrameId = u32;

const PAGE_SIZE: usize = 8192;
const DB_IO_SIZE: usize = 16;

// ============================================================================
// SECTION 1: ARC REPLACER
// ============================================================================
// Adaptive Replacement Cache (ARC) replacement policy implementation
// Tracks page usage and decides which frames to evict from buffer pool
// ============================================================================
struct ArcReplacer {
    mru: VecDeque<(FrameId, PageId)>, // most recently used list
    mfu: VecDeque<(FrameId, PageId)>, // most frequently used list
    timestamp_access: HashMap<(FrameId, PageId), SystemTime>, // record when sepecifc page is accessed in timestamp in a frame

    mru_ghost: VecDeque<PageId>, // most recently used evicted from buffer pool
    mfu_ghost: VecDeque<PageId>, // most frequently used evicted from buffer pool

    mru_target_size: u32, // target size of the MRU list , the actual size of MRU list can be different than the this

    replacer_size: usize, // the maximum number of the frames that ArcReplacer support, the same size as the buffer pool

    curr_size: u32, // current size of evictable frames, init with 0 and incrase when a frame is marked as evictable, frame is not in use or pinned decrease
    
    dirty_pages: HashSet<PageId>, // pages marked as dity
    evictable_pages: HashSet<FrameId>, // pages marked as evictable by frame
}

impl ArcReplacer {
    fn new(replacer_size: usize) -> Self {
        Self {
            mru: VecDeque::new(),
            mfu: VecDeque::new(),
            timestamp_access: HashMap::new(),
            mru_ghost: VecDeque::new(),
            mfu_ghost: VecDeque::new(),
            mru_target_size: 0,
            replacer_size: replacer_size,
            curr_size: 0,
            dirty_pages: HashSet::new(),
            evictable_pages: HashSet::new(),
        }
    }

    fn size(&self) -> usize {
        return self.evictable_pages.len();
    }
 
    fn set_evictable(&mut self, frame_id: u32) {
        self.evictable_pages.insert(frame_id);
        self.curr_size += 1;
    }
    
    fn record_access(&mut self, frame_id: u32, page_id: u32) {

        self.timestamp_access.insert((frame_id, page_id), SystemTime::now());

        if self.mru.contains(&(frame_id, page_id)) || self.mfu.contains(&(frame_id, page_id)) {
            let target = (frame_id, page_id);
            self.mru.retain(|&(a, b)| !(a == target.0 && b == target.1));
            self.mfu.retain(|&(a, b)| !(a == target.0 && b == target.1));
            self.mfu.push_front((frame_id, page_id));
            return ();
        }

        if self.mru_ghost.contains(&page_id) {
            if self.mru_ghost.len() >= self.mfu_ghost.len() && (self.mru_target_size + 1) < self.replacer_size.try_into().unwrap() {
                self.mru_target_size += 1
            } else  {
                let increase: u32 = (self.mfu_ghost.len() / self.mru_ghost.len()) as u32;
                if self.mru_target_size + increase <= self.replacer_size.try_into().unwrap() {
                    self.mru_target_size += increase;
                }
            }


            self.mru_ghost.retain(|&a| !( a == page_id));
            self.mfu.push_front((frame_id, page_id));
            return ();
        }

        if self.mfu_ghost.contains(&page_id) {
            if self.mfu_ghost.len() >= self.mru_ghost.len() && (self.mru_target_size - 1) > 0 {
                self.mru_target_size -= 1;
            } else {
                let decrease: u32 = (self.mru_ghost.len() / self.mfu_ghost.len()) as u32;
                if self.mru_target_size >= decrease {
                    self.mru_target_size -= decrease;
                }
            }
            self.mfu_ghost.retain(|&a| !( a == page_id));
            self.mfu.push_front((frame_id, page_id));
            return ();
        }

        if self.mru.len() + self.mru_ghost.len() == self.replacer_size {
            self.mru_ghost.pop_back();
            self.mru.push_front((frame_id, page_id));
            return ();
        }

        if self.mru.len() + self.mru_ghost.len() < self.replacer_size {
            if 
                self.mru.len() + 
                self.mfu.len() + 
                self.mfu_ghost.len() + 
                self.mru_ghost.len() ==
                2 * self.replacer_size
            {
                self.mfu_ghost.pop_back();
                self.mru.push_front((frame_id, page_id));
            } else {
                self.mru.push_front((frame_id, page_id));
            }
            return ();
        }
    }

    fn evict(&mut self) -> Option<FrameId> {
        let should_evict_from_mru = self.mru.len() >= self.mru_target_size as usize;

        if should_evict_from_mru {
            if let Some(evicted) = self.evict_from_mru_ghost() {
                return Some(evicted);
            }

            if let Some(evicted) = self.evict_from_mfu_ghost() {
                return Some(evicted);
            }
        } else {
            if let Some(evicted) = self.evict_from_mfu_ghost() {
                return Some(evicted);
            }

            if let Some(evicted) = self.evict_from_mru_ghost() {
                return Some(evicted);
            }
        }

        None
    }

    fn evict_from_mfu_ghost(&mut self) -> Option<FrameId> {
        for i in (0..self.mfu.len()).rev() {
            if let Some(&(frame_id, page_id)) = self.mfu.get(i) {
                if self.evictable_pages.contains(&frame_id) {
                    self.mfu.remove(i);

                    self.mfu_ghost.push_front(page_id);

                    self.evictable_pages.remove(&frame_id);

                    self.timestamp_access.remove(&(frame_id, page_id));

                    self.curr_size -= 1;

                    return Some(frame_id);
                }
            }
        }

        None
    }

    fn evict_from_mru_ghost(&mut self) -> Option<FrameId> {
        for i in (0..self.mru.len()).rev() {
            if let Some(&(frame_id, page_id)) = self.mru.get(i) {
                if self.evictable_pages.contains(&frame_id) {
                    self.mru.remove(i);

                    self.mru_ghost.push_front(page_id);

                    self.evictable_pages.remove(&frame_id);

                    self.timestamp_access.remove(&(frame_id, page_id));

                    self.curr_size -= 1;

                    return Some(frame_id);
                }
            }
        }

        None
    }

    fn remove(&mut self, frame_id: u32) {

        if self.evictable_pages.contains(&frame_id) {
            return;
        }

        let mut page_id_option = Option::<PageId>::None;

        self.mru.retain(|&(f, p)| {
            if f == frame_id {
                page_id_option = Some(p);
                false
            } else {
                true
            }
        });

        if page_id_option.is_none() {
            self.mfu.retain(|&(f, p)| {
                if f == frame_id {
                    page_id_option = Some(p);
                    false
                } else {
                    true
                }
            });
        }

        if let Some(page_id) = page_id_option {
            self.timestamp_access.remove(&(frame_id, page_id));
            self.evictable_pages.remove(&frame_id);
            self.dirty_pages.remove(&page_id);
        }

        self.mru_ghost.retain(|&p| p != page_id_option.unwrap_or(0));
        self.mfu_ghost.retain(|&p| p != page_id_option.unwrap_or(0));
    }
}

#[cfg(test)]
mod arc_replacer_tests {
    use super::*;

    #[test]
    fn basic_record_access() {
        let mut replacer = ArcReplacer::new(10);

        assert!(replacer.replacer_size == 10);

        replacer.record_access(1, 1);
        replacer.record_access(22, 1);

        assert!(replacer.mru.contains(&(1, 1)));
        assert!(replacer.mru.contains(&(22, 1)));

        assert!(replacer.mru.len() == 2);
        assert!(replacer.mfu.len() == 0);
        assert!(replacer.mru_target_size == 0);

        assert_eq!(replacer.timestamp_access.contains_key(&(1, 1)), true);
        assert_eq!(replacer.timestamp_access.contains_key(&(22, 1)), true);
    }

    #[test]
    fn promote_from_mru_to_mfu() {
        let mut replacer = ArcReplacer::new(10);
        replacer.record_access(1, 1);
        replacer.record_access(2, 2);

        assert!(replacer.mru.contains(&(1, 1)));
        assert!(replacer.mru.contains(&(2, 2)));
        
        replacer.record_access(1, 1);

        assert!(replacer.mfu.contains(&(1, 1)));
        assert_eq!(replacer.mru.contains(&(1, 1)), false);
    }
    
    #[test]
    fn evict_from_mru_lge_target_size() {
        let mut replacer = ArcReplacer::new(10);
        replacer.record_access(1, 1);
        replacer.record_access(2, 2);
        replacer.record_access(3, 3);

        replacer.set_evictable(1);

        assert_eq!(replacer.curr_size, 1);
        assert_eq!(replacer.evictable_pages.contains(&1), true);

        replacer.evict();

        assert!(replacer.mru_ghost.contains(&1));
        assert_eq!(replacer.mru.contains(&(1, 1)), false);
        assert_eq!(replacer.mru.contains(&(2, 2)), true);
        assert_eq!(replacer.mru.contains(&(3, 3)), true);
        assert_eq!(replacer.mfu_ghost.len(), 0);
    }

    #[test]
    fn evict_from_mfu_mru_lge_target_size() {
       let mut replacer = ArcReplacer::new(10);
       replacer.mfu.push_front((1, 1));
       replacer.mfu.push_front((2, 2));
       replacer.mfu.push_front((3, 3));
       replacer.mfu.push_front((4, 4));

       replacer.set_evictable(3);

       let evicted = replacer.evict();
       assert_eq!(evicted, Some(3));

       assert!(replacer.mfu_ghost.contains(&3));
       assert_eq!(replacer.mru_ghost.len(), 0);
    }
    
    #[test]
    fn evict_from_mfu_mru_sml_target_size() {
        let mut replacer = ArcReplacer::new(10);

        replacer.mru.push_front((1, 1));
        replacer.mru_target_size = 5;

        replacer.mfu.push_front((2, 2));
        replacer.mfu.push_front((3, 3));

        replacer.set_evictable(3);

        let evicted = replacer.evict();
        assert_eq!(evicted, Some(3));

        assert!(replacer.mfu_ghost.contains(&3));
        assert_eq!(replacer.mru_ghost.len(), 0)
    }

    #[test] 
    fn evict_from_mru_mru_sml_target_size() {
        let mut replacer = ArcReplacer::new(10);

        replacer.mru.push_front((1, 1));
        replacer.mru_target_size = 5;

        replacer.mfu.push_front((2, 2));
        replacer.mfu.push_front((3, 3));

        replacer.set_evictable(1);

        replacer.evict();

        assert!(replacer.mru_ghost.contains(&1));
    }

    #[test]
    fn no_evict() {
        let mut replacer = ArcReplacer::new(10);
        replacer.mru.push_front((1, 1));
        replacer.mfu.push_front((2, 2));

        let evicted = replacer.evict();

        assert_eq!(evicted, None);
    }

    #[test]
    fn mru_hit_increase_target_size_by_one() {
        let mut replacer = ArcReplacer::new(10);
        
        replacer.mru_ghost.push_front(1);
        replacer.mfu_ghost.push_front(2);


        replacer.record_access(1, 1);

        assert_eq!(replacer.mru_target_size, 1);
        assert!(replacer.mfu.contains(&(1, 1)));
    }

    #[test]
    fn mru_hit_increase_target_size_by_mod() {
        let mut replacer = ArcReplacer::new(10);
        
        replacer.mru_ghost.push_front(1);
        replacer.mfu_ghost.push_front(2);
        replacer.mfu_ghost.push_front(3);

        replacer.record_access(1, 1);

        assert_eq!(replacer.mru_target_size, 2);
        assert!(replacer.mfu.contains(&(1, 1)));
    }

    #[test]
    fn mfu_hit_decrease_target_size_by_one() {
        let mut replacer = ArcReplacer::new(10);
        replacer.mru_ghost.push_front(2);
        replacer.mfu_ghost.push_front(1);

        replacer.mru_target_size = 3;
        replacer.record_access(1, 1);

        assert_eq!(replacer.mru_target_size, 2);
        assert!(replacer.mfu.contains(&(1, 1)));
    }

    #[test]
    fn mfu_hit_decrease_target_size_by_mod() {
        let mut replacer = ArcReplacer::new(10);
        
        replacer.mru_ghost.push_front(1);
        replacer.mru_ghost.push_front(2);
        replacer.mfu_ghost.push_front(3);

        replacer.mru_target_size = 2;
        replacer.record_access(3, 3);

        assert_eq!(replacer.mru_target_size, 0);
        assert!(replacer.mfu.contains(&(3, 3)));
    }

    #[test]
    fn mru_mru_ghost_eq_replacer_size() {
        let mut replacer = ArcReplacer::new(4);

        replacer.mru.push_front((1, 1));
        replacer.mru.push_front((2, 2));
        replacer.mru_ghost.push_front(3);
        replacer.mru_ghost.push_front(4);


        replacer.record_access(5, 5);


        assert!(replacer.mru.contains(&(5, 5)));
        assert_eq!(replacer.mru_ghost.contains(&3), false);

    }

    #[test]
    fn mru_all_less_than_replacer_size_and_all_full() {
        let mut replacer = ArcReplacer::new(4);

        replacer.mru.push_front((1, 1));
        replacer.mfu.push_front((2, 2));
        replacer.mru_ghost.push_front(3);
        replacer.mfu_ghost.push_front(4);

        replacer.record_access(5, 5);

        assert_eq!(replacer.mfu_ghost.contains(&5), false);
        assert!(replacer.mru.contains(&(5, 5)));
        assert_eq!(replacer.mru_ghost.contains(&3), true);
        assert_eq!(replacer.mfu.contains(&(2, 2)), true);
        assert_eq!(replacer.mru_target_size, 0);
    }

    #[test]
    fn mru_all_less_than_replacer_size_and_not_full() {
        let mut replacer = ArcReplacer::new(9);
        replacer.mru.push_front((1, 1));
        replacer.mfu.push_front((2, 2));
        replacer.mru_ghost.push_front(3);
        replacer.mfu_ghost.push_front(4);

        replacer.record_access(5, 5);

        assert_eq!(replacer.mru.contains(&(1, 1)), true);
        assert_eq!(replacer.mru.contains(&(5, 5)), true);
        assert_eq!(replacer.mfu.contains(&(2, 2)), true);
        assert_eq!(replacer.mru_ghost.contains(&3), true);
        assert_eq!(replacer.mfu_ghost.contains(&4), true);
    }
}
// ============================================================================
// END OF SECTION 1:  ARC REPLACER
// ============================================================================


// ============================================================================
// SECTION 2: DISK SCHEDULER
// ============================================================================
// Schedules disk read/write operations asynchronously
// Uses background worker thread to process queued requests
// ============================================================================
enum DiskRequestType {
    Read,
    Write,
    Delete,
}

struct DiskRequest {
    r#type: DiskRequestType,
    promise: mpsc::Sender<Option<Vec<u8>>>,
    data: Vec<u8>,
    page_id: PageId,
}

struct DiskScheduler {
    request_tx: mpsc::Sender<Option<DiskRequest>>,
    disk_manager: Arc<Mutex<DiskManager>>,
    worker_handle: Option<thread::JoinHandle<()>>,
}

impl DiskScheduler {
    fn new(disk_manager: Arc<Mutex<DiskManager>>) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<Option<DiskRequest>>();
        let dm = Arc::clone(&disk_manager);

        let worker_handle = thread::spawn(move || {
            while let Ok(Some(mut request)) = request_rx.recv() {
                let mut dm = dm.lock().unwrap();

                match request.r#type {
                    DiskRequestType::Read => {
                        match dm.read_page(request.page_id, &mut request.data[..])  {
                            Ok(_) => { let _ = request.promise.send(Some(request.data));}
                            Err(_) => { let _ = request.promise.send(None);}
                        }
                    }
                    DiskRequestType::Write => {
                        match dm.write_page(request.page_id, &request.data[..]) {
                            Ok(_) => { let _ = request.promise.send(Some(request.data));}
                            Err(_) => { let _ = request.promise.send(None);}
                        }
                    }
                    DiskRequestType::Delete => {
                        match dm.delete_page(request.page_id) {
                            Ok(_) => { let _ = request.promise.send(Some(request.data));}
                            Err(_) => { let _ = request.promise.send(None);}
                        }
                    }
                }
            }
        });

        Self {
            request_tx,
            disk_manager,
            worker_handle: Some(worker_handle),
        }
    }

    fn schedule(&self, request: DiskRequest) {
        self.request_tx.send(Some(request)).unwrap();
    }
}

impl Drop for DiskScheduler {
    fn drop(&mut self) {
        let _ = self.request_tx.send(None);

        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod disk_scheduler_tests {
    use super::*;

    #[test]
    fn test_basic_read_write_operations() {
        let test_file = "test.db";
        let _ = remove_file(test_file);
        let dm = Arc::new(Mutex::new(DiskManager::new(test_file)));
        let scheduler = DiskScheduler::new(dm);


        let page_data = b"Hello world".repeat(PAGE_SIZE / 10);
        let page_data = &page_data[..PAGE_SIZE];

        let (tx1, rx1 ) = mpsc::channel::<Option<Vec<u8>>>();
        let req1 = DiskRequest {
           r#type: DiskRequestType::Write,
           page_id: 0,
           data: page_data.to_vec(),
           promise: tx1,
        };

        scheduler.schedule(req1);
        assert_eq!(rx1.recv().unwrap(), Some(page_data.to_vec()));

        let (tx2, rx2) = mpsc::channel::<Option<Vec<u8>>>();
        let req2 = DiskRequest {
            r#type: DiskRequestType::Read,
            page_id: 1,
            data: page_data.to_vec(),
            promise: tx2,
        };

        scheduler.schedule(req2);
        assert_eq!(rx2.recv().unwrap(), Some(page_data.to_vec()));

        let _ = remove_file(test_file);
    }
}
// ============================================================================
// END OF SECTION 2: DISK SCHEDULER
// ============================================================================


// ============================================================================
// SECTION 3: DISK MANAGER
// ============================================================================
// Handles file I/O operations for database pages
// Reads/writes 8KB pages to/from disk files
// Manages page allocation and free slot reuse
// ============================================================================

struct DiskManager {
    db_file: Mutex<File>,
    page_capacity: usize,
    page_size: usize,
    pages: Mutex<HashMap<PageId, u64>>,
    num_writes: Mutex<u64>,
    num_deletes: Mutex<u64>,
    free_slots: Vec<u64>,
}

/**
 *  Implement DiskManager
 * File I/O: File, OpenOptions, read(), write(), seek()
 * Start simple: open a file, write a page, read it back
 * Add page allocation: track offsets, reuse deleted slots
 * Test: verify you can read/write pages correctly
 */
impl DiskManager {
    fn new(file_path: &str) -> Self {
        
        let db_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(file_path).unwrap();
        
        let page_capacity = DB_IO_SIZE;
        let page_size = PAGE_SIZE;
    
        let _ = db_file.set_len(((page_capacity + 1) * page_size) as u64);

        let metadata = db_file.metadata().unwrap();

        if metadata.len() < (page_capacity * page_size) as u64 {
            panic!("Error Database file is too large");
        }

        Self {
            db_file: Mutex::new(db_file),
            pages: Mutex::new(HashMap::new()),
            num_writes: Mutex::new(0),
            page_capacity: DB_IO_SIZE,
            page_size: PAGE_SIZE,
            free_slots: Vec::new(),
            num_deletes: Mutex::new(0),
        }
    }

    fn write_page(&mut self, page_id: PageId, page_data: &[u8]) -> Result<(), std::io::Error> {
        let mut file = self.db_file.lock().unwrap();
        let offset =  {
            let pages = self.pages.lock().unwrap();
            if let Some(&existing_offset) = pages.get(&page_id) {
                existing_offset
            } else {
                if !self.free_slots.is_empty() {
                    let new_offset: u64 = { 
                       if let Some(value) = self.free_slots.pop() {
                            value
                       } else {
                        println!("No free slots available");
                        0
                       }
                    };

                    new_offset
                }  else {
                    if pages.len() + 1 >= self.page_capacity {
                        self.page_capacity *= 2;
                        let _ = file.set_len(((self.page_capacity + 1) * PAGE_SIZE) as u64);
                    }
                    let new_offset: u64 = (pages.len() * PAGE_SIZE) as u64;

                    new_offset
                }
            }
        };

        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&page_data[..PAGE_SIZE])?;

        *self.num_writes.lock().unwrap() += 1;
        self.pages.lock().unwrap().insert(page_id, offset);

        file.flush()?;

        Ok(())
    }

    fn read_page<'a>(&mut self, page_id: PageId, page_data: &'a mut[u8]) -> Result<&'a[u8], std::io::Error> {
        let mut file = self.db_file.lock().unwrap();
        let offset = {
            let pages = self.pages.lock().unwrap();
            if let Some(&existing_offset) = pages.get(&page_id) {
                existing_offset
            } else {
                if !self.free_slots.is_empty() {
                    let new_offset: u64 = { 
                       if let Some(value) = self.free_slots.pop() {
                            value
                       } else {
                        println!("No free slots available");
                        0
                       }
                    };

                    new_offset
                } else {          
                    if pages.len() + 1 >= self.page_capacity {
                        self.page_capacity *= 2;
                        let _ = file.set_len(((self.page_capacity + 1) * PAGE_SIZE) as u64);
                    }
                    let new_offset: u64 = (pages.len() * PAGE_SIZE) as u64;
                    new_offset
                }
            }
        };

        let file_size = match file.metadata() {
            Ok(metadata) => metadata.len(),
            Err(e) => {
                eprintln!("Error getting file size: {}", e);
                return Err(e);
            }
        };

        if offset > file_size {
            panic!("Page out of bounds");
        }

        file.seek(SeekFrom::Start(offset))?;
        match file.read_exact(&mut page_data[..PAGE_SIZE]) {
            Ok(_) => {
                return Ok(page_data)
            }

            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                eprintln!("I/O error: read page {} hit end of file  at offset {}", page_id, offset);
                return Ok(page_data)
            }

            Err(e) => {
                return Err(e)
            }
        }
    }

    fn delete_page(&mut self, page_id: PageId) -> Result<(), std::io::Error> {
        let mut pages = self.pages.lock().unwrap();

        match pages.remove(&page_id) {
            Some(offset) => {
                self.free_slots.push(offset);
                *self.num_deletes.lock().unwrap() += 1;
            } 
            None => {
                println!("Page {} not found", page_id);
            }
        }
    
        Ok(())
    }
}

#[cfg(test)]
mod disk_manager_tests {
    use super::*;

    #[test]
    fn test_basic_write_read_delete_operations() {
        let test_file = "test.db";
        let _ = remove_file(test_file);

        let mut dm = DiskManager::new(test_file);

        let page_data = b"Hello world".repeat(PAGE_SIZE / 10);
        let page_data = &page_data[..PAGE_SIZE];

        dm.write_page(0, page_data).unwrap();

        assert_eq!(*dm.num_writes.lock().unwrap(), 1);

        let mut buffer = vec![0u8; PAGE_SIZE];
        dm.read_page(0, &mut buffer).unwrap();

        assert_eq!(dm.free_slots.len(), 0);
        assert_eq!(&page_data[..1], &buffer[..1]);

        dm.delete_page(0).unwrap();

        assert_eq!(*dm.num_deletes.lock().unwrap(), 1);
        assert_eq!(dm.free_slots.len(), 1);

        assert_eq!(dm.page_capacity, DB_IO_SIZE);

        let _ = remove_file(test_file);
    }
    
    #[test]
    fn test_use_existing_slot() {
        let test_file =  "test.db";
        let _ = remove_file(test_file);

        let mut dm = DiskManager::new(test_file);

        let page_data = b"Hello world".repeat(PAGE_SIZE / 10);
        let page_data = &page_data[..PAGE_SIZE];
        dm.write_page(1, page_data).unwrap();
        dm.write_page(2, page_data).unwrap();
        dm.write_page(3, page_data).unwrap();
        dm.write_page(4, page_data).unwrap();
        dm.write_page(5, page_data).unwrap();

        assert_eq!(*dm.num_writes.lock().unwrap(), 5);
        let third_offset = {
            let binding = dm.pages.lock().unwrap();
            binding.get(&3).unwrap().clone()
        };

        dm.delete_page(3).unwrap();
        
        assert_eq!(dm.free_slots.contains(&third_offset), true);
        assert_eq!(dm.free_slots.len(), 1);
    
        dm.write_page(9, page_data).unwrap();

        let ninth_offset = {
            let binding = dm.pages.lock().unwrap();
            binding.get(&9).unwrap().clone()
        };
        assert_eq!(dm.free_slots.len(), 0);
        assert_eq!(third_offset, ninth_offset);

        let second_offset = {
            let binding = dm.pages.lock().unwrap();
            println!("binding: {:?}", binding);
            binding.get(&2).unwrap().clone()
        };
        assert_ne!(second_offset, third_offset);

        let _ = remove_file(test_file);
    }

    #[test]
    fn test_increase_page_capacity() {
        let test_file = "test.db";
        let _ = remove_file(test_file);

        let mut dm = DiskManager::new(test_file);

        dm.page_capacity = 3;

        let page_data = b"Hello world".repeat(PAGE_SIZE / 10);
        let page_data = &page_data[..PAGE_SIZE];

        dm.write_page(1, page_data).unwrap();
        dm.write_page(2, page_data).unwrap();
        dm.write_page(3, page_data).unwrap();

        assert_eq!(dm.page_capacity, 6);
        assert_eq!(dm.db_file.lock().unwrap().metadata().unwrap().len(), 57344);

        let _ = remove_file(test_file);
    }
}

// ============================================================================
// END OF SECTION 3: DISK MANAGER
// ============================================================================


// ============================================================================
// SECTION 4: BUFFER POOL MANAGER
// ============================================================================
// Manages the buffer pool and the ARC replacer
// ============================================================================

struct FrameHeader {
    frame_id: FrameId,
    pin_count: AtomicUsize,
    is_dirty: AtomicBool, // if false means no thread is currently using this frame its safe to evict or reuse
    data: RwLock<Vec<u8>>,
}

impl FrameHeader {
    fn new(frame_id: FrameId) -> Self {
        Self {
            frame_id,
            pin_count: AtomicUsize::new(0),
            is_dirty: AtomicBool::new(false),
            data: RwLock::new(vec![0u8; PAGE_SIZE]),
        }
    }

    fn reset(& self) {
        self.data.write().unwrap().fill(0);
        // TODO: check if we can use Relaxed here and if it will affect the correctness of the program
        self.pin_count.store(0, Ordering::SeqCst);
        self.is_dirty.store(false, Ordering::SeqCst);
    }
}

// encapsulate the frames operations and lock it
struct WritePageGuard {
    page_id: PageId,
    frame: Arc<FrameHeader>,
    arc_replacer: Arc<Mutex<ArcReplacer>>,
    bpm_latch: Arc<Mutex<()>>,
    disk_scheduler: Arc<DiskScheduler>,
    is_valid: bool,
}

impl WritePageGuard {
    fn new(
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

    fn flush(&self) -> bool {
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

struct ReadPageGuard {
    page_id: PageId,
    frame: Arc<FrameHeader>,
    arc_replacer: Arc<Mutex<ArcReplacer>>,
    bpm_latch: Arc<Mutex<()>>,
    disk_scheduler: Arc<DiskScheduler>,
    is_valid: bool,
}

impl ReadPageGuard {

    fn new(
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

    fn data(&self) -> LockResult<RwLockReadGuard<'_, Vec<u8>>> {
        return self.frame.data.read()
    }

    fn flush(&self) -> bool {
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
/**
 * To Implement BufferPoolManager i need to
 *  - Read about RaII , Page Guards Write and Read then implement this X
 *  - Read frame manager understand the role plan and implement a basic one and enahance it while implementing buffer pool manager X
 *  - understand the conccuurency issues that we might face and plan how to solve it X
 *  - Implement buffer pool manager with the frame manager and the disk manager
 */
struct BufferPoolManager {
    num_frames: usize,
    next_page_id: AtomicU32,
    arc_replacer: Arc<Mutex<ArcReplacer>>,
    disk_scheduler: Arc<DiskScheduler>,
    frames: Vec<Arc<FrameHeader>>,
    free_frames: Vec<FrameId>,
    latch: Arc<Mutex<()>>,
    page_table: HashMap<PageId, FrameId>,
}

/**
 * functions implementation order 
 * 1. GetPinCount() X
 * 2. NewPage() X
 * 3. ReadPageGuard
 * 4. WritePageGuard
 * 5. CheckedReadPage()
 * 6. CheckedWritePage()
 * 7. DeletePage()
 * 8. FlushPage()
 */
impl BufferPoolManager {
    fn new(num_frames: usize, disk_manager: Arc<Mutex<DiskManager>>) -> Self {
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

    fn get_pin_count(&self,  page_id: PageId) -> Option<usize> {
        match self.page_table.get(&page_id) {
            Some(frame_id) => {
                Some(self.frames[*frame_id as usize].pin_count.load(Ordering::SeqCst))
            }
            None => {
                None
            }
        }
    }

    fn new_page(&self) -> PageId {
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
 
    fn delete_page(&mut self, page_id: PageId) { 
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
 
    fn check_write_page(&mut self, page_id: PageId) -> Option<WritePageGuard>{
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

    fn check_read_page(&mut self, page_id: PageId) -> Option<ReadPageGuard> {
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

    fn flush_page(&mut self, page_id: PageId) -> bool {
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

    fn flush_all_pages(&mut self) -> bool {
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
        assert_eq!(bpm.get_pin_count(page_id).unwrap(), 2);

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

    #[test]
    fn test_multiple_threads_read_write_delete_operations() {
        let bpm = BufferPoolManager::new(10, Arc::new(Mutex::new(DiskManager::new("test.db"))));
        // create multiple pages 
        println!("bpm {:?}", bpm.page_table.len());
        // read and wirte delete operations on different threads
    }
}
// ============================================================================
// END OF SECTION 4: BUFFER POOL MANAGER
// ============================================================================


// ============================================================================
// MAIN
// ============================================================================

fn main() {
}
