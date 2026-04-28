use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use crate::device::Clock;
use crate::device::cache_trace::{
    CacheAccessEvent, CacheAccessKind, CacheAccessSource, SharedCacheTrace,
};

use super::memory::{CacheTiming, InvalidationListener, MemoryDevice};
use super::secdcp_memory::{SecurityClass, SecurityClassControl};

#[derive(Clone)]
struct CacheLine {
    base_addr: usize,
    data: Box<[u8]>,
    dirty: bool,
    last_used: usize,
    owner: SecurityClass,
    owner_pid: u32,
    owner_domain: u32,
}

#[derive(Default)]
struct DomainState {
    mapping: BTreeMap<usize, usize>,
}

pub struct NewCacheMemory<'a> {
    line_size: usize,
    total_lines: usize,
    cache: RefCell<Box<[Option<CacheLine>]>>,
    domains: RefCell<[DomainState; 2]>,
    active_domain: RefCell<SecurityClass>,
    requester_pid: RefCell<u32>,
    requester_domain_id: RefCell<u32>,
    use_counter: RefCell<usize>,
    rng_state: RefCell<u64>,
    backing_memory: &'a dyn MemoryDevice,
    clock: Option<Rc<Clock>>,
    timing: CacheTiming,
    trace: Option<SharedCacheTrace>,
}

impl<'a> NewCacheMemory<'a> {
    pub fn new(
        line_size: usize,
        num_sets: usize,
        associativity: usize,
        backing_memory: &'a dyn MemoryDevice,
    ) -> Self {
        debug_assert!(line_size > 0, "NewCache line size must be > 0");
        debug_assert!(num_sets > 0, "NewCache num_sets must be > 0");
        debug_assert!(associativity > 0, "NewCache associativity must be > 0");
        let total_lines = num_sets * associativity;
        let cache = (0..total_lines)
            .map(|_| None)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            line_size,
            total_lines,
            cache: RefCell::new(cache),
            domains: RefCell::new([DomainState::default(), DomainState::default()]),
            active_domain: RefCell::new(SecurityClass::High),
            requester_pid: RefCell::new(0),
            requester_domain_id: RefCell::new(0),
            use_counter: RefCell::new(0),
            rng_state: RefCell::new(0x6e65_7763_6163_6865),
            backing_memory,
            clock: None,
            timing: CacheTiming::default(),
            trace: None,
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

    pub fn with_trace(mut self, trace: SharedCacheTrace) -> Self {
        self.trace = Some(trace);
        self
    }

    fn domain_index(domain: SecurityClass) -> usize {
        match domain {
            SecurityClass::High => 0,
            SecurityClass::Low => 1,
        }
    }

    fn active_domain(&self) -> SecurityClass {
        *self.active_domain.borrow()
    }

    fn line_base(&self, addr: usize) -> usize {
        (addr / self.line_size) * self.line_size
    }

    fn next_use_counter(&self) -> usize {
        let mut counter = self.use_counter.borrow_mut();
        let current = *counter;
        *counter = counter.saturating_add(1);
        current
    }

    fn tick(&self, n: u64) {
        if let Some(clock) = &self.clock {
            clock.advance(n);
        }
    }

    fn random_u64(&self) -> u64 {
        let mut state = self.rng_state.borrow_mut();
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    fn random_index(&self, len: usize) -> usize {
        debug_assert!(len > 0);
        (self.random_u64() as usize) % len
    }

    fn fill_line_from_backing(&self, base_addr: usize, owner: SecurityClass) -> CacheLine {
        let mut data = vec![0u8; self.line_size].into_boxed_slice();
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = self.backing_memory.load_u8(base_addr + i);
        }
        CacheLine {
            base_addr,
            data,
            dirty: false,
            last_used: self.next_use_counter(),
            owner,
            owner_pid: *self.requester_pid.borrow(),
            owner_domain: *self.requester_domain_id.borrow(),
        }
    }

    fn write_back_line(&self, line: &CacheLine) {
        for (i, &byte) in line.data.iter().enumerate() {
            self.backing_memory.store_u8(line.base_addr + i, byte);
        }
    }

    fn lookup_active_line(&self, base_addr: usize) -> Option<usize> {
        let owner = self.active_domain();
        let owner_idx = Self::domain_index(owner);
        let mapped_idx = {
            let domains = self.domains.borrow();
            domains[owner_idx].mapping.get(&base_addr).copied()
        }?;

        let is_hit = {
            let cache = self.cache.borrow();
            cache[mapped_idx]
                .as_ref()
                .is_some_and(|line| line.owner == owner && line.base_addr == base_addr)
        };

        if is_hit {
            Some(mapped_idx)
        } else {
            self.domains.borrow_mut()[owner_idx]
                .mapping
                .remove(&base_addr);
            None
        }
    }

    fn choose_install_index(&self) -> usize {
        let empty_indices = {
            let cache = self.cache.borrow();
            cache
                .iter()
                .enumerate()
                .filter_map(|(idx, slot)| slot.is_none().then_some(idx))
                .collect::<Vec<_>>()
        };
        if !empty_indices.is_empty() {
            empty_indices[self.random_index(empty_indices.len())]
        } else {
            self.random_index(self.total_lines)
        }
    }

    fn install_line(&self, idx: usize, line: CacheLine) -> Option<CacheLine> {
        let evicted = {
            let mut cache = self.cache.borrow_mut();
            let old = cache[idx].take();
            cache[idx] = Some(line.clone());
            old
        };

        if let Some(old_line) = evicted.as_ref() {
            let owner_idx = Self::domain_index(old_line.owner);
            self.domains.borrow_mut()[owner_idx]
                .mapping
                .remove(&old_line.base_addr);
            if old_line.dirty {
                self.write_back_line(old_line);
                self.tick(self.timing.write_back);
            }
        }

        let owner_idx = Self::domain_index(line.owner);
        self.domains.borrow_mut()[owner_idx]
            .mapping
            .insert(line.base_addr, idx);
        evicted
    }

    fn record_access(
        &self,
        requester: SecurityClass,
        kind: CacheAccessKind,
        addr: usize,
        hit: bool,
        slot: Option<usize>,
        evicted_line: Option<&CacheLine>,
    ) {
        if let Some(trace) = &self.trace {
            trace.record(CacheAccessEvent {
                architecture: "newcache",
                requester,
                requester_pid: *self.requester_pid.borrow(),
                requester_domain: *self.requester_domain_id.borrow(),
                kind,
                addr,
                set: slot,
                hit,
                source: if hit {
                    CacheAccessSource::L1D
                } else {
                    CacheAccessSource::Lower
                },
                evicted_owner: evicted_line.map(|line| line.owner),
                evicted_pid: evicted_line.map(|line| line.owner_pid),
                evicted_domain: evicted_line.map(|line| line.owner_domain),
                evicted_addr: evicted_line.map(|line| line.base_addr),
                slice: None,
                writebacks: evicted_line.is_some_and(|line| line.dirty) as u64,
            });
        }
    }

    fn load_u8_internal(&self, addr: usize, charge_access_timing: bool) -> u8 {
        let base_addr = self.line_base(addr);
        let offset = addr - base_addr;

        if let Some(idx) = self.lookup_active_line(base_addr) {
            let use_count = self.next_use_counter();
            let byte = {
                let mut cache = self.cache.borrow_mut();
                let line = cache[idx].as_mut().unwrap();
                line.last_used = use_count;
                line.data[offset]
            };
            if charge_access_timing {
                self.tick(self.timing.load_hit);
            }
            self.record_access(
                self.active_domain(),
                CacheAccessKind::Load,
                addr,
                true,
                Some(idx),
                None,
            );
            return byte;
        }

        let owner = self.active_domain();
        let line = self.fill_line_from_backing(base_addr, self.active_domain());
        let byte = line.data[offset];
        let idx = self.choose_install_index();
        let evicted = self.install_line(idx, line);
        if charge_access_timing {
            self.tick(self.timing.load_miss);
        }
        self.record_access(
            owner,
            CacheAccessKind::Load,
            addr,
            false,
            Some(idx),
            evicted.as_ref(),
        );
        byte
    }

    fn store_u8_internal(&self, addr: usize, value: u8, charge_access_timing: bool) {
        let base_addr = self.line_base(addr);
        let offset = addr - base_addr;

        if let Some(idx) = self.lookup_active_line(base_addr) {
            let use_count = self.next_use_counter();
            {
                let mut cache = self.cache.borrow_mut();
                let line = cache[idx].as_mut().unwrap();
                line.last_used = use_count;
                line.dirty = true;
                line.data[offset] = value;
            }
            if charge_access_timing {
                self.tick(self.timing.store_hit);
            }
            self.record_access(
                self.active_domain(),
                CacheAccessKind::Store,
                addr,
                true,
                Some(idx),
                None,
            );
            return;
        }

        let owner = self.active_domain();
        let mut line = self.fill_line_from_backing(base_addr, self.active_domain());
        line.dirty = true;
        line.data[offset] = value;
        let idx = self.choose_install_index();
        let evicted = self.install_line(idx, line);
        if charge_access_timing {
            self.tick(self.timing.store_miss);
        }
        self.record_access(
            owner,
            CacheAccessKind::Store,
            addr,
            false,
            Some(idx),
            evicted.as_ref(),
        );
    }

    #[cfg(test)]
    fn debug_lookup_for(&self, domain: SecurityClass, base_addr: usize) -> Option<usize> {
        let domains = self.domains.borrow();
        domains[Self::domain_index(domain)]
            .mapping
            .get(&base_addr)
            .copied()
    }
}

impl<'a> SecurityClassControl for NewCacheMemory<'a> {
    fn set_requester_class(&self, class: SecurityClass) {
        *self.active_domain.borrow_mut() = class;
    }

    fn set_requester_identity(&self, class: SecurityClass, pid: u32, domain: u32) {
        *self.active_domain.borrow_mut() = class;
        *self.requester_pid.borrow_mut() = pid;
        *self.requester_domain_id.borrow_mut() = domain;
    }
}

impl<'a> InvalidationListener for NewCacheMemory<'a> {
    fn invalidate_line(&self, base_addr: usize) {
        let removed = {
            let mut cache = self.cache.borrow_mut();
            let mut removed = None;
            for slot in cache.iter_mut() {
                if slot
                    .as_ref()
                    .is_some_and(|line| line.base_addr == base_addr)
                {
                    removed = slot.take();
                    break;
                }
            }
            removed
        };

        if let Some(line) = removed {
            let owner_idx = Self::domain_index(line.owner);
            self.domains.borrow_mut()[owner_idx]
                .mapping
                .remove(&line.base_addr);
            if line.dirty {
                self.write_back_line(&line);
                self.tick(self.timing.write_back);
            }
            self.tick(self.timing.invalidation_apply);
        }
    }
}

impl<'a> MemoryDevice for NewCacheMemory<'a> {
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
    use std::rc::Rc;

    use crate::device::Clock;
    use crate::device::memory::{MainMemory, MainMemoryTiming};

    use super::*;

    #[test]
    fn separate_domains_can_hold_the_same_line_independently() {
        let mem = MainMemory::new(256);
        let cache = NewCacheMemory::new(16, 1, 4, &mem);

        cache.set_requester_class(SecurityClass::High);
        let _ = cache.load_u8(0);
        let high_idx = cache.debug_lookup_for(SecurityClass::High, 0).unwrap();

        cache.set_requester_class(SecurityClass::Low);
        let _ = cache.load_u8(0);
        let low_idx = cache.debug_lookup_for(SecurityClass::Low, 0).unwrap();

        assert_ne!(high_idx, low_idx);
    }

    #[test]
    fn invalidation_removes_resident_line() {
        let mem = MainMemory::new(256);
        let cache = NewCacheMemory::new(16, 1, 4, &mem);
        cache.set_requester_class(SecurityClass::High);
        let _ = cache.load_u8(0);
        assert!(cache.debug_lookup_for(SecurityClass::High, 0).is_some());

        cache.invalidate_line(0);
        assert!(cache.debug_lookup_for(SecurityClass::High, 0).is_none());
    }

    #[test]
    fn clock_treats_hits_and_misses_like_l1() {
        let clock = Rc::new(Clock::new());
        let mem = MainMemory::new(256)
            .with_clock(clock.clone())
            .with_timing(MainMemoryTiming {
                load: 100,
                store: 100,
            });
        let cache = NewCacheMemory::new(1, 1, 4, &mem)
            .with_clock(clock.clone())
            .with_timing(CacheTiming {
                load_hit: 4,
                load_miss: 20,
                store_hit: 4,
                store_miss: 20,
                write_back: 8,
                invalidation_send: 1,
                invalidation_apply: 1,
            });

        let _ = cache.load_u8(0);
        assert_eq!(clock.curr_tick(), 120);
        let _ = cache.load_u8(0);
        assert_eq!(clock.curr_tick(), 124);
    }
}
