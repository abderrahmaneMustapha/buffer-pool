
// ============================================================================
// B+Tree 
// a balanced search data structure
// ============================================================================
use crate::buffer_pool::BufferPoolManager;
use crate::disk_manager::DiskManager;
use crate::common::{PageId, INVALID_PAGE_ID};
use crate::guard::WritePageGuard;
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

struct DEPRECATED_LocatedNode {
    parent_key_index: usize,
    parent_page_id: PageId,
    page_id: PageId,
    node: TreeNode,
}

struct PathFrame {
    slot_to_next_child: usize,
    guard: WritePageGuard,
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

    fn find_kv_index(&self, key: Key) -> Option<LeafKv> {
        let mut low: usize = 0;
        let mut high: usize = self.entries.len();
        let mut res: Option<LeafKv> = None;

        while low < high {
            let mid = low + (high - low) / 2;

            if self.entries[mid].0 <= key {
                low = mid + 1;
            } else {
                high = mid;
            }

            if self.entries[mid].0 == key {
                res = Some(self.entries[mid]);
                break;
            }
        }

        res
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

    fn find(&mut self, key: Key) -> Option<LeafKv> {
        let mut id_to_next_child =  {
            let header_guard = self.buffer_pool.check_read_page(self.header_page_id).unwrap();
            let header_data = header_guard.data().unwrap();

            u32::from_le_bytes(header_data[0 .. 4].try_into().unwrap())
        };

        let mut leaf_kv: Option<LeafKv> = None;

        loop {
            let page_guard = self.buffer_pool.check_read_page(id_to_next_child).unwrap();
            let data = page_guard.data().unwrap();
            let node_type = data[0];

            if node_type == LEAF_NODE {
                let leaf = LeafNode::decode(&data[..]);
                leaf_kv = leaf.find_kv_index(key);
            } 
            
            if node_type == INTERNAL_NODE {
                let internal = InternalNode::decode(&data[..]);
                let slot_to_next_child = internal.find_child_index(key);
                id_to_next_child = internal.entries[slot_to_next_child].1;
            }

            else {
                break;
            }
        }

        leaf_kv
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
        } 

        // other insertions
        else {
            // this is where we go advanced splits we get a stack and we save the write guards
            // thats naiv approach and to be improved later with latch crabbing after making
            // the solution work
            let mut path_stack: Vec<PathFrame> = vec![];

            {
                let mut root_page_guard = self.buffer_pool.check_write_page(self.root_page_id).unwrap();
  
                let node_type = {
                    let mut root_page_data = root_page_guard.data_mut().unwrap();

                    root_page_data[0]
                };

                if node_type == LEAF_NODE {
                    path_stack.push(PathFrame {
                        slot_to_next_child: usize::MAX,
                        guard:root_page_guard,
                    })
                }
                
                // navigating the tree
                else if node_type == INTERNAL_NODE {
                    let mut internal  = {
                        let root_page_data = root_page_guard.data().unwrap();
                        InternalNode::decode(&root_page_data[..])
                    };


                    let mut slot_to_next_child = internal.find_child_index(key);
                    path_stack.push(PathFrame {
                        slot_to_next_child,
                        guard: root_page_guard,
                    });
                    loop {
                        let mut child_page_id = INVALID_PAGE_ID;
                        let mut node_type: u8 = u8::MAX;
                        {   
                            child_page_id =  internal.entries[slot_to_next_child].1;

                            let child_read_guard = self.buffer_pool.check_read_page(child_page_id).unwrap();
                            let child_read_data = child_read_guard.data().unwrap();

                            node_type = child_read_data[0];

                            if node_type == INTERNAL_NODE {
                                let child_internal_node = InternalNode::decode(&child_read_data[..]);
                                slot_to_next_child = child_internal_node.find_child_index(key);
                                internal = child_internal_node;
                            }
                        } 

                        if node_type == LEAF_NODE {

                            slot_to_next_child = INVALID_PAGE_ID as usize;

                            let guard = self.buffer_pool.check_write_page(child_page_id).unwrap();
                    
                            path_stack.push(PathFrame {
                                guard,
                                slot_to_next_child,
                            });
                            break;
                        }

                        let guard = self.buffer_pool.check_write_page(child_page_id).unwrap();
                        path_stack.push(PathFrame {
                            guard,
                            slot_to_next_child,
                        });
                    }
                }
            }

            let split:u32 = {
                let entrie = path_stack.pop();
                let mut leaf_guard = entrie.unwrap().guard;
                let leaf_page_id = leaf_guard.page_id();
                let mut leaf_data = leaf_guard.data_mut().unwrap();
                let mut leaf_node = LeafNode::decode(&leaf_data[..]);
                leaf_node.entries.push((key, RecordId { page_id, slot_num }));
                leaf_node.entries.sort_unstable_by_key(|item| item.0);
                leaf_node.encode(&mut leaf_data[..]);

                if leaf_node.entries.len() > self.leaf_node_max_size.try_into().unwrap() {
                    println!("we are splitting things here len {}, max len {}, leaf page id {}", leaf_node.entries.len(), self.leaf_node_max_size, leaf_page_id);
                    leaf_page_id
                } else {
                    INVALID_PAGE_ID
                }
            };

            if split != INVALID_PAGE_ID {

                let right_leaf_page_id = self.buffer_pool.new_page();

                let left_leaf_page_id = split;
                let mut middle_entry;

                // we splitted things here
                {
                    let mut left_leaf_guard = self.buffer_pool.check_write_page(left_leaf_page_id).unwrap();
                    let mut left_leaf_data = left_leaf_guard.data_mut().unwrap();
                    let mut left_leaf = LeafNode::decode(&left_leaf_data[..]);

                    let middle_index = left_leaf.entries.len() / 2;
                    middle_entry = left_leaf.entries[middle_index];

                    let mut right_leaf_guard = self.buffer_pool.check_write_page(right_leaf_page_id).unwrap();
                    let mut right_leaf_data = right_leaf_guard.data_mut().unwrap();

                    let right_leaf_entries = left_leaf.entries.split_off(middle_index);
                    let left_leaf_entries = left_leaf.entries;

                    let right_leaf = LeafNode {
                        next_page_id: left_leaf.next_page_id,
                        entries: right_leaf_entries,
                    };
                    right_leaf.encode(&mut right_leaf_data[..]);
  
                    left_leaf.entries = left_leaf_entries;

                    left_leaf.next_page_id = right_leaf_page_id;
                    left_leaf.encode(&mut left_leaf_data[..]);
                } 

                let mut intended_insert = (INVALID_KEY, INVALID_PAGE_ID);
                let (sep_key, ..) = middle_entry;
                let mut intended_insert = (sep_key, right_leaf_page_id);
                let mut left_page_id = left_leaf_page_id;
                println!("stack len {}", path_stack.len());
                loop {
                    match path_stack.pop() {
                        Some(frame) => {
                            let mut internal_page_id = {
                                frame.guard.page_id()
                            };

                            let mut internal_guard = frame.guard;
                            let mut internal_data = internal_guard.data_mut().unwrap();
                            let mut internal_node = InternalNode::decode(&internal_data[..]);

                            let (key, right_page_id) = intended_insert;
                      
                            let ptr_to_left_index = internal_node.entries.iter().position(|(.. , _pid)| *_pid == left_page_id).unwrap();
                            internal_node.entries.insert(ptr_to_left_index + 1, (key, right_page_id));

                            if internal_node.entries.len() <= self.internal_node_max_size as usize {
                                internal_node.encode(&mut internal_data[..]);
                                break;
                            }

                            let mid = internal_node.entries.len() / 2;
                            let (up_key, up_pid) = internal_node.entries[mid];

                            let right_internal_id = self.buffer_pool.new_page();

                            let mut right_entries = internal_node.entries.split_off(mid);
                            right_entries[0].0 = INVALID_KEY;
                            
                            {
                                let mut right_guard = self.buffer_pool.check_write_page(right_internal_id).unwrap();
                                let mut right_data = right_guard.data_mut().unwrap();
                                let right_internal_node = InternalNode {
                                    entries: right_entries
                                };

                                right_internal_node.encode(&mut right_data[..]);
                            }

                            internal_node.encode(&mut internal_data);

                            intended_insert = (up_key, right_internal_id);
                            left_page_id = internal_page_id;
                        }

                        None => {
                            println!("asndioashdas");
                            let root_page_id = {
                                let new_root_page_id = self.buffer_pool.new_page();
                                let mut root_page_guard = self.buffer_pool.check_write_page(new_root_page_id).unwrap();
                                let mut root_page_data = root_page_guard.data_mut().unwrap();

                                let (key, right_page_id) = intended_insert;
                                let root_node = InternalNode {
                                    entries: vec![(INVALID_KEY, left_page_id), (key, right_page_id)]
                                };

                                {
                                    root_node.encode(&mut root_page_data[..]);
                                }

                                new_root_page_id
                            };

                            self.root_page_id = root_page_id;
                            let mut header_guard = self.buffer_pool.check_write_page(self.header_page_id).unwrap();
                            let mut header_data = header_guard.data_mut().unwrap();
                            self.set_root_page_id(&mut header_data[..]);
                            break;
                        }
                    }
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
    fn btree_insert_after_split_persists_to_leaf() {
        let mut btree = BTree::new();
        btree.set_leaf_max_size(4);

        for (k, pid) in [(41, 1), (42, 2), (43, 3), (44, 4), (45, 5)] {
            btree.insert(k, pid, DEFAULT_SLOT_NUMBER);
        }
        /*
                43 
        
        41, 42,     43, 44, 45
        */

        btree.insert(46, 6, DEFAULT_SLOT_NUMBER);

        let header_guard = btree.buffer_pool.check_read_page(btree.header_page_id).unwrap();
        let header_data = header_guard.data().unwrap();
        let root_from_header = u32::from_le_bytes(header_data[0 .. 4].try_into().unwrap());

        let root_page_guard = btree.buffer_pool.check_read_page(root_from_header).unwrap();
        let root_data = root_page_guard.data().unwrap();
        
        let root_node = InternalNode::decode(&root_data[..]);

        {
            let left_guard = btree.buffer_pool.check_read_page(root_node.entries[0].1).unwrap();
            let left_data = left_guard.data().unwrap();
            let left_node = LeafNode::decode(&left_data[..]);

            assert_eq!(root_node.entries[1].1, left_node.next_page_id);
        }    

        {
            let right_guard = btree.buffer_pool.check_read_page(root_node.entries[1].1).unwrap();
            let right_data = right_guard.data().unwrap();
            let right_node = LeafNode::decode(&right_data[..]);

            assert_eq!(right_node.next_page_id, INVALID_PAGE_ID);
        }   
    }
    
    #[test]
    fn btree_advanced_splits() {
        let mut btree = BTree::new();
        btree.set_leaf_max_size(3);
        btree.set_internal_max_size(4);

        {
            for (k, pid) in [(41, 1), (42, 2), (43, 3), (44, 4), (45, 5), (46, 6), (47, 7)] {
                btree.insert(k, pid, DEFAULT_SLOT_NUMBER);
            }
            /*
                    43,     45
                    
                41,42   43,44,  45,46,47

                
                     {3}  43.          45 
                {1} 41 42     {2} 43 44    {4} 45 46
            */
            let header_guard = btree.buffer_pool.check_read_page(btree.header_page_id).unwrap();
            let header_data = header_guard.data().unwrap();
            let root_from_header = u32::from_le_bytes(header_data[0 .. 4].try_into().unwrap());

            let root_page_guard = btree.buffer_pool.check_read_page(root_from_header).unwrap();
            let root_data = root_page_guard.data().unwrap();

            let root_node = InternalNode::decode(&root_data[..]);

            assert_eq!(root_from_header, 3);
            assert_eq!(root_node.entries.len(), 3);
            assert_eq!(root_node.entries[0].0, INVALID_KEY);
            assert_eq!(root_node.entries[1].0, 43);
            assert_eq!(root_node.entries[2].0, 45);

            // reading the leaf that has the keys 41, 42
            let first_leaf_guard = btree.buffer_pool.check_read_page(root_node.entries[0].1).unwrap();
            let first_leaf_data = first_leaf_guard.data().unwrap();
            let first_leaf_first_key = i64::from_le_bytes(first_leaf_data[HEADER_SIZE .. HEADER_SIZE + 8].try_into().unwrap());
            let first_leaf_second_key = i64::from_le_bytes(first_leaf_data[HEADER_SIZE + 16 .. HEADER_SIZE + 16 + 8].try_into().unwrap());
            let first_leaf_next_page_id = u32::from_le_bytes(first_leaf_data[12 .. 16].try_into().unwrap());
            let first_leaf_current_size = u32::from_le_bytes(first_leaf_data[4 .. 8].try_into().unwrap());

            assert_eq!(first_leaf_first_key, 41);
            assert_eq!(first_leaf_second_key, 42);
            assert_eq!(first_leaf_current_size, 2);
            assert_eq!(first_leaf_next_page_id, root_node.entries[1].1);


            let second_leaf_guard = btree.buffer_pool.check_read_page(first_leaf_next_page_id).unwrap();
            let second_leaf_data = second_leaf_guard.data().unwrap();
            let second_leaf_first_key = i64::from_le_bytes(second_leaf_data[HEADER_SIZE .. HEADER_SIZE + 8].try_into().unwrap());
            let second_leaf_second_key = i64::from_le_bytes(second_leaf_data[HEADER_SIZE + 16 .. HEADER_SIZE + 16 + 8].try_into().unwrap());
            let second_leaf_next_page_id = u32::from_le_bytes(second_leaf_data[12 .. 16].try_into().unwrap());
            let second_leaf_current_size = u32::from_le_bytes(second_leaf_data[4 .. 8].try_into().unwrap());

            assert_eq!(second_leaf_first_key, 43);
            assert_eq!(second_leaf_second_key, 44);
            assert_eq!(second_leaf_current_size, 2);
            assert_eq!(second_leaf_next_page_id, root_node.entries[2].1);


            let third_leaf_guard = btree.buffer_pool.check_read_page(second_leaf_next_page_id).unwrap();
            let third_leaf_data = third_leaf_guard.data().unwrap();
            let third_leaf_first_key = i64::from_le_bytes(third_leaf_data[HEADER_SIZE .. HEADER_SIZE + 8].try_into().unwrap());
            let third_leaf_second_key = i64::from_le_bytes(third_leaf_data[HEADER_SIZE + 16 .. HEADER_SIZE + 16 + 8].try_into().unwrap());
            let third_leaf_third_key = i64::from_le_bytes(third_leaf_data[HEADER_SIZE + 32 .. HEADER_SIZE + 32 + 8 ].try_into().unwrap());
            let third_leaf_next_page_id = u32::from_le_bytes(third_leaf_data[12 .. 16].try_into().unwrap());
            let third_leaf_current_size = u32::from_le_bytes(third_leaf_data[4 .. 8].try_into().unwrap());

            assert_eq!(third_leaf_first_key, 45);
            assert_eq!(third_leaf_second_key, 46);
            assert_eq!(third_leaf_third_key, 47);
            assert_eq!(third_leaf_next_page_id, INVALID_PAGE_ID);
            assert_eq!(third_leaf_current_size, 3);
        }

        {
            /*
                43,     45
                
            40,41,42   43,44,  45,46,47
            */
            btree.insert(40, 10, DEFAULT_SLOT_NUMBER);

            let root_from_header = btree.root_page_id;
            let root_page_guard = btree.buffer_pool.check_read_page(root_from_header).unwrap();
            let root_data = root_page_guard.data().unwrap();
            let root_node = InternalNode::decode(&root_data[..]);

            let first_leaf_guard = btree.buffer_pool.check_read_page(root_node.entries[0].1).unwrap();
            let first_leaf_data = first_leaf_guard.data().unwrap();
            let first_leaf_first_key = i64::from_le_bytes(first_leaf_data[HEADER_SIZE .. HEADER_SIZE + 8].try_into().unwrap());

            assert_eq!(first_leaf_first_key, 40);
        }

        {
            /*
                  41,       43,     45, 47
                
            39,40    41,42   43,44,  45,46,  47, 48
            */

            btree.insert(39, 1009, DEFAULT_SLOT_NUMBER);

            let header_guard = btree.buffer_pool.check_read_page(btree.header_page_id).unwrap();
            let header_data = header_guard.data().unwrap();
                
            let root_from_header = u32::from_le_bytes(header_data[0 .. 4].try_into().unwrap());

            let root_page_guard = btree.buffer_pool.check_read_page(root_from_header).unwrap();
            let root_data = root_page_guard.data().unwrap();

            let root_node = InternalNode::decode(&root_data[..]);

            assert_eq!(root_from_header, 3);
            assert_eq!(root_node.entries.len(), 4);
            assert_eq!(root_node.entries[0].0, INVALID_KEY);
            assert_eq!(root_node.entries[1].0, 41);
            assert_eq!(root_node.entries[2].0, 43);
            assert_eq!(root_node.entries[3].0, 45);
        }
        { 
            btree.insert(48, 8, DEFAULT_SLOT_NUMBER);

            let header_guard = btree.buffer_pool.check_read_page(btree.header_page_id).unwrap();
            let header_data = header_guard.data().unwrap();
                
            let root_from_header = u32::from_le_bytes(header_data[0 .. 4].try_into().unwrap());

            let root_page_guard = btree.buffer_pool.check_read_page(root_from_header).unwrap();
            let root_data = root_page_guard.data().unwrap();

            let root_node = InternalNode::decode(&root_data[..]);
            assert_ne!(root_from_header, 3);
            assert_eq!(root_from_header, 8);

            assert_eq!(root_node.entries.len(), 2);
            assert_eq!(root_node.entries[0].0, INVALID_KEY);
            assert_eq!(root_node.entries[1].0, 43);
        }
    }

    #[test]
    fn btree_split_stops_at_grandparent() {
        let mut btree = BTree::new();
        btree.set_leaf_max_size(3);
        btree.set_internal_max_size(3);

        for (k, pid) in [
            (10, 1), (20, 2), (30, 3), (40, 4), (50, 5), (60, 6),
            (70, 7), (80, 8), (90, 9), (100, 10), (110, 11),
        ] {
            btree.insert(k, pid, DEFAULT_SLOT_NUMBER);
        }

        let root_before = btree.root_page_id;

        btree.insert(120, 12, DEFAULT_SLOT_NUMBER);

        let header_guard = btree.buffer_pool.check_read_page(btree.header_page_id).unwrap();
        let header_data = header_guard.data().unwrap();
        let root_from_header = u32::from_le_bytes(header_data[0..4].try_into().unwrap());

        assert_eq!(root_from_header, root_before);

        let root_guard = btree.buffer_pool.check_read_page(root_from_header).unwrap();
        let root = InternalNode::decode(&root_guard.data().unwrap()[..]);

        assert_eq!(root.entries.len(), 3);
        assert_eq!(root.entries[0].0, INVALID_KEY);
        assert_eq!(root.entries[1].0, 50);
        assert_eq!(root.entries[2].0, 90);
    }

    #[test]
    fn btree_find() {

    }
    // TODO next add find function for search and do latch crabbing in insert
}
