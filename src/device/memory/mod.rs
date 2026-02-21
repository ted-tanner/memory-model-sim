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

    /// Returns the next lower level in the hierarchy (if any).
    /// For caches this is the backing device; for main memory this is None.
    fn backing_memory(&self) -> Option<&dyn MemoryDevice> {
        None
    }
}

mod set_associative_cache;
pub use set_associative_cache::{CacheTiming, SetAssociativeCache};

mod main_memory;
pub use main_memory::{MainMemory, MainMemoryTiming};

#[cfg(test)]
mod integration_tests {
    use std::rc::Rc;

    use crate::device::Clock;

    use super::*;

    #[test]
    fn test_three_level_writeback_eventually_reaches_main_memory() {
        let mem = MainMemory::new(1024);
        let l3 = SetAssociativeCache::new(1, 1, 16, &mem);
        let l2 = SetAssociativeCache::new(1, 1, 8, &l3);
        let l1 = SetAssociativeCache::new(1, 1, 4, &l2);

        // L3 is non-inclusive by default in this simulator, so we do not wire L3 -> L2.
        // Keep L2 -> L1 inclusive.
        l2.set_invalidation_listener(&l1);

        l1.store_u8(0, 0xAB);
        assert_eq!(mem.load_u8(0), 0);

        // Force enough unique lines to evict line 0 from L1, then L2, then L3.
        for addr in 1..40 {
            let _ = l1.load_u8(addr);
        }

        assert_eq!(mem.load_u8(0), 0xAB);
    }

    #[test]
    fn test_three_level_hierarchy_reads_latest_after_eviction_chain() {
        let mem = MainMemory::new(1024);
        let l3 = SetAssociativeCache::new(1, 1, 16, &mem);
        let l2 = SetAssociativeCache::new(1, 1, 8, &l3);
        let l1 = SetAssociativeCache::new(1, 1, 4, &l2);

        l2.set_invalidation_listener(&l1);

        l1.store_u8(0, 0x5A);
        for addr in 1..40 {
            let _ = l1.load_u8(addr);
        }

        // Value should be observable through full hierarchy after writeback propagation.
        assert_eq!(l1.load_u8(0), 0x5A);
        assert_eq!(l2.load_u8(0), 0x5A);
        assert_eq!(l3.load_u8(0), 0x5A);
        assert_eq!(mem.load_u8(0), 0x5A);
    }

    #[test]
    fn test_clock_accumulates_across_l1_l2_l3_main_memory() {
        let clock = Rc::new(Clock::new());
        let mem = MainMemory::new(16)
            .with_clock(clock.clone())
            .with_timing(MainMemoryTiming {
                load: 100,
                store: 100,
            });
        let l3 = SetAssociativeCache::new(1, 1, 16, &mem)
            .with_clock(clock.clone())
            .with_timing(CacheTiming {
                load_hit: 3,
                load_miss: 30,
                store_hit: 3,
                store_miss: 30,
                write_back: 7,
                invalidation_send: 1,
                invalidation_apply: 1,
            });
        let l2 = SetAssociativeCache::new(1, 1, 8, &l3)
            .with_clock(clock.clone())
            .with_timing(CacheTiming {
                load_hit: 2,
                load_miss: 20,
                store_hit: 2,
                store_miss: 20,
                write_back: 5,
                invalidation_send: 1,
                invalidation_apply: 1,
            });
        let l1 = SetAssociativeCache::new(1, 1, 4, &l2)
            .with_clock(clock.clone())
            .with_timing(CacheTiming {
                load_hit: 1,
                load_miss: 10,
                store_hit: 1,
                store_miss: 10,
                write_back: 3,
                invalidation_send: 1,
                invalidation_apply: 1,
            });

        l2.set_invalidation_listener(&l1);

        // First access is a cold miss through all levels:
        // L1 miss (10) + L2 miss (20) + L3 miss (30) + main memory load (100) = 160
        let _ = l1.load_u8(0);
        assert_eq!(clock.curr_tick(), 160);

        // Second access hits in L1 only (+1).
        let _ = l1.load_u8(0);
        assert_eq!(clock.curr_tick(), 161);
    }

    #[test]
    fn test_non_inclusive_l3_eviction_does_not_invalidate_l2() {
        let mem = MainMemory::new(1024);
        let l3 = SetAssociativeCache::new(1, 1, 16, &mem);
        let l2 = SetAssociativeCache::new(1, 1, 8, &l3);

        // Intentionally no l3 -> l2 invalidation wiring (non-inclusive L3).
        assert_eq!(l2.load_u8(0), 0);
        l2.store_u8(0, 0xCC);
        assert_eq!(l2.load_u8(0), 0xCC);

        // Evict line 0 from L3 by touching many unique lines directly through L3.
        // L3 is 16-way; 17+ unique lines force eviction in this 1-set config.
        for addr in 1..20 {
            let _ = l3.load_u8(addr);
        }

        // L2 should still retain its dirty line even if L3 evicted the same address.
        assert_eq!(l2.load_u8(0), 0xCC);
        // Main memory is still old value because L2 has not been forced to write back yet.
        assert_eq!(mem.load_u8(0), 0);
    }

    #[test]
    fn test_invalidation_is_line_granular_not_byte_granular() {
        // Small 1-way caches to make eviction deterministic.
        let mem = MainMemory::new(128);
        let l2: SetAssociativeCache<'_> = SetAssociativeCache::new(16, 1, 1, &mem);
        let l1: SetAssociativeCache<'_> = SetAssociativeCache::new(16, 1, 1, &l2);
        l2.set_invalidation_listener(&l1);

        mem.store_u8(7, 1);

        // Bring line base 0 into L1/L2.
        assert_eq!(l1.load_u8(7), 1);

        // Mutate backing memory directly at another byte in same line.
        // If L1 still kept any stale byte after invalidation, we'd read the old value.
        mem.store_u8(7, 2);

        // Evict line base 0 from L2 by touching a different line base (16),
        // which should invalidate the entire line in L1.
        let _ = l2.load_u8(16);

        // Reload from L1: should miss and fetch fresh value (2), proving line-level invalidation.
        assert_eq!(l1.load_u8(7), 2);
    }
}
