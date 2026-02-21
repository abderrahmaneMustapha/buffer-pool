mod arc_replacer;
mod common;
mod disk_manager;
mod disk_scheduler;
mod frame_header;
mod guard;
mod buffer_pool;

use crate::buffer_pool::BufferPoolManager;
use crate::disk_manager::DiskManager;
use std::sync::{Mutex, Arc};

fn main() {
    let mut buffer_pool_manager = BufferPoolManager::new(100, Arc::new(Mutex::new(DiskManager::new("test.db"))));
    let page_id = buffer_pool_manager.new_page();
    buffer_pool_manager.check_read_page(page_id);
    let data = buffer_pool_manager.check_write_page(page_id);
    data.unwrap().data_mut().unwrap().fill(1);

    buffer_pool_manager.flush_page(page_id);
}
