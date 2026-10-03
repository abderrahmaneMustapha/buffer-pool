# Buffer Pool + B+Tree in Rust

A small disk-based storage engine built step by step, following CMU 15-445 (Fall 2025).
It has four layers: a disk manager, a disk scheduler, a buffer pool with an ARC replacer, and a B+Tree index on top.

## Architecture

```
        BTree (index)
            │  check_read_page / check_write_page / new_page
            ▼
   BufferPoolManager ──► ArcReplacer   (who gets evicted)
            │
            │  DiskRequest + channel for the reply
            ▼
      DiskScheduler   (one background worker thread)
            │
            ▼
       DiskManager    (page_id -> file offset, 8 KB pages)
            │
            ▼
        main.db file
```

Every page is 8192 bytes (`PAGE_SIZE`). A `PageId` is a `u32`, and `INVALID_PAGE_ID` is `u32::MAX`.

## Modules

| File | Role |
|---|---|
| `common.rs` | Shared types and constants: `PageId`, `FrameId`, `PAGE_SIZE`, `INVALID_PAGE_ID`. |
| `disk_manager.rs` | Reads and writes 8 KB pages in one file. |
| `disk_scheduler.rs` | Queues disk requests to a worker thread. |
| `frame_header.rs` | One in-memory slot that holds a page. |
| `arc_replacer.rs` | Chooses which frame to evict. |
| `guard.rs` | `ReadPageGuard` / `WritePageGuard`: pin a page while you use it. |
| `buffer_pool.rs` | Caches pages in frames and loads or evicts them. |
| `b_plus_tree.rs` | The B+Tree index. |

### Disk manager

Maps each `page_id` to a byte offset in the file. When a page is deleted, its offset goes on a free list and the next new page reuses it. When the file is full, it doubles its capacity.

### Disk scheduler

The buffer pool never touches the file directly. It sends a `DiskRequest` (`Read`, `Write`, or `Delete`) with a channel sender, then waits on the receiver. One worker thread runs the requests in order. Dropping the scheduler stops the worker and joins it.

### Frame header

Holds the page bytes in a `RwLock<Vec<u8>>`, plus a `pin_count` and an `is_dirty` flag. A frame with `pin_count > 0` cannot be evicted.

### ARC replacer

Adaptive Replacement Cache. It balances recency and frequency with four lists:

- **MRU**: pages seen once recently.
- **MFU**: pages seen more than once.
- **MRU ghost / MFU ghost**: page ids that were evicted recently.

A hit in a ghost list moves `mru_target_size`, so the cache shifts toward whichever list is paying off. Only frames marked evictable (unpinned) can be chosen.

### Page guards

`check_read_page` and `check_write_page` return a guard. Creating it pins the frame and records an access in the replacer. Dropping it unpins the frame and marks it evictable at zero pins. `data()` and `data_mut()` take the frame's `RwLock` only for as long as that borrow lives. `data_mut()` also marks the page dirty.

### Buffer pool manager

On a page request:

1. If the page is already in a frame, return a guard.
2. Otherwise take a free frame and read the page from disk.
3. If no frame is free, ask the replacer for a victim, flush it if dirty, reset it, and load the new page there.

It also provides `new_page`, `delete_page`, `flush_page`, and `flush_all_pages`.

## B+Tree

Keys are `i64`. A value is a `RecordId { page_id, slot_num }` that points at a tuple. `slot_num` is a placeholder until a table page exists.

### On-disk layout

**Header page**: bytes `0..4` hold `root_page_id`.

**Node header (16 bytes)**:

| Bytes | Field |
|---|---|
| `0` | node type: `1` = leaf, `0` = internal |
| `4..8` | current number of entries |
| `8..12` | max size |
| `12..16` | `next_page_id` (leaf only) |

**Leaf entry (16 bytes)**: `key: i64 | page_id: u32 | slot_num: u32`

**Internal entry (12 bytes)**: `key: i64 | child_page_id: u32`. The first entry's key is `INVALID_KEY` (`i64::MIN`), because the leftmost child has no lower bound.

Nodes are decoded into `LeafNode` / `InternalNode` structs, changed in memory, then encoded back into the page bytes.

### Operations

**`find(key)`**: start at the root, binary-search each internal node for the right child, and stop at a leaf. Returns `Some((key, RecordId))` or `None`.

**`insert(key, page_id, slot_num)`**:

1. **Empty tree**: create a leaf, make it the root, and save its id in the header page.
2. **Descend**: walk to the target leaf and keep write guards for the path on a stack. If a node still has room, it cannot split, so the stack is cleared and the ancestors' guards are released.
3. **Insert into the leaf**, keeping the entries sorted.
4. **Leaf split** when the leaf exceeds its max size: move the upper half to a new right leaf, link `left.next_page_id -> right`, and push `(first key of right, right page id)` up to the parent.
5. **Propagate upward**: insert into the parent. If the parent overflows, split it as well and push its middle key up. This repeats until a node has room.
6. **New root**: if the old root splits, create an internal root with two children and update the header page.

### Tests

`cargo test` covers:

- Leaf and internal encode/decode round trips.
- First insert and several inserts into one leaf.
- First leaf split creating an internal root.
- Leaf `next_page_id` links after a split.
- Splits that propagate through internal nodes and grow a new root.
- A split that stops at the grandparent.
- `find` across a multi-level tree.

## Status

Done:

- Disk manager, disk scheduler, ARC replacer, page guards, buffer pool manager.
- B+Tree page layout, `find`, `insert` with leaf and internal splits, root growth.
- Insert releases ancestor guards early once it reaches a node that has room.

Not done yet:

- **Delete**, including merging with a sibling, borrowing from a sibling, and shrinking the root.
- **Iterator**: a cursor that follows `next_page_id` for range scans.
- **Full concurrency**:
  - Page latches are not held for the whole life of a guard.
  - `find` does not use crabbing.
  - There is no optimistic insert with restart.
  - There is no concurrent test.
  - `BTree` methods take `&mut self`.

## Run

```bash
cargo build
cargo test
cargo run
```
