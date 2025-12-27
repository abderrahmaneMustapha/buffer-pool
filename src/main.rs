use std::collections::VecDeque;
use std::collections::HashSet;
use std::collections::HashMap;
use std::time::SystemTime;

type PageId = u32;
type FrameId = u32;

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


// TESTS
mod tests {
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


// MAIN no need for this for now but yes just keep here 
fn main() {
}
