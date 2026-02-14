use std::cell::RefCell;

use super::MemoryDevice;

pub struct MainMemory {
    buf: RefCell<Box<[u8]>>,
    size: usize,
}

impl MainMemory {
    pub fn new(size: usize) -> Self {
        Self {
            buf: RefCell::new(vec![0; size].into_boxed_slice()),
            size,
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }
}

impl MemoryDevice for MainMemory {
    fn load_u8(&self, addr: usize) -> u8 {
        self.buf.borrow()[addr]
    }

    fn store_u8(&self, addr: usize, n: u8) {
        self.buf.borrow_mut()[addr] = n;
    }

    fn load_u16(&self, addr: usize) -> u16 {
        u16::from_le_bytes([self.buf.borrow()[addr], self.buf.borrow()[addr + 1]])
    }

    fn store_u16(&self, addr: usize, n: u16) {
        let bytes = n.to_le_bytes();
        self.buf.borrow_mut()[addr..addr + 2].copy_from_slice(&bytes);
    }

    fn load_u32(&self, addr: usize) -> u32 {
        let b = self.buf.borrow();
        u32::from_le_bytes([b[addr], b[addr + 1], b[addr + 2], b[addr + 3]])
    }

    fn store_u32(&self, addr: usize, n: u32) {
        self.buf.borrow_mut()[addr..addr + 4].copy_from_slice(&n.to_le_bytes());
    }

    fn load_i8(&self, addr: usize) -> i8 {
        self.buf.borrow()[addr] as i8
    }

    fn store_i8(&self, addr: usize, n: i8) {
        self.buf.borrow_mut()[addr] = n as u8;
    }

    fn load_i16(&self, addr: usize) -> i16 {
        i16::from_le_bytes([self.buf.borrow()[addr], self.buf.borrow()[addr + 1]])
    }

    fn store_i16(&self, addr: usize, n: i16) {
        self.buf.borrow_mut()[addr..addr + 2].copy_from_slice(&n.to_le_bytes());
    }

    fn load_i32(&self, addr: usize) -> i32 {
        i32::from_le_bytes([
            self.buf.borrow()[addr],
            self.buf.borrow()[addr + 1],
            self.buf.borrow()[addr + 2],
            self.buf.borrow()[addr + 3],
        ])
    }

    fn store_i32(&self, addr: usize, n: i32) {
        self.buf.borrow_mut()[addr..addr + 4].copy_from_slice(&n.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::super::MemoryDevice;
    use super::MainMemory;

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
}
