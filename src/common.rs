pub type PageId = u32;
pub type FrameId = u32;

pub const PAGE_SIZE: usize = 8192;
pub const DB_IO_SIZE: usize = 16;
pub const INVALID_PAGE_ID: PageId = u32::MAX;
