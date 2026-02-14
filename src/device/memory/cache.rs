use std::{cell::RefCell, rc::Rc};

use crate::device::Clock;

use super::{InvalidationListener, MemoryDevice};

struct CacheLine {
    base_addr: usize,
    data: Box<[u8]>,
    dirty: bool,
    last_used: usize,
}

type CacheSets = Box<[Box<[Option<CacheLine>]>]>;

#[derive(Clone, Copy)]
pub struct CacheTiming {
    pub load_hit: u64,
    pub load_miss: u64,
    pub store_hit: u64,
    pub store_miss: u64,
    pub write_back: u64,
    pub invalidation_send: u64,
    pub invalidation_apply: u64,
}

impl Default for CacheTiming {
    fn default() -> Self {
        Self {
            load_hit: 1,
            load_miss: 4,
            store_hit: 1,
            store_miss: 4,
            write_back: 2,
            invalidation_send: 1,
            invalidation_apply: 1,
        }
    }
}

pub struct Cache<'a, const ASSOCIATIVITY: usize, M: MemoryDevice> {
    line_size: usize,
    num_sets: usize,
    cache: RefCell<CacheSets>,
    use_counter: RefCell<usize>,
    backing_memory: &'a M,
    clock: Option<Rc<Clock>>,
    timing: CacheTiming,
    invalidation_listener: RefCell<Option<&'a dyn InvalidationListener>>,
    /// Deferred invalidations to avoid re-entrant borrows when listener is called during a backing access.
    pending_invalidations: RefCell<Vec<usize>>,
}

impl<'a, const ASSOCIATIVITY: usize, M: MemoryDevice> Cache<'a, ASSOCIATIVITY, M> {
    pub fn new(line_size: usize, num_sets: usize, backing_memory: &'a M) -> Self {
        debug_assert!(ASSOCIATIVITY > 0, "cache associativity must be > 0");
        debug_assert!(line_size > 0, "cache line_size must be > 0");
        debug_assert!(num_sets > 0, "cache num_sets must be > 0");
        let cache: CacheSets = (0..num_sets)
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
            clock: None,
            timing: CacheTiming::default(),
            invalidation_listener: RefCell::new(None),
            pending_invalidations: RefCell::new(Vec::new()),
        }
    }

    pub fn with_clock(mut self, clock: Rc<Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    pub fn with_timing(mut self, timing: CacheTiming) -> Self {
        self.timing = timing;
        self
    }

    /// Registers a listener for inclusive hierarchies. When this cache evicts a line,
    /// it will call `listener.invalidate_line(base_addr)` so the upper-level cache
    /// can remove that line (maintaining L1 ⊆ L2).
    pub fn set_invalidation_listener(&self, listener: &'a dyn InvalidationListener) {
        *self.invalidation_listener.borrow_mut() = Some(listener);
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

    fn tick(&self, n: u64) {
        if let Some(clock) = &self.clock {
            clock.advance(n);
        }
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

    fn apply_pending_invalidations(&self, cache: &mut CacheSets) {
        let addrs = std::mem::take(&mut *self.pending_invalidations.borrow_mut());
        for base_addr in addrs {
            let set_idx = self.set_index(base_addr);
            let set = &mut cache[set_idx];
            for way in 0..ASSOCIATIVITY {
                if set[way].as_ref().is_some_and(|l| l.base_addr == base_addr) {
                    if let Some(ref line) = set[way]
                        && line.dirty
                    {
                        self.write_back_line(line);
                        self.tick(self.timing.write_back);
                    }
                    set[way] = None;
                    // Charge invalidation cost only when an actual line is invalidated.
                    self.tick(self.timing.invalidation_apply);
                    break;
                }
            }
        }
    }

    fn load_u8_internal(&self, addr: usize, charge_access_timing: bool) -> u8 {
        let set_idx = self.set_index(addr);
        let base = self.line_base(addr);
        let offset = self.offset_in_line(addr);
        let use_count = self.next_use_counter();

        let mut cache = self.cache.borrow_mut();
        self.apply_pending_invalidations(&mut cache);
        let set = &mut cache[set_idx];

        for way in 0..ASSOCIATIVITY {
            if let Some(ref l) = set[way]
                && l.base_addr == base
            {
                set[way].as_mut().unwrap().last_used = use_count;
                if charge_access_timing {
                    self.tick(self.timing.load_hit);
                }
                return set[way].as_ref().unwrap().data[offset];
            }
        }

        let victim_way = self.find_victim(set);
        let evicted_base = set[victim_way].as_ref().map(|l| l.base_addr);
        if let Some(ref old) = set[victim_way]
            && old.dirty
        {
            self.write_back_line(old);
            self.tick(self.timing.write_back);
        }
        let new_line = self.fill_line_from_backing(base, use_count);
        let byte = new_line.data[offset];
        set[victim_way] = Some(new_line);
        drop(cache);
        if let Some(addr) = evicted_base
            && let Some(listener) = self.invalidation_listener.borrow().as_ref()
        {
            listener.invalidate_line(addr);
            self.tick(self.timing.invalidation_send);
        }
        if charge_access_timing {
            self.tick(self.timing.load_miss);
        }
        byte
    }

    fn store_u8_internal(&self, addr: usize, n: u8, charge_access_timing: bool) {
        let set_idx = self.set_index(addr);
        let base = self.line_base(addr);
        let offset = self.offset_in_line(addr);
        let use_count = self.next_use_counter();

        let mut cache = self.cache.borrow_mut();
        self.apply_pending_invalidations(&mut cache);
        let set = &mut cache[set_idx];

        for way in 0..ASSOCIATIVITY {
            if let Some(ref l) = set[way]
                && l.base_addr == base
            {
                set[way].as_mut().unwrap().data[offset] = n;
                set[way].as_mut().unwrap().dirty = true;
                set[way].as_mut().unwrap().last_used = use_count;
                if charge_access_timing {
                    self.tick(self.timing.store_hit);
                }
                return;
            }
        }

        let victim_way = self.find_victim(set);
        let evicted_base = set[victim_way].as_ref().map(|l| l.base_addr);
        if let Some(ref old) = set[victim_way]
            && old.dirty
        {
            self.write_back_line(old);
            self.tick(self.timing.write_back);
        }
        let mut new_line = self.fill_line_from_backing(base, use_count);
        new_line.data[offset] = n;
        new_line.dirty = true;
        set[victim_way] = Some(new_line);
        drop(cache);
        if let Some(addr) = evicted_base
            && let Some(listener) = self.invalidation_listener.borrow().as_ref()
        {
            listener.invalidate_line(addr);
            self.tick(self.timing.invalidation_send);
        }
        if charge_access_timing {
            self.tick(self.timing.store_miss);
        }
    }
}

impl<'a, const ASSOCIATIVITY: usize, M: MemoryDevice> MemoryDevice for Cache<'a, ASSOCIATIVITY, M> {
    fn load_u8(&self, addr: usize) -> u8 {
        self.load_u8_internal(addr, true)
    }

    fn store_u8(&self, addr: usize, n: u8) {
        self.store_u8_internal(addr, n, true);
    }

    fn load_u16(&self, addr: usize) -> u16 {
        let lo = self.load_u8_internal(addr, true) as u16;
        let hi = self.load_u8_internal(addr + 1, self.line_base(addr + 1) != self.line_base(addr))
            as u16;
        lo | (hi << 8)
    }

    fn store_u16(&self, addr: usize, n: u16) {
        self.store_u8_internal(addr, n as u8, true);
        self.store_u8_internal(
            addr + 1,
            (n >> 8) as u8,
            self.line_base(addr + 1) != self.line_base(addr),
        );
    }

    fn load_u32(&self, addr: usize) -> u32 {
        let b0 = self.load_u8_internal(addr, true) as u32;
        let b1 = self.load_u8_internal(addr + 1, self.line_base(addr + 1) != self.line_base(addr))
            as u32;
        let b2 = self.load_u8_internal(
            addr + 2,
            self.line_base(addr + 2) != self.line_base(addr + 1),
        ) as u32;
        let b3 = self.load_u8_internal(
            addr + 3,
            self.line_base(addr + 3) != self.line_base(addr + 2),
        ) as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }

    fn store_u32(&self, addr: usize, n: u32) {
        self.store_u8_internal(addr, n as u8, true);
        self.store_u8_internal(
            addr + 1,
            (n >> 8) as u8,
            self.line_base(addr + 1) != self.line_base(addr),
        );
        self.store_u8_internal(
            addr + 2,
            (n >> 16) as u8,
            self.line_base(addr + 2) != self.line_base(addr + 1),
        );
        self.store_u8_internal(
            addr + 3,
            (n >> 24) as u8,
            self.line_base(addr + 3) != self.line_base(addr + 2),
        );
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

impl<'a, const ASSOCIATIVITY: usize, M: MemoryDevice> InvalidationListener
    for Cache<'a, ASSOCIATIVITY, M>
{
    fn invalidate_line(&self, base_addr: usize) {
        self.pending_invalidations.borrow_mut().push(base_addr);
    }
}

pub type L1Cache<'a, M> = Cache<'a, 4, M>;
pub type L2Cache<'a, M> = Cache<'a, 8, M>;
pub type L3Cache<'a, M> = Cache<'a, 16, M>;

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use crate::device::Clock;

    use super::super::{InvalidationListener, MemoryDevice};
    use super::super::{MainMemory, MainMemoryTiming};
    use super::{Cache, CacheTiming, L1Cache, L2Cache, L3Cache};

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

    #[test]
    fn test_inclusive_l2_eviction_invalidates_l1() {
        let mem = MainMemory::new(512);
        mem.store_u8(0, 1);
        for &addr in &[16, 32, 48, 64, 80, 96, 112, 128, 144] {
            mem.store_u8(addr, 2);
        }

        let l2 = L2Cache::new(16, 1, &mem);
        let l1: Cache<'_, 8, _> = Cache::new(16, 1, &l2);
        l2.set_invalidation_listener(&l1);

        assert_eq!(l1.load_u8(0), 1);
        for &addr in &[16, 32, 48, 64, 80, 96, 112] {
            let _ = l1.load_u8(addr);
        }
        l1.load_u8(128);
        l1.load_u8(144);

        // L2 evicted line 0 and invalidated L1, so the next load through L1 sees backing (1), not a stale copy.
        assert_eq!(l1.load_u8(0), 1);
        assert_eq!(mem.load_u8(0), 1);
    }

    #[test]
    fn test_l3_load_store() {
        let mem = MainMemory::new(1024);
        mem.store_u8(0, 100);
        mem.store_u32(64, 0xDEAD_BEEF);

        let l3 = L3Cache::new(64, 4, &mem);
        assert_eq!(l3.load_u8(0), 100);
        assert_eq!(l3.load_u32(64), 0xDEAD_BEEF);
        l3.store_u8(0, 200);
        assert_eq!(l3.load_u8(0), 200);
        assert_eq!(mem.load_u8(0), 100);
    }

    #[test]
    fn test_cache_timing_advances_clock() {
        let clock = Rc::new(Clock::new());
        let mem = MainMemory::new(8)
            .with_clock(clock.clone())
            .with_timing(MainMemoryTiming {
                load: 20,
                store: 30,
            });
        let cache: Cache<'_, 1, _> = Cache::new(1, 1, &mem)
            .with_clock(clock.clone())
            .with_timing(CacheTiming {
                load_hit: 1,
                load_miss: 3,
                store_hit: 2,
                store_miss: 4,
                write_back: 5,
                invalidation_send: 7,
                invalidation_apply: 11,
            });

        cache.load_u8(0); // miss: 3 + mem load 20
        cache.load_u8(0); // hit: 1
        cache.store_u8(0, 9); // hit: 2
        cache.load_u8(1); // miss with dirty eviction: 5 + 30 + 3 + 20

        assert_eq!(clock.curr_tick(), 84);
    }

    #[test]
    fn test_dirty_line_writeback_on_invalidation_apply() {
        let mem = MainMemory::new(16);
        let cache: Cache<'_, 1, _> = Cache::new(1, 1, &mem);

        cache.load_u8(0);
        cache.store_u8(0, 77);
        cache.invalidate_line(0);

        // Applying pending invalidations should flush dirty line 0.
        let _ = cache.load_u8(1);
        assert_eq!(mem.load_u8(0), 77);
    }

    #[test]
    fn test_u32_timing_charged_per_line_not_per_byte() {
        let clock = Rc::new(Clock::new());
        let mem = MainMemory::new(32)
            .with_clock(clock.clone())
            .with_timing(MainMemoryTiming { load: 0, store: 0 });
        let cache: Cache<'_, 2, _> = Cache::new(8, 1, &mem)
            .with_clock(clock.clone())
            .with_timing(CacheTiming {
                load_hit: 2,
                load_miss: 5,
                store_hit: 0,
                store_miss: 0,
                write_back: 0,
                invalidation_send: 0,
                invalidation_apply: 0,
            });

        // Cold same-line u32 read should charge one miss.
        let _ = cache.load_u32(0);
        assert_eq!(clock.curr_tick(), 5);

        // Warm same-line u32 read should charge one hit.
        let _ = cache.load_u32(0);
        assert_eq!(clock.curr_tick(), 7);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "cache line_size must be > 0")]
    fn test_debug_assert_line_size_zero() {
        let mem = MainMemory::new(8);
        let _cache: Cache<'_, 1, _> = Cache::new(0, 1, &mem);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "cache num_sets must be > 0")]
    fn test_debug_assert_num_sets_zero() {
        let mem = MainMemory::new(8);
        let _cache: Cache<'_, 1, _> = Cache::new(1, 0, &mem);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "cache associativity must be > 0")]
    fn test_debug_assert_associativity_zero() {
        let mem = MainMemory::new(8);
        let _cache: Cache<'_, 0, _> = Cache::new(1, 1, &mem);
    }
}
