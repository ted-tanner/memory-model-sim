use std::{cell::RefCell, rc::Rc};

use crate::device::Clock;

use super::MemoryDevice;

#[derive(Clone, Copy)]
pub struct MainMemoryTiming {
    pub load: u64,
    pub store: u64,
}

impl Default for MainMemoryTiming {
    fn default() -> Self {
        Self {
            load: 100,
            store: 100,
        }
    }
}

pub struct MainMemory {
    buf: RefCell<Box<[u8]>>,
    size: usize,
    clock: Option<Rc<Clock>>,
    timing: MainMemoryTiming,
}

impl MainMemory {
    pub fn new(size: usize) -> Self {
        Self {
            buf: RefCell::new(vec![0; size].into_boxed_slice()),
            size,
            clock: None,
            timing: MainMemoryTiming::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub fn with_clock(mut self, clock: Rc<Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    pub fn with_timing(mut self, timing: MainMemoryTiming) -> Self {
        self.timing = timing;
        self
    }

    fn tick_load(&self) {
        if let Some(clock) = &self.clock {
            clock.advance(self.timing.load);
        }
    }

    fn tick_store(&self) {
        if let Some(clock) = &self.clock {
            clock.advance(self.timing.store);
        }
    }
}

impl MemoryDevice for MainMemory {
    fn load_u8(&self, addr: usize) -> u8 {
        self.tick_load();
        self.buf.borrow()[addr]
    }

    fn store_u8(&self, addr: usize, n: u8) {
        self.tick_store();
        self.buf.borrow_mut()[addr] = n;
    }

    fn load_u16(&self, addr: usize) -> u16 {
        self.tick_load();
        u16::from_le_bytes([self.buf.borrow()[addr], self.buf.borrow()[addr + 1]])
    }

    fn store_u16(&self, addr: usize, n: u16) {
        self.tick_store();
        let bytes = n.to_le_bytes();
        self.buf.borrow_mut()[addr..addr + 2].copy_from_slice(&bytes);
    }

    fn load_u32(&self, addr: usize) -> u32 {
        self.tick_load();
        let b = self.buf.borrow();
        u32::from_le_bytes([b[addr], b[addr + 1], b[addr + 2], b[addr + 3]])
    }

    fn store_u32(&self, addr: usize, n: u32) {
        self.tick_store();
        self.buf.borrow_mut()[addr..addr + 4].copy_from_slice(&n.to_le_bytes());
    }

    fn load_i8(&self, addr: usize) -> i8 {
        self.tick_load();
        self.buf.borrow()[addr] as i8
    }

    fn store_i8(&self, addr: usize, n: i8) {
        self.tick_store();
        self.buf.borrow_mut()[addr] = n as u8;
    }

    fn load_i16(&self, addr: usize) -> i16 {
        self.tick_load();
        i16::from_le_bytes([self.buf.borrow()[addr], self.buf.borrow()[addr + 1]])
    }

    fn store_i16(&self, addr: usize, n: i16) {
        self.tick_store();
        self.buf.borrow_mut()[addr..addr + 2].copy_from_slice(&n.to_le_bytes());
    }

    fn load_i32(&self, addr: usize) -> i32 {
        self.tick_load();
        i32::from_le_bytes([
            self.buf.borrow()[addr],
            self.buf.borrow()[addr + 1],
            self.buf.borrow()[addr + 2],
            self.buf.borrow()[addr + 3],
        ])
    }

    fn store_i32(&self, addr: usize, n: i32) {
        self.tick_store();
        self.buf.borrow_mut()[addr..addr + 4].copy_from_slice(&n.to_le_bytes());
    }

    fn debug_load_u8_no_timing(&self, addr: usize) -> u8 {
        self.buf.borrow()[addr]
    }

    fn debug_load_u32_no_timing(&self, addr: usize) -> u32 {
        let b = self.buf.borrow();
        u32::from_le_bytes([b[addr], b[addr + 1], b[addr + 2], b[addr + 3]])
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use crate::device::Clock;

    use super::super::MemoryDevice;
    use super::{MainMemory, MainMemoryTiming};

    #[test]
    fn test_load_store_u8() {
        let mem = MainMemory::new(32);

        mem.store_u8(20, 42);
        assert_eq!(mem.load_u8(20), 42);

        mem.store_u8(3, 3);
        mem.store_u8(17, 6);
        mem.store_u8(30, 9);
        assert_eq!(mem.load_u8(20), 42);
        assert_eq!(mem.load_u8(3), 3);
        assert_eq!(mem.load_u8(17), 6);
        assert_eq!(mem.load_u8(30), 9);
    }

    #[test]
    fn test_load_store_u16_u32() {
        let mem = MainMemory::new(32);

        mem.store_u16(0, 0x1234);
        assert_eq!(mem.load_u16(0), 0x1234);
        assert_eq!(mem.load_u8(0), 0x34);
        assert_eq!(mem.load_u8(1), 0x12);

        mem.store_u32(10, 0xDEAD_BEEF);
        assert_eq!(mem.load_u32(10), 0xDEAD_BEEF);
    }

    #[test]
    fn test_load_store_i8_i16_i32() {
        let mem = MainMemory::new(32);

        mem.store_i8(0, -1);
        assert_eq!(mem.load_i8(0), -1);
        assert_eq!(mem.load_u8(0), 0xFF);

        mem.store_i16(4, -0x1234);
        assert_eq!(mem.load_i16(4), -0x1234);

        mem.store_i32(8, i32::MIN);
        assert_eq!(mem.load_i32(8), i32::MIN);
        mem.store_i32(12, 0xDEAD_BEEFu32 as i32);
        assert_eq!(mem.load_i32(12), 0xDEAD_BEEFu32 as i32);
    }

    #[test]
    fn test_timing_advances_clock() {
        let clock = Rc::new(Clock::new());
        let mem = MainMemory::new(32)
            .with_clock(clock.clone())
            .with_timing(MainMemoryTiming { load: 7, store: 11 });

        mem.store_u8(0, 1);
        mem.load_u8(0);
        mem.store_u32(4, 0xDEAD_BEEF);
        mem.load_i16(4);

        // 2 stores + 2 loads
        assert_eq!(clock.curr_tick(), 2 * 11 + 2 * 7);
    }
}
