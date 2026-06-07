
// ============================================================================
// DISK MANAGER
// ============================================================================
// Handles file I/O operations for database pages
// Reads/writes 8KB pages to/from disk files
// Manages page allocation and free slot reuse
// ============================================================================

use std::collections::{ HashMap };
use crate::common::{ PageId, PAGE_SIZE, DB_IO_SIZE };
use std::sync::{Mutex};
use std::fs::{OpenOptions, File, remove_file};
use std::io::{self, Read, Write, SeekFrom, Seek};

pub struct DiskManager {
    db_file: Mutex<File>,
    page_capacity: usize,
    page_size: usize,
    pages: Mutex<HashMap<PageId, u64>>,
    num_writes: Mutex<u64>,
    num_deletes: Mutex<u64>,
    free_slots: Vec<u64>,
}

/**
 *  Implement DiskManager
 * File I/O: File, OpenOptions, read(), write(), seek()
 * Start simple: open a file, write a page, read it back
 * Add page allocation: track offsets, reuse deleted slots
 * Test: verify you can read/write pages correctly
 */
impl DiskManager {
    pub fn new(file_path: &str) -> Self {
        
        let db_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(file_path).unwrap();
        
        let page_capacity = DB_IO_SIZE;
        let page_size = PAGE_SIZE;
    
        let _ = db_file.set_len(((page_capacity + 1) * page_size) as u64);

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
            num_deletes: Mutex::new(0),
        }
    }

    pub fn write_page(&mut self, page_id: PageId, page_data: &[u8]) -> Result<(), std::io::Error> {
        let mut file = self.db_file.lock().unwrap();
        let offset =  {
            let pages = self.pages.lock().unwrap();
            if let Some(&existing_offset) = pages.get(&page_id) {
                existing_offset
            } else {
                if !self.free_slots.is_empty() {
                    let new_offset: u64 = { 
                       if let Some(value) = self.free_slots.pop() {
                            value
                       } else {
                        println!("No free slots available");
                        0
                       }
                    };

                    new_offset
                }  else {
                    if pages.len() + 1 >= self.page_capacity {
                        self.page_capacity *= 2;
                        let _ = file.set_len(((self.page_capacity + 1) * PAGE_SIZE) as u64);
                    }
                    let new_offset: u64 = (pages.len() * PAGE_SIZE) as u64;

                    new_offset
                }
            }
        };

        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&page_data[..PAGE_SIZE])?;

        *self.num_writes.lock().unwrap() += 1;
        self.pages.lock().unwrap().insert(page_id, offset);

        file.flush()?;

        Ok(())
    }

    pub fn read_page<'a>(&mut self, page_id: PageId, page_data: &'a mut[u8]) -> Result<&'a[u8], std::io::Error> {
        let mut file = self.db_file.lock().unwrap();
        let offset = {
            let pages = self.pages.lock().unwrap();
            if let Some(&existing_offset) = pages.get(&page_id) {
                existing_offset
            } else {
                if !self.free_slots.is_empty() {
                    let new_offset: u64 = { 
                       if let Some(value) = self.free_slots.pop() {
                            value
                       } else {
                        println!("No free slots available");
                        0
                       }
                    };

                    new_offset
                } else {          
                    if pages.len() + 1 >= self.page_capacity {
                        self.page_capacity *= 2;
                        let _ = file.set_len(((self.page_capacity + 1) * PAGE_SIZE) as u64);
                    }
                    let new_offset: u64 = (pages.len() * PAGE_SIZE) as u64;
                    new_offset
                }
            }
        };

        let file_size = match file.metadata() {
            Ok(metadata) => metadata.len(),
            Err(e) => {
                eprintln!("Error getting file size: {}", e);
                return Err(e);
            }
        };

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

    pub fn delete_page(&mut self, page_id: PageId) -> Result<(), std::io::Error> {
        let mut pages = self.pages.lock().unwrap();

        match pages.remove(&page_id) {
            Some(offset) => {
                self.free_slots.push(offset);
                *self.num_deletes.lock().unwrap() += 1;
            } 
            None => {
                println!("Page {} not found", page_id);
            }
        }
    
        Ok(())
    }
}

#[cfg(test)]
mod disk_manager_tests {
    use super::*;

    #[test]
    fn test_basic_write_read_delete_operations() {
        let test_file = "test.db";
        let _ = remove_file(test_file);

        let mut dm = DiskManager::new(test_file);

        let page_data = b"Hello world".repeat(PAGE_SIZE / 10);
        let page_data = &page_data[..PAGE_SIZE];

        dm.write_page(0, page_data).unwrap();

        assert_eq!(*dm.num_writes.lock().unwrap(), 1);

        let mut buffer = vec![0u8; PAGE_SIZE];
        dm.read_page(0, &mut buffer).unwrap();

        assert_eq!(dm.free_slots.len(), 0);
        assert_eq!(&page_data[..1], &buffer[..1]);

        dm.delete_page(0).unwrap();

        assert_eq!(*dm.num_deletes.lock().unwrap(), 1);
        assert_eq!(dm.free_slots.len(), 1);

        assert_eq!(dm.page_capacity, DB_IO_SIZE);

        let _ = remove_file(test_file);
    }
    
    #[test]
    fn test_use_existing_slot() {
        let test_file =  "test.db";
        let _ = remove_file(test_file);

        let mut dm = DiskManager::new(test_file);

        let page_data = b"Hello world".repeat(PAGE_SIZE / 10);
        let page_data = &page_data[..PAGE_SIZE];
        dm.write_page(1, page_data).unwrap();
        dm.write_page(2, page_data).unwrap();
        dm.write_page(3, page_data).unwrap();
        dm.write_page(4, page_data).unwrap();
        dm.write_page(5, page_data).unwrap();

        assert_eq!(*dm.num_writes.lock().unwrap(), 5);
        let third_offset = {
            let binding = dm.pages.lock().unwrap();
            binding.get(&3).unwrap().clone()
        };

        dm.delete_page(3).unwrap();
        
        assert_eq!(dm.free_slots.contains(&third_offset), true);
        assert_eq!(dm.free_slots.len(), 1);
    
        dm.write_page(9, page_data).unwrap();

        let ninth_offset = {
            let binding = dm.pages.lock().unwrap();
            binding.get(&9).unwrap().clone()
        };
        assert_eq!(dm.free_slots.len(), 0);
        assert_eq!(third_offset, ninth_offset);

        let second_offset = {
            let binding = dm.pages.lock().unwrap();
            println!("binding: {:?}", binding);
            binding.get(&2).unwrap().clone()
        };
        assert_ne!(second_offset, third_offset);

        let _ = remove_file(test_file);
    }

    #[test]
    fn test_increase_page_capacity() {
        let test_file = "test.db";
        let _ = remove_file(test_file);

        let mut dm = DiskManager::new(test_file);

        dm.page_capacity = 3;

        let page_data = b"Hello world".repeat(PAGE_SIZE / 10);
        let page_data = &page_data[..PAGE_SIZE];

        dm.write_page(1, page_data).unwrap();
        dm.write_page(2, page_data).unwrap();
        dm.write_page(3, page_data).unwrap();

        assert_eq!(dm.page_capacity, 6);
        assert_eq!(dm.db_file.lock().unwrap().metadata().unwrap().len(), 57344);

        let _ = remove_file(test_file);
    }
}