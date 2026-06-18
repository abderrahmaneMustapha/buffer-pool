
// ============================================================================
// B+Tree 
// a balanced search data structure
// ============================================================================
use crate::buffer_pool::BufferPoolManager;
use crate::disk_manager::DiskManager;
use crate::common::PageId;
use std::sync::{Arc, Mutex};

const DEFAULT_LEAF_NODE_MAX_SIZE: u32 = 511;
const DEFAULT_INNER_NODE_MAX_SIZE: u32 = 681;

type Key = String;
type RecordId = (PageId, u32); // page id and slot number

struct LeafPage {
    keys: Vec<Key>,
    values: Vec<RecordId>,
}

struct InnerPage {
    keys: Vec<Key>,
    values: Vec<PageId>,
}

pub struct BTree {
    root_page_id: PageId,
    buffer_pool: BufferPoolManager,
    max_leaf_node_size: u32,
    max_inner_node_size: u32,
}

impl BTree {
    pub fn new(buffer_pool: BufferPoolManager) -> Self {
        Self {
            buffer_pool,
            root_page_id: 0,
            max_leaf_node_size: DEFAULT_LEAF_NODE_MAX_SIZE,
            max_inner_node_size: DEFAULT_INNER_NODE_MAX_SIZE,
        }
    }

    pub fn insert(&mut self, key: &str) -> () {
       
    }
}

#[cfg(test)]
mod b_plus_tree_testing {
    use super::*;

    #[test]
    fn basic_insert() {
        let mut disk_manager = Arc::new(Mutex::new(DiskManager::new("test.db")));
        let buffer_pool = BufferPoolManager::new(1000, disk_manager);
        let bTree = BTree::new(buffer_pool);
        bTree.insert()
        
    }
}
