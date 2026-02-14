/// Called by a lower-level cache when it evicts a line, so upper-level caches
/// can invalidate that line and maintain an inclusive hierarchy (e.g. L1 ⊆ L2).
pub trait InvalidationListener {
    fn invalidate_line(&self, base_addr: usize);
}

pub trait MemoryDevice {
    fn load_u8(&self, addr: usize) -> u8;
    fn store_u8(&self, addr: usize, n: u8);

    fn load_u16(&self, addr: usize) -> u16;
    fn store_u16(&self, addr: usize, n: u16);

    fn load_u32(&self, addr: usize) -> u32;
    fn store_u32(&self, addr: usize, n: u32);

    fn load_i8(&self, addr: usize) -> i8;
    fn store_i8(&self, addr: usize, n: i8);

    fn load_i16(&self, addr: usize) -> i16;
    fn store_i16(&self, addr: usize, n: i16);

    fn load_i32(&self, addr: usize) -> i32;
    fn store_i32(&self, addr: usize, n: i32);
}

mod cache;
pub use cache::{Cache, L1Cache, L2Cache, L3Cache};

mod main_memory;
pub use main_memory::MainMemory;
