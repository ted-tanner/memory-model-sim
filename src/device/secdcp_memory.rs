use std::{cell::RefCell, ops::Range, rc::Rc};

use crate::device::Clock;

use super::memory::{CacheTiming, MemoryDevice};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityClass {
    High,
    Low,
}

pub trait SecurityClassControl {
    fn set_requester_class(&self, class: SecurityClass);

    fn set_requester_identity(&self, class: SecurityClass, _pid: u32, _domain: u32) {
        self.set_requester_class(class);
    }
}

#[derive(Clone, Copy)]
pub struct SecDcpPolicy {
    pub epoch_public_accesses: u64,
    pub increase_threshold: f64,
    pub decrease_threshold: f64,
    pub min_high_ways: usize,
    pub min_low_ways: usize,
}

impl Default for SecDcpPolicy {
    fn default() -> Self {
        Self {
            epoch_public_accesses: 256,
            increase_threshold: 0.60,
            decrease_threshold: 0.10,
            min_high_ways: 1,
            min_low_ways: 1,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct EpochStats {
    public_accesses: u64,
    public_misses: u64,
}

impl EpochStats {
    fn clear(&mut self) {
        self.public_accesses = 0;
        self.public_misses = 0;
    }
}

struct CacheLine {
    base_addr: usize,
    data: Box<[u8]>,
    dirty: bool,
    last_used: usize,
    owner: SecurityClass,
}

type CacheSets = Box<[Box<[Option<CacheLine>]>]>;

pub struct SecDcpMemory<'a> {
    line_size: usize,
    num_sets: usize,
    associativity: usize,
    cache: RefCell<CacheSets>,
    use_counter: RefCell<usize>,
    backing_memory: &'a dyn MemoryDevice,
    clock: Option<Rc<Clock>>,
    timing: CacheTiming,
    policy: SecDcpPolicy,
    requester_class: RefCell<SecurityClass>,
    public_ways: RefCell<usize>,
    epoch_stats: RefCell<EpochStats>,
}

impl<'a> SecDcpMemory<'a> {
    pub fn new(
        line_size: usize,
        num_sets: usize,
        associativity: usize,
        backing_memory: &'a dyn MemoryDevice,
    ) -> Self {
        debug_assert!(line_size > 0, "SecDCP line size must be > 0");
        debug_assert!(num_sets > 0, "SecDCP num_sets must be > 0");
        debug_assert!(associativity > 1, "SecDCP associativity must be > 1");
        let cache: CacheSets = (0..num_sets)
            .map(|_| {
                (0..associativity)
                    .map(|_| None)
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Self {
            line_size,
            num_sets,
            associativity,
            cache: RefCell::new(cache),
            use_counter: RefCell::new(0),
            backing_memory,
            clock: None,
            timing: CacheTiming::default(),
            policy: SecDcpPolicy::default(),
            requester_class: RefCell::new(SecurityClass::High),
            public_ways: RefCell::new(1),
            epoch_stats: RefCell::new(EpochStats::default()),
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

    pub fn with_policy(mut self, policy: SecDcpPolicy) -> Self {
        self.policy = policy;
        let min_public = self.policy.min_low_ways.max(1);
        let max_public = self
            .associativity
            .saturating_sub(self.policy.min_high_ways)
            .max(min_public);
        *self.public_ways.borrow_mut() = min_public.min(max_public);
        self
    }

    pub fn public_ways(&self) -> usize {
        *self.public_ways.borrow()
    }

    fn public_way_range(&self) -> Range<usize> {
        0..*self.public_ways.borrow()
    }

    fn high_way_range(&self) -> Range<usize> {
        *self.public_ways.borrow()..self.associativity
    }

    fn way_range_for(&self, class: SecurityClass) -> Range<usize> {
        match class {
            SecurityClass::Low => self.public_way_range(),
            SecurityClass::High => self.high_way_range(),
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

    fn tick(&self, n: u64) {
        if let Some(clock) = &self.clock {
            clock.advance(n);
        }
    }

    fn fill_line_from_backing(
        &self,
        base_addr: usize,
        last_used: usize,
        owner: SecurityClass,
    ) -> CacheLine {
        let mut data = vec![0u8; self.line_size].into_boxed_slice();
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = self.backing_memory.load_u8(base_addr + i);
        }
        CacheLine {
            base_addr,
            data,
            dirty: false,
            last_used,
            owner,
        }
    }

    fn write_back_line(&self, line: &CacheLine) {
        for (i, &byte) in line.data.iter().enumerate() {
            self.backing_memory.store_u8(line.base_addr + i, byte);
        }
    }

    fn find_victim(&self, set: &[Option<CacheLine>], allowed_ways: Range<usize>) -> usize {
        let mut empty = None;
        let mut lru_way = allowed_ways.start;
        let mut lru_used = usize::MAX;

        for way in allowed_ways {
            match &set[way] {
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

    fn maybe_repartition_for_public_epoch(&self) {
        let should_resize = {
            let stats = self.epoch_stats.borrow();
            stats.public_accesses >= self.policy.epoch_public_accesses
        };
        if !should_resize {
            return;
        }

        let miss_rate = {
            let stats = self.epoch_stats.borrow();
            if stats.public_accesses == 0 {
                0.0
            } else {
                stats.public_misses as f64 / stats.public_accesses as f64
            }
        };

        let min_public = self.policy.min_low_ways.max(1);
        let max_public = self
            .associativity
            .saturating_sub(self.policy.min_high_ways)
            .max(min_public);

        let current_public_ways = *self.public_ways.borrow();
        let mut new_public_ways = current_public_ways;
        if miss_rate > self.policy.increase_threshold && new_public_ways < max_public {
            new_public_ways += 1;
        } else if miss_rate < self.policy.decrease_threshold && new_public_ways > min_public {
            new_public_ways -= 1;
        }

        if new_public_ways < current_public_ways {
            for reclaimed_way in new_public_ways..current_public_ways {
                self.flush_low_owned_way(reclaimed_way);
            }
        }

        *self.public_ways.borrow_mut() = new_public_ways;
        self.epoch_stats.borrow_mut().clear();
    }

    fn flush_low_owned_way(&self, way: usize) {
        for set_idx in 0..self.num_sets {
            let removed = {
                let mut cache = self.cache.borrow_mut();
                let slot = &mut cache[set_idx][way];
                if slot
                    .as_ref()
                    .is_some_and(|line| line.owner == SecurityClass::Low)
                {
                    slot.take()
                } else {
                    None
                }
            };

            if let Some(line) = removed
                && line.dirty
            {
                self.write_back_line(&line);
                self.tick(self.timing.write_back);
            }
        }
    }

    fn record_public_access(&self, miss: bool) {
        let mut stats = self.epoch_stats.borrow_mut();
        stats.public_accesses += 1;
        if miss {
            stats.public_misses += 1;
        }
    }

    fn load_u8_internal(&self, addr: usize, charge_access_timing: bool) -> u8 {
        let requester = *self.requester_class.borrow();
        let set_idx = self.set_index(addr);
        let base = self.line_base(addr);
        let offset = self.offset_in_line(addr);
        let use_count = self.next_use_counter();
        let allowed_ways = self.way_range_for(requester);

        {
            let mut cache = self.cache.borrow_mut();
            let set = &mut cache[set_idx];
            for way in allowed_ways.clone() {
                if let Some(ref l) = set[way]
                    && l.base_addr == base
                {
                    set[way].as_mut().unwrap().last_used = use_count;
                    if requester == SecurityClass::Low {
                        self.record_public_access(false);
                    }
                    if charge_access_timing {
                        self.tick(self.timing.load_hit);
                    }
                    self.maybe_repartition_for_public_epoch();
                    return set[way].as_ref().unwrap().data[offset];
                }
            }
        }

        let (victim_way, evicted_line) = {
            let mut cache = self.cache.borrow_mut();
            let set = &mut cache[set_idx];
            let victim_way = self.find_victim(set, allowed_ways);
            let evicted_line = set[victim_way].take();
            (victim_way, evicted_line)
        };

        if let Some(ref old) = evicted_line
            && old.dirty
        {
            self.write_back_line(old);
            self.tick(self.timing.write_back);
        }

        let new_line = self.fill_line_from_backing(base, use_count, requester);
        let byte = new_line.data[offset];
        {
            let mut cache = self.cache.borrow_mut();
            cache[set_idx][victim_way] = Some(new_line);
        }

        if requester == SecurityClass::Low {
            self.record_public_access(true);
        }
        if charge_access_timing {
            self.tick(self.timing.load_miss);
        }
        self.maybe_repartition_for_public_epoch();
        byte
    }

    fn store_u8_internal(&self, addr: usize, n: u8, charge_access_timing: bool) {
        let requester = *self.requester_class.borrow();
        let set_idx = self.set_index(addr);
        let base = self.line_base(addr);
        let offset = self.offset_in_line(addr);
        let use_count = self.next_use_counter();
        let allowed_ways = self.way_range_for(requester);

        {
            let mut cache = self.cache.borrow_mut();
            let set = &mut cache[set_idx];
            for way in allowed_ways.clone() {
                if let Some(ref l) = set[way]
                    && l.base_addr == base
                {
                    let line = set[way].as_mut().unwrap();
                    line.data[offset] = n;
                    line.dirty = true;
                    line.last_used = use_count;
                    if requester == SecurityClass::Low {
                        self.record_public_access(false);
                    }
                    if charge_access_timing {
                        self.tick(self.timing.store_hit);
                    }
                    self.maybe_repartition_for_public_epoch();
                    return;
                }
            }
        }

        let (victim_way, evicted_line) = {
            let mut cache = self.cache.borrow_mut();
            let set = &mut cache[set_idx];
            let victim_way = self.find_victim(set, allowed_ways);
            let evicted_line = set[victim_way].take();
            (victim_way, evicted_line)
        };

        if let Some(ref old) = evicted_line
            && old.dirty
        {
            self.write_back_line(old);
            self.tick(self.timing.write_back);
        }

        let mut new_line = self.fill_line_from_backing(base, use_count, requester);
        new_line.data[offset] = n;
        new_line.dirty = true;
        {
            let mut cache = self.cache.borrow_mut();
            cache[set_idx][victim_way] = Some(new_line);
        }

        if requester == SecurityClass::Low {
            self.record_public_access(true);
        }
        if charge_access_timing {
            self.tick(self.timing.store_miss);
        }
        self.maybe_repartition_for_public_epoch();
    }

    #[cfg(test)]
    fn debug_force_public_ways(&self, new_public_ways: usize) {
        let current = *self.public_ways.borrow();
        if new_public_ways < current {
            for reclaimed_way in new_public_ways..current {
                self.flush_low_owned_way(reclaimed_way);
            }
        }
        *self.public_ways.borrow_mut() = new_public_ways;
    }

    #[cfg(test)]
    fn debug_line_owner(&self, set_idx: usize, way: usize) -> Option<SecurityClass> {
        self.cache.borrow()[set_idx][way]
            .as_ref()
            .map(|line| line.owner)
    }
}

impl<'a> SecurityClassControl for SecDcpMemory<'a> {
    fn set_requester_class(&self, class: SecurityClass) {
        *self.requester_class.borrow_mut() = class;
    }
}

impl<'a> MemoryDevice for SecDcpMemory<'a> {
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

    fn backing_memory(&self) -> Option<&dyn MemoryDevice> {
        Some(self.backing_memory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::memory::MainMemory;

    #[test]
    fn public_demand_can_grow_partition_but_high_demand_cannot() {
        let mem = MainMemory::new(64);
        let cache = SecDcpMemory::new(1, 1, 4, &mem).with_policy(SecDcpPolicy {
            epoch_public_accesses: 4,
            increase_threshold: 0.5,
            decrease_threshold: 0.0,
            min_high_ways: 1,
            min_low_ways: 1,
        });

        cache.set_requester_class(SecurityClass::High);
        for addr in 0..4 {
            let _ = cache.load_u8(addr);
        }
        assert_eq!(cache.public_ways(), 1);

        cache.set_requester_class(SecurityClass::Low);
        for addr in 16..20 {
            let _ = cache.load_u8(addr);
        }
        assert_eq!(cache.public_ways(), 2);
    }

    #[test]
    fn shrinking_public_partition_flushes_only_low_owned_lines() {
        let mem = MainMemory::new(64);
        let cache = SecDcpMemory::new(1, 1, 4, &mem);

        cache.debug_force_public_ways(2);
        cache.set_requester_class(SecurityClass::Low);
        cache.store_u8(0, 0xAA);
        cache.store_u8(1, 0xBB);
        assert_eq!(cache.debug_line_owner(0, 0), Some(SecurityClass::Low));
        assert_eq!(cache.debug_line_owner(0, 1), Some(SecurityClass::Low));

        cache.set_requester_class(SecurityClass::High);
        let _ = cache.load_u8(2);
        assert_eq!(cache.debug_line_owner(0, 2), Some(SecurityClass::High));

        cache.debug_force_public_ways(1);

        assert_eq!(mem.load_u8(1), 0xBB);
        assert_eq!(cache.debug_line_owner(0, 1), None);
        assert_eq!(cache.debug_line_owner(0, 2), Some(SecurityClass::High));
    }
}
