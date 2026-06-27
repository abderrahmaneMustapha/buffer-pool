
// ============================================================================
// B+Tree 
// a balanced search data structure
// ============================================================================
use crate::buffer_pool::BufferPoolManager;
use crate::disk_manager::DiskManager;
use crate::common::PageId;
use std::sync::{Arc, Mutex};

const DEFAULT_LEAF_NODE_MAX_SIZE: u32 = 511;
const DEFAULT_INTERNAL_NODE_MAX_SIZE: u32 = 681;
const HEADER_SIZE: usize = 16;

type Key = i64;

#[derive(Clone, Copy, PartialEq)]
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
    root_page_id: PageId,
    buffer_pool: BufferPoolManager,
    max_leaf_size: u32,
    max_internal_size: u32,
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
          let key = Key::from_le_bytes(bytes[off .. off+8].try_into().unwrap());
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
            let key = Key::from_le_bytes(bytes[off .. off+8].try_into().unwrap());
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
        assert_eq!(internal.entries.first(), decoded_internal.entries.first());
    }
}
