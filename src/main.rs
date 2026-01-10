use std::collections::{ HashMap, HashSet, VecDeque };
use std::time::SystemTime;
use std::fs::{ remove_file, OpenOptions, File};
use std::sync::Mutex;
use std::io::{self, Read, Write, SeekFrom, Seek };

type PageId = u32;
type FrameId = u32;

const PAGE_SIZE: usize = 8192;
const DB_IO_SIZE: usize = 16;

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

/**
 * Implement DiskScheduler
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
    r#type: DisRequestType,
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

    fn schedule(&self, _: Vec<DiskRequest>) {
        // schedule a vector of requests for the disk manager to execute
    }

    fn start_worker_thread(&self) {
        // start the worker thread
        // worker thread created in disk Scheduler constructor
        // receive queued requests  => dispatch to disk manager
    }

    // signal that the request is completed
}

struct DiskManager {
    db_file: Mutex<File>,
    page_capacity: usize,
    page_size: usize,
    pages: Mutex<HashMap<PageId, u64>>,
    num_writes: Mutex<u64>,
    free_slots: Vec<u64>,
}

/**
 *  Implement DiskManager first
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
    
        db_file.set_len(((page_capacity + 1) * page_size) as u64);

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
        }
    }

    fn write_page(&mut self, page_id: PageId, page_data: &[u8]) -> Result<(), std::io::Error> {
        let mut file = self.db_file.lock().unwrap();
        let offset =  {
            let mut pages = self.pages.lock().unwrap();
            if let Some(&existing_offset) = pages.get(&page_id) {
                existing_offset
            } else {
                let mut new_offset: u64 = 0;
                if self.free_slots.is_empty() {
                    new_offset = { 
                       if let Some(value) = self.free_slots.pop() {
                            value
                       } else {
                        println!("No free slots available");
                        0
                       }
                    };
                }
        
                if pages.len() + 1 >= self.page_capacity {
                    self.page_capacity *= 2;
                    file.set_len(((self.page_capacity + 1) * PAGE_SIZE) as u64);
                    new_offset = (pages.len() * PAGE_SIZE) as u64
                    
                }
                pages.insert(page_id, new_offset);
                new_offset
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
            let mut pages = self.pages.lock().unwrap();
            if let Some(&existing_offset) = pages.get(&page_id) {
                existing_offset
            } else {
                let mut new_offset: u64 = 0;
                if self.free_slots.is_empty() {
                    new_offset = { 
                       if let Some(value) = self.free_slots.pop() {
                            value
                       } else {
                        println!("No free slots available");
                        0
                       }
                    };
                }
        
                if pages.len() + 1 >= self.page_capacity {
                    self.page_capacity *= 2;
                    file.set_len(((self.page_capacity + 1) * PAGE_SIZE) as u64);
                    new_offset = (pages.len() * PAGE_SIZE) as u64
                }
                pages.insert(page_id, new_offset);
                new_offset
            }
        };

        let file_size = file.metadata().unwrap().len();

        if file_size < 0 {
            panic!("Failed to get file size");
        }

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
}

mod dis_manager_tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let test_file = "test.db";
        let _ = remove_file(test_file);

        let mut dm = DiskManager::new(test_file);

        let page_data = b"Hello world".repeat(PAGE_SIZE / 10);
        let page_data = &page_data[..PAGE_SIZE];

        dm.write_page(0, page_data).unwrap();

        let mut buffer = vec![0u8; PAGE_SIZE];
        dm.read_page(0, &mut buffer).unwrap();

        assert_eq!(&page_data[..1], &buffer[..1]);

        let _ = remove_file(test_file);
    }
}

// MAIN no need for this for now but yes just keep here 
fn main() {
}
