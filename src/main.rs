use std::collections::VecDeque;

type PageId = u32;
type FrameId = u32;

struct ArcReplacer {
    mru: VecDeque<(FrameId, PageId)>, // most recently used list
    mfu: VecDeque<(FrameId, PageId)>, // most frequently used list

    mru_ghost: VecDeque<PageId>, // most recently used evicted from buffer pool
    mfu_ghost: VecDeque<PageId>, // most frequently used evicted from buffer pool

    mru_target_size: u32, // target size of the MRU list , the actual size of MRU list can be different than the this

    replacer_size: usize, // the maximum number of the frames that ArcReplacer support, the same size as the buffer pool

    curr_size: u32, // current size of evictable frames, init with 0 and incrase when a frame is marked as evictable, frame is not in use or pinned decrease
}

impl ArcReplacer {
    fn new(replacer_size: usize) -> Self {
        Self {
            mru: VecDeque::new(),
            mfu: VecDeque::new(),
            mru_ghost: VecDeque::new(),
            mfu_ghost: VecDeque::new(),
            mru_target_size: 0,
            replacer_size: replacer_size,
            curr_size: 0
        }
    }
    fn size(&self) -> u32 {
        // TODO: return number of evictable frames
        return 0;
    }
 
    fn setEvictable(&self, frame_id: u32, set_evictable: bool) {
        // TODO: set the evictable status of the frame
    }
    
    fn RecordAccess(frame_id: u32, page_id: u32) {
        // TODO: record that page is accessed on specific timestamp in the given frame
        // called after a page has been pinned into a frame
    }

    fn Evict(&self) -> u32 {
        // TODO: evict a frame and return the frame id or a None
        // following the eviction process of the ARC algorithm , no evictable frame return None
        return 0;
    }

    fn Remove(frame_id: u32) {
        // TODO: remove the frame  and its corresponding page from the replacer
        // only called when a page is deleted from the buffer pool manger
    }
}

struct RecrodAccess {
    arc_replacer: ArcReplacer,
}

impl RecrodAccess {
    fn new(arc_replacer: ArcReplacer) -> Self {
        Self {
            arc_replacer,
        }
    }

    fn add(&mut self, frame_id: FrameId, page_id: PageId) -> () {
        if (
            self.arc_replacer.mru.contains(&(frame_id, page_id)) ||
            self.arc_replacer.mfu.contains(&(frame_id, page_id)) ||
            self.arc_replacer.mru_ghost.contains(&page_id) ||
            self.arc_replacer.mfu_ghost.contains(&page_id)
        ) {
            return ();
        }

        if (self.arc_replacer.mru.len() + self.arc_replacer.mru_ghost.len() == self.arc_replacer.replacer_size) {
            self.arc_replacer.mru_ghost.pop_back();
            self.arc_replacer.mru.push_front((frame_id, page_id));
        }

        if (self.arc_replacer.mru.len() + self.arc_replacer.mru_ghost.len() < self.arc_replacer.replacer_size) {
            if (
                self.arc_replacer.mru.len() + 
                self.arc_replacer.mfu.len() + 
                self.arc_replacer.mfu_ghost.len() + 
                self.arc_replacer.mru_ghost.len() ==
                2 * self.arc_replacer.replacer_size
            ) {
                self.arc_replacer.mfu_ghost.pop_back();
                self.arc_replacer.mru.push_front((frame_id, page_id));
            } else {
                self.arc_replacer.mru.push_front((frame_id, page_id));
            }
        }
    }
}

fn main() {
   let arc_replacer = ArcReplacer::new(200);
   let record_access = RecrodAccess::new(arc_replacer);
}
