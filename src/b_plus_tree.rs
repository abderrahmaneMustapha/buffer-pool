
// ============================================================================
// B+Tree 
// a balanced search data structure
// ============================================================================
use crate::buffer_pool::BufferPoolManager;
use crate::disk_manager::DiskManager;
use crate::common::{PageId, INVALID_PAGE_ID};
use std::sync::{Arc, Mutex};

const DEFAULT_LEAF_NODE_MAX_SIZE: u32 = 511;
const DEFAULT_INTERNAL_NODE_MAX_SIZE: u32 = 681;
const HEADER_SIZE: usize = 16;
const LEAF_NODE: u8 = 1;
const INTERNAL_NODE: u8 = 0;
// this is a temporary default slot number placeholder until we build a the table page
const DEFAULT_SLOT_NUMBER: u32 = 0;

type Key = i64;
const INVALID_KEY: Key = i64::MIN;

enum TreeNode {
    Leaf(LeafNode),
    Internal(InternalNode)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordId {
    pub page_id: PageId,
    pub slot_num: u32,
}

type LeafKv = (Key, RecordId);
type InternalKv = (Key, PageId);

struct LeafNode {
    next_page_id: PageId,
    entries: Vec<LeafKv>
}

struct InternalNode {
    entries: Vec<InternalKv>
}

pub struct BTree {
    header_page_id: PageId, 
    root_page_id: PageId,
    buffer_pool: BufferPoolManager,
    leaf_node_max_size: u32,
    internal_node_max_size: u32,
}

 
impl LeafNode {

    fn decode(bytes: &[u8]) -> LeafNode {
        // get current size from the 4th to 8th bytes
        let current_size = u32::from_le_bytes(bytes[4 .. 8].try_into().unwrap());
        // then get the next page id from 12th to 16th bytes
        let next_page_id = u32::from_le_bytes(bytes[12 .. 16].try_into().unwrap());

        // declare a vector to contain data
        let mut entries = Vec::with_capacity(current_size as usize);
        // start from the 16th bit which is the header size  a bit where the header size ends
        let mut off = HEADER_SIZE;

        // it is time now to loop through the slots that contains the real data of the node
        for _ in 0 .. current_size {
        // inside the loop do the following
          // - get the key from start jump with 8 bytes => we are expecting this to be big
          let key = i64::from_le_bytes(bytes[off .. off+8].try_into().unwrap());
          // - get the pid from jump with 4 bytes => with this 32 bits we can go up  to 4 bilion so that is great
          let page_id = u32::from_le_bytes(bytes[off+8 .. off+12].try_into().unwrap());
          // - get the slot jump with 4 bytes => same here 4 bilion slots for a one page
          let slot_num = u32::from_le_bytes(bytes[off+12 .. off+16].try_into().unwrap());
          // add the data to the array
          entries.push((key, RecordId {page_id, slot_num}));
          // advnace with 16 bytes
          off += 16
        }

        // return a Leaf node with next_page_id and the array of data
        LeafNode { next_page_id, entries }
    }

    fn encode(&self, bytes: &mut [u8]) {
        bytes[0] = 1; // a leaf page
        // we are using copy_from_slice where the two side must fit in the same size 
        // and also it should  have implement the Copy trait
        bytes[4 .. 8].copy_from_slice(&(self.entries.len() as u32).to_le_bytes());
        bytes[8 .. 12].copy_from_slice(&DEFAULT_LEAF_NODE_MAX_SIZE.to_le_bytes());
        bytes[12 .. 16].copy_from_slice(&self.next_page_id.to_le_bytes());

        let mut off = HEADER_SIZE;
        for (key, rid) in &self.entries {
            bytes[off .. off + 8].copy_from_slice(&key.to_le_bytes());
            bytes[off + 8 .. off + 12].copy_from_slice(&rid.page_id.to_le_bytes());
            bytes[off + 12 .. off + 16].copy_from_slice(&rid.slot_num.to_le_bytes());
            off += 16
        }
    }
}


impl InternalNode {

    fn decode(bytes: &[u8]) -> InternalNode {
        let current_size = u32::from_le_bytes(bytes[4 .. 8].try_into().unwrap());

        let mut entries = Vec::with_capacity(current_size as usize);
        let mut off = HEADER_SIZE;

        for _ in 0 .. current_size {
            let key = i64::from_le_bytes(bytes[off .. off+8].try_into().unwrap());
            let pid = u32::from_le_bytes(bytes[off+8 .. off+12].try_into().unwrap());

            entries.push((key, pid));
            off += 12
        }

        InternalNode { entries }
    }


    fn encode(&self, bytes: &mut [u8]) {
        bytes[0] = 0;

        bytes[4 .. 8].copy_from_slice(&(self.entries.len() as u32).to_le_bytes());
        bytes[8 .. 12].copy_from_slice(&DEFAULT_INTERNAL_NODE_MAX_SIZE.to_le_bytes());

        // skip the first slot in the internal node
        let mut off = HEADER_SIZE;

        for (key, pid) in &self.entries {
            bytes[off .. off + 8].copy_from_slice(&key.to_le_bytes());
            bytes[off + 8 .. off + 12].copy_from_slice(&pid.to_le_bytes());

            off += 12
        }
    }

    fn find_child_index(&self, key: Key) -> usize {
        let mut low: usize = 0;
        let mut high: usize = self.entries.len();

        while low < high {
            let mid = low + (high - low) / 2;

            if self.entries[mid].0 <= key {
                low = mid + 1;
            } else {
                high = mid;
            }
        }

        low - 1
    }
}

impl BTree {

    pub fn new() -> Self {
        let buffer_pool = BufferPoolManager::new(10, Arc::new(Mutex::new( DiskManager::new("main.db"))));
        let header_page_id = buffer_pool.new_page();

        Self {
            header_page_id,
            buffer_pool,
            root_page_id: INVALID_PAGE_ID,
            leaf_node_max_size: DEFAULT_LEAF_NODE_MAX_SIZE,
            internal_node_max_size: DEFAULT_INTERNAL_NODE_MAX_SIZE,
        }
    }

    fn set_leaf_max_size(&mut self, leaf_node_max_size: u32) {
        self.leaf_node_max_size = leaf_node_max_size;
    }

    fn set_internal_max_size(&mut self, internal_node_max_size: u32) {
        self.internal_node_max_size = internal_node_max_size;
    }

    fn get_root_page_id(&mut self, bytes: &[u8]) -> PageId {
        u32::from_le_bytes(bytes[0 .. 4].try_into().unwrap())
    }

    fn set_root_page_id(&self , bytes: &mut[u8]) {
        bytes[0 .. 4].copy_from_slice(&self.root_page_id.to_le_bytes());
    }

    fn get_child(&mut self, child: InternalNode, key: Key) -> TreeNode {
        let child_index = child.find_child_index(key);
        let child_page_id = child.entries[child_index].1;

        let page_guard = self.buffer_pool.check_read_page(child_page_id).unwrap();
        let page_data = page_guard.data().unwrap();
        let node_type = page_data[0];

        match page_data[0] {
            LEAF_NODE => TreeNode::Leaf(LeafNode::decode(&page_data[..])),
            INTERNAL_NODE => TreeNode::Internal(InternalNode::decode(&page_data[..])),
            _ => panic!("unknown node type")
        }
    }

    fn insert(&mut self, key: Key, page_id: PageId, slot_num: u32) {
        // first ever insert
        if self.root_page_id == INVALID_PAGE_ID {
            {   
                let root_page_id = self.buffer_pool.new_page();
                self.root_page_id = root_page_id;
                let mut header_guard = self.buffer_pool.check_write_page(self.header_page_id).unwrap();
                let mut data = header_guard.data_mut().unwrap();
                self.set_root_page_id(&mut data[..]);

            }

            // if the root page id is equal to an invalid page id this mean that we do not have 
            // a root node yet so yes we are creating a root page first then we go create an in memory leaf 
            let leaf = LeafNode {
                // root leaf in the first ever insert will not have a next page id
                next_page_id: INVALID_PAGE_ID,

                entries: vec![
                    (key, RecordId { page_id, slot_num })
                ]
            };

            // get the root page with a write guard
            let mut root_page_guard = self.buffer_pool.check_write_page(self.root_page_id).unwrap();
            let mut root_page_data = root_page_guard.data_mut().unwrap();

            leaf.encode(&mut root_page_data[..]);

        } else {
            let mut leaf = LeafNode {
                next_page_id: INVALID_PAGE_ID,
                entries: vec![],
            };
            let mut right_leaf_page_id = self.root_page_id;

            {
                let mut root_page_guard = self.buffer_pool.check_write_page(self.root_page_id).unwrap();
                let mut root_page_data = root_page_guard.data_mut().unwrap();
                let node_type = root_page_data[0];

                if node_type == LEAF_NODE {
                    leaf = LeafNode::decode(&root_page_data[..]);

                    // add element when the root is a leaf
                    if leaf.entries.len() <= self.leaf_node_max_size.try_into().unwrap() {
                        // should be sorted
                        leaf.entries.push((key, RecordId { page_id, slot_num }));
                        leaf.entries.sort_unstable_by_key(|item| item.0);
                        leaf.encode(&mut root_page_data[..]);
                    } 
                } 
                

                // navigating the tree
                if node_type == INTERNAL_NODE {
                    let mut internal  = InternalNode::decode(&root_page_data[..]);
                    loop {
                        match self.get_child(internal, key) {
                            TreeNode::Leaf(mut leafNode) => {
                                leaf = leafNode;
                                leaf.entries.push((key, RecordId { page_id, slot_num }));
                                leaf.entries.sort_unstable_by_key(|item| item.0);
                                break;
                            }
                            
                            TreeNode::Internal(next_internal) => {
                                internal = next_internal
                            }
                        }
                    }
                }
            }
        
            // first split
            if leaf.entries.len() > self.leaf_node_max_size.try_into().unwrap() {
                // first split
                let middle_index = leaf.entries.len() / 2;
                let middle_entry = leaf.entries[middle_index];

                let left_leaf_page_id = self.buffer_pool.new_page();
                {
                    let mut left_leaf_guard = self.buffer_pool.check_write_page(left_leaf_page_id).unwrap();
                    let mut left_leaf_data = left_leaf_guard.data_mut().unwrap();
                    let right_leaf_entries = leaf.entries.split_off(middle_index);
                    let left_leaf_entries = leaf.entries;
                    
                    let left_leaf = LeafNode {
                        next_page_id: right_leaf_page_id,
                        entries: left_leaf_entries,
                    };
                    leaf.entries = right_leaf_entries;

                    left_leaf.encode(&mut left_leaf_data[..]);
                } 

                {
                    let mut right_leaf_guard = self.buffer_pool.check_write_page(right_leaf_page_id).unwrap();
                    let mut right_leaf_data = right_leaf_guard.data_mut().unwrap();
                    leaf.encode(&mut right_leaf_data[..]);
                }


                let (internal_key , _) = middle_entry;
                let internal = InternalNode {
                    entries: vec![
                        (INVALID_KEY, left_leaf_page_id), 
                        (internal_key, right_leaf_page_id)
                    ]
                };

                let internal_page_id = self.buffer_pool.new_page();
                let mut internal_leaf_guard = self.buffer_pool.check_write_page(internal_page_id).unwrap();
                let mut internal_leaf_data = internal_leaf_guard.data_mut().unwrap();
                internal.encode(&mut internal_leaf_data[..]);

                if right_leaf_page_id == self.root_page_id {
                    let mut header_guard = self.buffer_pool.check_write_page(self.header_page_id).unwrap();
                    let mut header_data = header_guard.data_mut().unwrap();
        
                    self.root_page_id = internal_page_id;
                    self.set_root_page_id(&mut header_data[..]);
                }

            }
        }
    }
}


#[cfg(test)]
mod b_plus_tree_testing {
    use super::*;

    #[test]
    fn basic_leaf_encode_decode() {
        let mut bytes:&mut[u8] =  &mut [0; DEFAULT_LEAF_NODE_MAX_SIZE as usize];

        let leaf = LeafNode {
            next_page_id: 2,
            entries: vec![
                (11, RecordId { page_id: 111, slot_num: 1111 }),
                (12, RecordId { page_id: 112, slot_num: 1112 }),
            ],
        };

        leaf.encode(bytes);

        let decoded_leaf = LeafNode::decode(bytes);

        assert_eq!(leaf.next_page_id, decoded_leaf.next_page_id);
        assert_eq!(leaf.entries.len(), decoded_leaf.entries.len());
        assert_eq!(leaf.entries, decoded_leaf.entries);
    }

    #[test]
    fn basic_internal_encode_decode() {
        let mut bytes:&mut[u8] = &mut [0; DEFAULT_INTERNAL_NODE_MAX_SIZE as usize];

        let internal = InternalNode {
            entries: vec![
                (1, 11),
                (2, 12),
                (3, 13),
            ]
        };

        internal.encode(bytes);

        let decoded_internal = InternalNode::decode(bytes);

        assert_eq!(internal.entries.len(), decoded_internal.entries.len());
        assert_eq!(internal.entries, decoded_internal.entries);
    }

    #[test]
    fn btree_first_insert() {
        let mut btree = BTree::new();

        const KEY: i64 = 42;
        const PAGE_ID: u32 = 3;
        btree.insert(KEY, PAGE_ID, DEFAULT_SLOT_NUMBER);

        let header_guard = btree.buffer_pool.check_write_page(btree.header_page_id).unwrap();
        let header_data = header_guard.data().unwrap();
        let root_from_header = u32::from_le_bytes(header_data[0 .. 4].try_into().unwrap());

        assert_ne!(root_from_header, INVALID_PAGE_ID);


        let root_page_guard = btree.buffer_pool.check_read_page(root_from_header).unwrap();
        let root_data = root_page_guard.data().unwrap();
        let node_type = root_data[0];

        assert_eq!(node_type, 1);

        let expected_leaf = LeafNode {
            next_page_id: INVALID_PAGE_ID,
            entries: vec![
                (KEY, RecordId { page_id: PAGE_ID, slot_num: DEFAULT_SLOT_NUMBER})
            ]
        };

        let leaf = LeafNode::decode(&root_data[..]);

        assert_eq!(expected_leaf.next_page_id, leaf.next_page_id);
        assert_eq!(expected_leaf.entries, leaf.entries);
    }

    #[test] 
    fn btree_first_insert_multiple_keys() {
        let mut btree = BTree::new();

        const KEY: i64 = 42;
        const PAGE_ID: u32 = 2;
        btree.insert(KEY, PAGE_ID, DEFAULT_SLOT_NUMBER);

        const SECOND_KEY:i64 = 43;
        const SECOND_PAGE_ID:u32 = 3;
        btree.insert(SECOND_KEY, SECOND_PAGE_ID, DEFAULT_SLOT_NUMBER);

        const THIRD_KEY:i64 = 44;
        const THIRD_PAGE_ID:u32 = 4;
        btree.insert(THIRD_KEY, THIRD_PAGE_ID, DEFAULT_SLOT_NUMBER);

        let header_guard = btree.buffer_pool.check_read_page(btree.header_page_id).unwrap();
        let header_data = header_guard.data().unwrap();
        let root_from_header = u32::from_le_bytes(header_data[0 .. 4].try_into().unwrap());


        let root_page_guard = btree.buffer_pool.check_read_page(root_from_header).unwrap();
        let root_data = root_page_guard.data().unwrap();
        let node_type = root_data[0];

        let expected_leaf = LeafNode {
            next_page_id: INVALID_PAGE_ID,
            entries: vec![
                (KEY, RecordId { page_id: PAGE_ID, slot_num: DEFAULT_SLOT_NUMBER }),
                (SECOND_KEY, RecordId { page_id: SECOND_PAGE_ID, slot_num: DEFAULT_SLOT_NUMBER }),
                (THIRD_KEY, RecordId { page_id: THIRD_PAGE_ID, slot_num: DEFAULT_SLOT_NUMBER })
            ]
        };

        let leaf = LeafNode::decode(&root_data[..]);

        assert_eq!(expected_leaf.next_page_id, leaf.next_page_id);
        assert_eq!(expected_leaf.entries, leaf.entries)
    }

    #[test]
    fn btree_first_split() {
        let mut btree = BTree::new();
        btree.set_leaf_max_size(4);

        const FIRST_KEY: i64 = 41;
        const FIRST_PAGE_ID: u32 = 1;
        btree.insert(FIRST_KEY, FIRST_PAGE_ID, DEFAULT_SLOT_NUMBER);

        const SECOND_KEY: i64 = 42;
        const SECOND_PAGE_ID: u32 = 2;
        btree.insert(SECOND_KEY, SECOND_PAGE_ID, DEFAULT_SLOT_NUMBER);

        let old_root_page_id = btree.root_page_id;
        const THIRD_KEY: i64 = 43;
        const THIRD_PAGE_ID: u32 = 3;
        btree.insert(THIRD_KEY, THIRD_PAGE_ID, DEFAULT_SLOT_NUMBER);

        const FOURTH_KEY: i64 = 44;
        const FOURHT_PAGE_ID: u32 = 4;
        btree.insert(FOURTH_KEY, FOURHT_PAGE_ID, DEFAULT_SLOT_NUMBER);


        const FIFTH_KEY: i64 = 45;
        const FIFTH_PAGE_ID: u32 = 5;
        btree.insert(FIFTH_KEY, FIFTH_PAGE_ID, DEFAULT_SLOT_NUMBER);

        let header_guard = btree.buffer_pool.check_read_page(btree.header_page_id).unwrap();
        let header_data = header_guard.data().unwrap();
        let root_from_header = u32::from_le_bytes(header_data[0 .. 4].try_into().unwrap());

        assert_eq!(root_from_header, btree.root_page_id);
        assert_ne!(btree.root_page_id, old_root_page_id);
        assert_ne!(root_from_header, old_root_page_id);

        let root_page_guard = btree.buffer_pool.check_read_page(root_from_header).unwrap();
        let root_data = root_page_guard.data().unwrap();
        let node_type: u8 = root_data[0];

        const EXPECTED_NODE_TYPE: u8 = 0;
        assert_eq!(EXPECTED_NODE_TYPE, node_type);

    }

    #[test]
    fn btree_advanced_splits() {

    }

}
