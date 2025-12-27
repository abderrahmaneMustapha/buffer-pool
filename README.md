# Buffer Pool with ARC Replacement Policy

## What is a Buffer Pool?

A buffer pool is a cache that keeps frequently accessed database pages in memory. Instead of reading from disk every time, the database stores hot pages in RAM for faster access. When the pool is full, we need a smart way to decide which pages to evict.

## What is ARC Algorithm?

ARC (Adaptive Replacement Cache) is a replacement policy that automatically adapts to the workload. It tracks both:
- **Recency** (LRU): Pages accessed recently
- **Frequency** (LFU): Pages accessed multiple times

ARC uses four lists:
- **MRU**: Pages seen once recently (in cache)
- **MFU**: Pages seen multiple times (in cache)  
- **MRU Ghost**: Recently evicted pages (seen once)
- **MFU Ghost**: Recently evicted pages (seen multiple times)

## Implementation

This project implements the ARC replacer in Rust. The `ArcReplacer` tracks page access patterns and decides which frames to evict when the buffer pool is full. It supports:

- Recording page accesses
- Marking frames as evictable (pinned/unpinned)
- Evicting frames based on ARC algorithm

## Next Steps

Next, we'll add:
- **Storage layer**: Disk I/O for reading/writing pages
- **Buffer Pool Manager**: Coordinates between the replacer, storage, and frame allocation

This will complete the buffer pool system for our database.