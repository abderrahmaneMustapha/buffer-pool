use std::collections::VecDeque;
use std::collections::HashSet;
use std::collections::HashMap;
use std::time::SystemTime;
use std::fs::OpenOptions;
use std::sync::Mutex;

type PageId = u32;
type FrameId = u32;

const PAGE_SIZE: usize = 8192;

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
                if self.mru_target_size - decrease >= 0 {
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


    // UNDERSTAND THE WHY AND HOW BEHIND THIS FUNCTION
    // not sure wtf this does lets keep it, later lets read more and understand how to it works
    // implemented this translating english to rust
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

// Arc replacer TESTS
mod arcReplacerTests {
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

/**
 *
 * Step 3: Implement DiskScheduler
 * Create channel: mpsc::channel()
 * Background thread: spawn worker thread
 * Process requests: worker receives from channel, calls DiskManager
 * Promise equivalent: use Arc<Mutex<bool>> or oneshot channel
 * Test: queue requests, verify they complete
 */

enum DisRequestType {
    Read,
    Write,
}

struct DiskRequest {
    type: DisRequestType,
    promise: bool,
    page_id: PageId,
}

struct DiskScheduler {

}

impl DiskScheduler {
    // queue disk requests
    // shared queue to schedule and process disk requests
    // thread add request to the queue
    // disk background worker will process queued requests
    // thread safety please
    // a constructor and destrcutor implemented for creating and joining the background worker thread

    schedule(&self, requets: Vec<DiskRequest>) {
        // schedule a vector of requests for the disk manager to execute
    }

    start_worker_thread(&self) {
        // start the worker thread
        // worker thread created in disk Scheduler constructor
        // receive queued requests  => dispatch to disk manager
    }

    // signal that the request is completed
}

struct DiskManager {
    db_file: Mutex<File>,
    log_file: File,
    page_capacity: u64,
    page_size: u64,
    buffer_used: Option<*const u8>,
    pages: Mutex<HashMap<PageId, u64>>,
    num_writes: Mutex<u64>,
}

/**
 * 
 * Step 2: Implement DiskManager first
 * File I/O: File, OpenOptions, read(), write(), seek()
 * Start simple: open a file, write a page, read it back
 * Add page allocation: track offsets, reuse deleted slots
 * Test: verify you can read/write pages correctly
 */
impl DiskManager {
    /**
     * Part 1: Basic file operations
     * Create struct with a File handle
     * Implement new(): open/create database file
     * Implement write_page(): write 8KB to file at calculated offset
     * Implement read_page(): read 8KB from file at calculated offset
     * Test: write a page, read it back, verify data matches
     * 
     * Part 2: Page tracking
     * Add HashMap<PageId, u64> to track page_id → file offset
     * Update write_page(): store offset in map
     * Update read_page(): look up offset from map
     * Test: write multiple pages, read them back
     * Part 3: Page allocation
     * Add Vec<u64> for free slots (deleted pages)
     * Update allocation: reuse free slots first, then append to end
     * Add delete_page(): mark slot as free
     * Test: delete a page, allocate new page, verify it reuses the slot
     */

    fn new(file_path: &str) {
        let log_file_name = format!("{}.log", file_path);

        _log_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            open(log_file_name)?;

        _db_file = Options::new()
            .read(true)
            .write(true)
            .create(true)
            .open(file_path)?;
        
        _db_file_.set_len((self.page_capacity + 1) * self.page_size)

        let metadata = _db_file_.metadat()?;

        if(metadata.len() >= self.page_capacity * self.page_size) {
            panic!("Error Database file is too large");
        }

        
        Self {
            db_file: Mutex::new(_db_file),
            log_file: _log_file,
            buffer_used: None,
        }
    }

    fn write_page(page_id: PageId, page_data: &[u8]) {
        let mut file = self.db_file.lock();
        let offset =  {
            let mut pages = self.pages.lock().unwrap();
            pages.get(&page_id)
                .copied()
                .unwrap_or_else( || {
                    let new_offset = self.allocate_page(&mut file)?;
                    pages.insert(page_id, new_offset);
                    new_offset
                })
        };

        file.seek(SeekFrom::Start(offset))?;
        file.write_all(page_data[..PAGE_SIZE])?;

        *self.num_writes.lock().unwrap() += 1;
        self.pages.lock().unwrap().insert(page_id, offset);

        file.flush()?;

        ok(())
    }

    fn read_page(page_id: PageId, page_data: &[u8]) {
        let mut file = self.db_file.lock();
        let offset = {
            let mut pages = self.pages.lock().unwrap();
            pages.get(&page_id)
                .copied()
                unwrap_or_else( || {
                    let new_offset = self.allocate_page(&mut file)?;
                    pages.insert(page_id, new_offset);
                    new_offset
                })
        };

        let file_size = self.get_file_size(&self.db_file_name)?;

        if file_size < 0 {
            panic!("Failed to get file size");
        }

        if offset > file_size {
            panic!("Page out of bounds");
        }

        file.seek(SeekFrom::Start(offset))?;
        match file.read_exact(&mut page_data[..PAGE_SIZE]) {
            Ok(_) => Ok(page_data)

            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                eprintln!("I/O error: read page {} hit end of file  at offset {}", page_id, offset);
                Ok(page_data)
            }
        }
        Ok(page_data)
    }

    fn get_file_size(file_name: &str) -> Result<u64, std::io::Error> {
        let metadata = fs::metadata(filename)?;
        Ok(metadata.len())
    }
}

// MAIN no need for this for now but yes just keep here 
fn main() {
}
