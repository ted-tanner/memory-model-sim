use std::cell::RefCell;

use super::MemoryDevice;

struct CacheLine {
    base_addr: usize,
    data: Box<[u8]>,
    dirty: bool,
    last_used: usize,
}

pub struct Cache<'a, const ASSOCIATIVITY: usize, M: MemoryDevice> {
    line_size: usize,
    num_sets: usize,
    cache: RefCell<Box<[Box<[Option<CacheLine>]>]>>,
    use_counter: RefCell<usize>,
    backing_memory: &'a M,
}

impl<'a, const ASSOCIATIVITY: usize, M: MemoryDevice> Cache<'a, ASSOCIATIVITY, M> {
    pub fn new(line_size: usize, num_sets: usize, backing_memory: &'a M) -> Self {
        let cache: Box<[Box<[Option<CacheLine>]>]> = (0..num_sets)
            .map(|_| {
                (0..ASSOCIATIVITY)
                    .map(|_| None)
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            line_size,
            num_sets,
            cache: RefCell::new(cache),
            use_counter: RefCell::new(0),
            backing_memory,
        }
    }

    #[inline]
    fn set_index(&self, addr: usize) -> usize {
        (addr / self.line_size) % self.num_sets
    }

    #[inline]
    fn line_base(&self, addr: usize) -> usize {
        (addr / self.line_size) * self.line_size
    }

    #[inline]
    fn offset_in_line(&self, addr: usize) -> usize {
        addr % self.line_size
    }

    fn next_use_counter(&self) -> usize {
        let mut c = self.use_counter.borrow_mut();
        let v = *c;
        *c = c.saturating_add(1);
        v
    }

    fn fill_line_from_backing(&self, base_addr: usize, last_used: usize) -> CacheLine {
        let mut data = vec![0u8; self.line_size].into_boxed_slice();
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = self.backing_memory.load_u8(base_addr + i);
        }
        CacheLine {
            base_addr,
            data,
            dirty: false,
            last_used,
        }
    }

    fn write_back_line(&self, line: &CacheLine) {
        for (i, &byte) in line.data.iter().enumerate() {
            self.backing_memory.store_u8(line.base_addr + i, byte);
        }
    }

    fn find_victim(&self, set: &[Option<CacheLine>]) -> usize {
        let mut empty = None;
        let mut lru_way = 0;
        let mut lru_used = usize::MAX;
        for (way, slot) in set.iter().enumerate() {
            match slot {
                None => {
                    empty = Some(way);
                    break;
                }
                Some(line) if line.last_used < lru_used => {
                    lru_used = line.last_used;
                    lru_way = way;
                }
                _ => {}
            }
        }
        empty.unwrap_or(lru_way)
    }
}

impl<'a, const ASSOCIATIVITY: usize, M: MemoryDevice> MemoryDevice for Cache<'a, ASSOCIATIVITY, M> {
    fn load_u8(&self, addr: usize) -> u8 {
        let set_idx = self.set_index(addr);
        let base = self.line_base(addr);
        let offset = self.offset_in_line(addr);
        let use_count = self.next_use_counter();

        let mut cache = self.cache.borrow_mut();
        let set = &mut cache[set_idx];

        for way in 0..ASSOCIATIVITY {
            if let Some(ref l) = set[way] {
                if l.base_addr == base {
                    set[way].as_mut().unwrap().last_used = use_count;
                    return set[way].as_ref().unwrap().data[offset];
                }
            }
        }

        let victim_way = self.find_victim(set);
        if let Some(ref old) = set[victim_way] {
            if old.dirty {
                self.write_back_line(old);
            }
        }
        let mut new_line = self.fill_line_from_backing(base, use_count);
        let byte = new_line.data[offset];
        set[victim_way] = Some(new_line);
        byte
    }

    fn store_u8(&self, addr: usize, n: u8) {
        let set_idx = self.set_index(addr);
        let base = self.line_base(addr);
        let offset = self.offset_in_line(addr);
        let use_count = self.next_use_counter();

        let mut cache = self.cache.borrow_mut();
        let set = &mut cache[set_idx];

        for way in 0..ASSOCIATIVITY {
            if let Some(ref l) = set[way] {
                if l.base_addr == base {
                    set[way].as_mut().unwrap().data[offset] = n;
                    set[way].as_mut().unwrap().dirty = true;
                    set[way].as_mut().unwrap().last_used = use_count;
                    return;
                }
            }
        }

        let victim_way = self.find_victim(set);
        if let Some(ref old) = set[victim_way] {
            if old.dirty {
                self.write_back_line(old);
            }
        }
        let mut new_line = self.fill_line_from_backing(base, use_count);
        new_line.data[offset] = n;
        new_line.dirty = true;
        set[victim_way] = Some(new_line);
    }

    fn load_u16(&self, addr: usize) -> u16 {
        let lo = self.load_u8(addr) as u16;
        let hi = self.load_u8(addr + 1) as u16;
        lo | (hi << 8)
    }

    fn store_u16(&self, addr: usize, n: u16) {
        self.store_u8(addr, n as u8);
        self.store_u8(addr + 1, (n >> 8) as u8);
    }

    fn load_u32(&self, addr: usize) -> u32 {
        let b0 = self.load_u8(addr) as u32;
        let b1 = self.load_u8(addr + 1) as u32;
        let b2 = self.load_u8(addr + 2) as u32;
        let b3 = self.load_u8(addr + 3) as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }

    fn store_u32(&self, addr: usize, n: u32) {
        self.store_u8(addr, n as u8);
        self.store_u8(addr + 1, (n >> 8) as u8);
        self.store_u8(addr + 2, (n >> 16) as u8);
        self.store_u8(addr + 3, (n >> 24) as u8);
    }

    fn load_i8(&self, addr: usize) -> i8 {
        self.load_u8(addr) as i8
    }

    fn store_i8(&self, addr: usize, n: i8) {
        self.store_u8(addr, n as u8);
    }

    fn load_i16(&self, addr: usize) -> i16 {
        self.load_u16(addr) as i16
    }

    fn store_i16(&self, addr: usize, n: i16) {
        self.store_u16(addr, n as u16);
    }

    fn load_i32(&self, addr: usize) -> i32 {
        self.load_u32(addr) as i32
    }

    fn store_i32(&self, addr: usize, n: i32) {
        self.store_u32(addr, n as u32);
    }
}

pub type L1Cache<'a, M> = Cache<'a, 4, M>;
pub type L2Cache<'a, M> = Cache<'a, 8, M>;

#[cfg(test)]
mod tests {
    use super::super::MainMemory;
    use super::super::MemoryDevice;
    use super::{Cache, L1Cache, L2Cache};

    #[test]
    fn test_l1_load_store_hit() {
        let mem = MainMemory::new(256);
        mem.store_u8(10, 42);
        mem.store_u8(11, 43);

        let cache = L1Cache::new(16, 1, &mem);

        assert_eq!(cache.load_u8(10), 42);
        assert_eq!(cache.load_u8(11), 43);
        cache.store_u8(10, 100);
        assert_eq!(cache.load_u8(10), 100);
        assert_eq!(mem.load_u8(10), 42);
    }

    #[test]
    fn test_l1_write_back_on_eviction() {
        let mem = MainMemory::new(256);
        mem.store_u8(0, 1);
        mem.store_u8(16, 2);
        mem.store_u8(32, 3);
        mem.store_u8(64, 4);
        mem.store_u8(128, 5);

        let cache = L1Cache::new(16, 1, &mem);
        assert_eq!(cache.load_u8(0), 1);
        cache.store_u8(0, 10);
        assert_eq!(cache.load_u8(16), 2);
        assert_eq!(cache.load_u8(32), 3);
        assert_eq!(cache.load_u8(64), 4);
        assert_eq!(mem.load_u8(0), 1);
        assert_eq!(cache.load_u8(128), 5);
        assert_eq!(mem.load_u8(0), 10);
        assert_eq!(mem.load_u8(16), 2);
    }

    #[test]
    fn test_l1_u16_u32() {
        let mem = MainMemory::new(256);
        mem.store_u32(0, 0xDEAD_BEEF);
        mem.store_u32(8, 0xCAFE_BABE);

        let cache = L1Cache::new(32, 1, &mem);
        assert_eq!(cache.load_u32(0), 0xDEAD_BEEF);
        assert_eq!(cache.load_u32(8), 0xCAFE_BABE);
        cache.store_u32(0, 0xCAFE_BABE);
        assert_eq!(cache.load_u32(0), 0xCAFE_BABE);
        assert_eq!(mem.load_u32(0), 0xDEAD_BEEF);
        cache.load_u8(32);
        cache.load_u8(64);
        cache.load_u8(96);
        cache.load_u8(128);
        assert_eq!(mem.load_u32(0), 0xCAFE_BABE);
    }

    #[test]
    fn test_l1_i8_i16_i32() {
        let mem = MainMemory::new(256);
        let neg_hex = 0xDEAD_BEEFu32 as i32;
        mem.store_i32(0, neg_hex);
        mem.store_i16(8, -42);

        let cache = L1Cache::new(32, 1, &mem);
        assert_eq!(cache.load_i32(0), neg_hex);
        assert_eq!(cache.load_i16(8), -42);
        cache.store_i32(0, i32::MAX);
        assert_eq!(cache.load_i32(0), i32::MAX);
        assert_eq!(mem.load_i32(0), neg_hex);
        cache.load_u8(32);
        cache.load_u8(64);
        cache.load_u8(96);
        cache.load_u8(128);
        assert_eq!(mem.load_i32(0), i32::MAX);
    }

    #[test]
    fn test_l2_load_store_hit() {
        let mem = MainMemory::new(256);
        mem.store_u8(10, 42);
        mem.store_u8(11, 43);

        let cache = L2Cache::new(16, 4, &mem);
        assert_eq!(cache.load_u8(10), 42);
        assert_eq!(cache.load_u8(11), 43);
        cache.store_u8(10, 100);
        assert_eq!(cache.load_u8(10), 100);
        assert_eq!(mem.load_u8(10), 42);
    }

    #[test]
    fn test_l2_set_associative_eviction() {
        let mem = MainMemory::new(512);
        for (i, &addr) in [0, 32, 64, 96, 128, 160, 192, 224, 256].iter().enumerate() {
            mem.store_u8(addr, i as u8 + 1);
        }

        let cache = L2Cache::new(16, 2, &mem);
        assert_eq!(cache.load_u8(0), 1);
        cache.store_u8(0, 10);
        for &addr in &[32, 64, 96, 128, 160, 192, 224] {
            let _ = cache.load_u8(addr);
        }
        assert_eq!(mem.load_u8(0), 1);
        assert_eq!(cache.load_u8(256), 9);
        assert_eq!(mem.load_u8(0), 10);
    }

    #[test]
    fn test_l2_write_back_on_eviction() {
        let mem = MainMemory::new(256);
        for addr in (0..128).step_by(16) {
            mem.store_u8(addr, (addr / 16) as u8 + 1);
        }
        mem.store_u8(128, 9);

        let cache = L2Cache::new(16, 1, &mem);
        assert_eq!(cache.load_u8(0), 1);
        cache.store_u8(0, 10);
        for addr in (16..128).step_by(16) {
            let _ = cache.load_u8(addr);
        }
        assert_eq!(mem.load_u8(0), 1);
        assert_eq!(cache.load_u8(128), 9);
        assert_eq!(mem.load_u8(0), 10);
        assert_eq!(mem.load_u8(16), 2);
        assert_eq!(mem.load_u8(128), 9);
    }

    #[test]
    fn test_cache_const_generic_2way() {
        let mem = MainMemory::new(64);
        mem.store_u8(0, 1);
        mem.store_u8(16, 2);
        mem.store_u8(32, 3);

        let cache: Cache<'_, 2, _> = Cache::new(16, 1, &mem);
        assert_eq!(cache.load_u8(0), 1);
        cache.store_u8(0, 10);
        assert_eq!(cache.load_u8(16), 2);
        assert_eq!(mem.load_u8(0), 1);
        assert_eq!(cache.load_u8(32), 3);
        assert_eq!(mem.load_u8(0), 10);
    }
}
