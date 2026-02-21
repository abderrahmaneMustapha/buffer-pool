
// ============================================================================
// DISK SCHEDULER
// ============================================================================
// Schedules disk read/write operations asynchronously
// Uses background worker thread to process queued requests
// ============================================================================

use crate::common::{ PageId };
use crate::disk_manager::DiskManager;
use std::sync::{Mutex, Arc};
use std::thread;
use std::sync::mpsc;

pub enum DiskRequestType {
    Read,
    Write,
    Delete,
}

pub struct DiskRequest {
    pub r#type: DiskRequestType,
    pub promise: mpsc::Sender<Option<Vec<u8>>>,
    pub data: Vec<u8>,
    pub page_id: PageId,
}

pub struct DiskScheduler {
    request_tx: mpsc::Sender<Option<DiskRequest>>,
    disk_manager: Arc<Mutex<DiskManager>>,
    worker_handle: Option<thread::JoinHandle<()>>,
}

impl DiskScheduler {
    pub fn new(disk_manager: Arc<Mutex<DiskManager>>) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<Option<DiskRequest>>();
        let dm = Arc::clone(&disk_manager);

        let worker_handle = thread::spawn(move || {
            while let Ok(Some(mut request)) = request_rx.recv() {
                let mut dm = dm.lock().unwrap();

                match request.r#type {
                    DiskRequestType::Read => {
                        match dm.read_page(request.page_id, &mut request.data[..])  {
                            Ok(_) => { let _ = request.promise.send(Some(request.data));}
                            Err(_) => { let _ = request.promise.send(None);}
                        }
                    }
                    DiskRequestType::Write => {
                        match dm.write_page(request.page_id, &request.data[..]) {
                            Ok(_) => { let _ = request.promise.send(Some(request.data));}
                            Err(_) => { let _ = request.promise.send(None);}
                        }
                    }
                    DiskRequestType::Delete => {
                        match dm.delete_page(request.page_id) {
                            Ok(_) => { let _ = request.promise.send(Some(request.data));}
                            Err(_) => { let _ = request.promise.send(None);}
                        }
                    }
                }
            }
        });

        Self {
            request_tx,
            disk_manager,
            worker_handle: Some(worker_handle),
        }
    }

    pub fn schedule(&self, request: DiskRequest) {
        self.request_tx.send(Some(request)).unwrap();
    }
}

impl Drop for DiskScheduler {
    fn drop(&mut self) {
        let _ = self.request_tx.send(None);

        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod disk_scheduler_tests {
    use super::*;

    #[test]
    fn test_basic_read_write_operations() {
        let test_file = "test.db";
        let _ = remove_file(test_file);
        let dm = Arc::new(Mutex::new(DiskManager::new(test_file)));
        let scheduler = DiskScheduler::new(dm);


        let page_data = b"Hello world".repeat(PAGE_SIZE / 10);
        let page_data = &page_data[..PAGE_SIZE];

        let (tx1, rx1 ) = mpsc::channel::<Option<Vec<u8>>>();
        let req1 = DiskRequest {
           r#type: DiskRequestType::Write,
           page_id: 0,
           data: page_data.to_vec(),
           promise: tx1,
        };

        scheduler.schedule(req1);
        assert_eq!(rx1.recv().unwrap(), Some(page_data.to_vec()));

        let (tx2, rx2) = mpsc::channel::<Option<Vec<u8>>>();
        let req2 = DiskRequest {
            r#type: DiskRequestType::Read,
            page_id: 1,
            data: page_data.to_vec(),
            promise: tx2,
        };

        scheduler.schedule(req2);
        assert_eq!(rx2.recv().unwrap(), Some(page_data.to_vec()));

        let _ = remove_file(test_file);
    }
}
