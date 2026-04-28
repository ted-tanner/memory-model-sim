use std::{cell::RefCell, rc::Rc};

use crate::device::Clock;
use crate::device::cache_trace::{
    CacheAccessEvent, CacheAccessKind, CacheAccessSource, SharedCacheTrace,
};
use crate::device::memory::{CacheTiming, InvalidationListener, MemoryDevice};
use crate::device::secdcp_memory::{SecurityClass, SecurityClassControl};

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

type CacheSets = Box<[Box<[Option<CacheLine>]>]>;

struct SliceState {
    owner: Option<SecurityClass>,
    last_used: usize,
    cache: CacheSets,
}

pub struct SmtCacheMemory<'a> {
    line_size: usize,
    num_sets: usize,
    associativity: usize,
    slices: RefCell<Box<[SliceState]>>,
    active_domain: RefCell<SecurityClass>,
    requester_pid: RefCell<u32>,
    requester_domain_id: RefCell<u32>,
    use_counter: RefCell<usize>,
    backing_memory: &'a dyn MemoryDevice,
    clock: Option<Rc<Clock>>,
    timing: CacheTiming,
    trace: Option<SharedCacheTrace>,
}

impl<'a> SmtCacheMemory<'a> {
    pub fn new(
        line_size: usize,
        num_sets: usize,
        associativity: usize,
        slice_count: usize,
        backing_memory: &'a dyn MemoryDevice,
    ) -> Self {
        debug_assert!(line_size > 0, "SMTCache line size must be > 0");
        debug_assert!(num_sets > 0, "SMTCache num_sets must be > 0");
        debug_assert!(associativity > 0, "SMTCache associativity must be > 0");
        debug_assert!(slice_count > 0, "SMTCache slice_count must be > 0");

        let slices = (0..slice_count)
            .map(|_| SliceState {
                owner: None,
                last_used: 0,
                cache: Self::empty_cache(num_sets, associativity),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Self {
            line_size,
            num_sets,
            associativity,
            slices: RefCell::new(slices),
            active_domain: RefCell::new(SecurityClass::High),
            requester_pid: RefCell::new(0),
            requester_domain_id: RefCell::new(0),
            use_counter: RefCell::new(0),
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

    fn empty_cache(num_sets: usize, associativity: usize) -> CacheSets {
        (0..num_sets)
            .map(|_| {
                (0..associativity)
                    .map(|_| None)
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            })
            .collect::<Vec<_>>()
            .into_boxed_slice()
    }

    fn active_domain(&self) -> SecurityClass {
        *self.active_domain.borrow()
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
            owner_pid: *self.requester_pid.borrow(),
            owner_domain: *self.requester_domain_id.borrow(),
        }
    }

    fn write_back_line(&self, line: &CacheLine) {
        for (i, &byte) in line.data.iter().enumerate() {
            self.backing_memory.store_u8(line.base_addr + i, byte);
        }
    }

    fn flush_slice(&self, slice: &mut SliceState) -> u64 {
        let mut writebacks = 0;
        for set in slice.cache.iter_mut() {
            for slot in set.iter_mut() {
                if let Some(line) = slot.take()
                    && line.dirty
                {
                    self.write_back_line(&line);
                    self.tick(self.timing.write_back);
                    writebacks += 1;
                }
            }
        }
        writebacks
    }

    fn active_slice_index(&self, domain: SecurityClass, use_count: usize) -> usize {
        let mut slices = self.slices.borrow_mut();
        if let Some(idx) = slices.iter().position(|slice| slice.owner == Some(domain)) {
            slices[idx].last_used = use_count;
            return idx;
        }

        if let Some(idx) = slices.iter().position(|slice| slice.owner.is_none()) {
            slices[idx].owner = Some(domain);
            slices[idx].last_used = use_count;
            return idx;
        }

        let victim = slices
            .iter()
            .enumerate()
            .min_by_key(|(_, slice)| slice.last_used)
            .map(|(idx, _)| idx)
            .unwrap();
        let old_owner = slices[victim].owner;
        let writebacks = self.flush_slice(&mut slices[victim]);
        slices[victim].owner = Some(domain);
        slices[victim].last_used = use_count;
        drop(slices);

        if let Some(trace) = &self.trace {
            trace.record(CacheAccessEvent {
                architecture: "smtcache",
                requester: domain,
                requester_pid: *self.requester_pid.borrow(),
                requester_domain: *self.requester_domain_id.borrow(),
                kind: CacheAccessKind::SliceReassign,
                addr: 0,
                set: None,
                hit: false,
                source: CacheAccessSource::Lower,
                evicted_owner: old_owner,
                evicted_pid: None,
                evicted_domain: None,
                evicted_addr: None,
                slice: Some(victim),
                writebacks,
            });
        }

        victim
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

    fn record_access(
        &self,
        requester: SecurityClass,
        kind: CacheAccessKind,
        addr: usize,
        hit: bool,
        slice: usize,
        evicted_line: Option<&CacheLine>,
    ) {
        if let Some(trace) = &self.trace {
            trace.record(CacheAccessEvent {
                architecture: "smtcache",
                requester,
                requester_pid: *self.requester_pid.borrow(),
                requester_domain: *self.requester_domain_id.borrow(),
                kind,
                addr,
                set: Some(self.set_index(addr)),
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
                slice: Some(slice),
                writebacks: evicted_line.is_some_and(|line| line.dirty) as u64,
            });
        }
    }

    fn load_u8_internal(&self, addr: usize, charge_timing: bool) -> u8 {
        let requester = self.active_domain();
        let use_count = self.next_use_counter();
        let slice_idx = self.active_slice_index(requester, use_count);
        let set_idx = self.set_index(addr);
        let base_addr = self.line_base(addr);
        let offset = self.offset_in_line(addr);

        let (victim_way, evicted_line) = {
            let mut slices = self.slices.borrow_mut();
            let set = &mut slices[slice_idx].cache[set_idx];
            for way in 0..self.associativity {
                if set[way]
                    .as_ref()
                    .is_some_and(|line| line.base_addr == base_addr)
                {
                    let line = set[way].as_mut().unwrap();
                    line.last_used = use_count;
                    let byte = line.data[offset];
                    drop(slices);
                    if charge_timing {
                        self.tick(self.timing.load_hit);
                    }
                    self.record_access(
                        requester,
                        CacheAccessKind::Load,
                        addr,
                        true,
                        slice_idx,
                        None,
                    );
                    return byte;
                }
            }
            let victim_way = self.find_victim(set);
            let evicted_line = set[victim_way].take();
            (victim_way, evicted_line)
        };

        if let Some(old) = evicted_line.as_ref()
            && old.dirty
        {
            self.write_back_line(old);
            self.tick(self.timing.write_back);
        }

        let new_line = self.fill_line_from_backing(base_addr, use_count, requester);
        let byte = new_line.data[offset];
        self.slices.borrow_mut()[slice_idx].cache[set_idx][victim_way] = Some(new_line);
        if charge_timing {
            self.tick(self.timing.load_miss);
        }
        self.record_access(
            requester,
            CacheAccessKind::Load,
            addr,
            false,
            slice_idx,
            evicted_line.as_ref(),
        );
        byte
    }

    fn store_u8_internal(&self, addr: usize, value: u8, charge_timing: bool) {
        let requester = self.active_domain();
        let use_count = self.next_use_counter();
        let slice_idx = self.active_slice_index(requester, use_count);
        let set_idx = self.set_index(addr);
        let base_addr = self.line_base(addr);
        let offset = self.offset_in_line(addr);

        let (victim_way, evicted_line) = {
            let mut slices = self.slices.borrow_mut();
            let set = &mut slices[slice_idx].cache[set_idx];
            for way in 0..self.associativity {
                if set[way]
                    .as_ref()
                    .is_some_and(|line| line.base_addr == base_addr)
                {
                    let line = set[way].as_mut().unwrap();
                    line.data[offset] = value;
                    line.dirty = true;
                    line.last_used = use_count;
                    drop(slices);
                    if charge_timing {
                        self.tick(self.timing.store_hit);
                    }
                    self.record_access(
                        requester,
                        CacheAccessKind::Store,
                        addr,
                        true,
                        slice_idx,
                        None,
                    );
                    return;
                }
            }
            let victim_way = self.find_victim(set);
            let evicted_line = set[victim_way].take();
            (victim_way, evicted_line)
        };

        if let Some(old) = evicted_line.as_ref()
            && old.dirty
        {
            self.write_back_line(old);
            self.tick(self.timing.write_back);
        }

        let mut new_line = self.fill_line_from_backing(base_addr, use_count, requester);
        new_line.data[offset] = value;
        new_line.dirty = true;
        self.slices.borrow_mut()[slice_idx].cache[set_idx][victim_way] = Some(new_line);
        if charge_timing {
            self.tick(self.timing.store_miss);
        }
        self.record_access(
            requester,
            CacheAccessKind::Store,
            addr,
            false,
            slice_idx,
            evicted_line.as_ref(),
        );
    }
}

impl<'a> SecurityClassControl for SmtCacheMemory<'a> {
    fn set_requester_class(&self, class: SecurityClass) {
        *self.active_domain.borrow_mut() = class;
    }

    fn set_requester_identity(&self, class: SecurityClass, pid: u32, domain: u32) {
        *self.active_domain.borrow_mut() = class;
        *self.requester_pid.borrow_mut() = pid;
        *self.requester_domain_id.borrow_mut() = domain;
    }
}

impl<'a> InvalidationListener for SmtCacheMemory<'a> {
    fn invalidate_line(&self, base_addr: usize) {
        let mut removed = Vec::new();
        {
            let mut slices = self.slices.borrow_mut();
            for slice in slices.iter_mut() {
                for set in slice.cache.iter_mut() {
                    for slot in set.iter_mut() {
                        if slot
                            .as_ref()
                            .is_some_and(|line| line.base_addr == base_addr)
                            && let Some(line) = slot.take()
                        {
                            removed.push(line);
                        }
                    }
                }
            }
        }
        for line in removed {
            if line.dirty {
                self.write_back_line(&line);
                self.tick(self.timing.write_back);
            }
        }
    }
}

impl<'a> MemoryDevice for SmtCacheMemory<'a> {
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
    use crate::device::cache_trace::CacheTrace;
    use crate::device::memory::MainMemory;

    use super::*;

    #[test]
    fn different_domains_do_not_evict_each_other() {
        let mem = MainMemory::new(4096);
        let cache = SmtCacheMemory::new(64, 1, 1, 2, &mem);

        cache.set_requester_class(SecurityClass::High);
        let _ = cache.load_u8(0);
        cache.set_requester_class(SecurityClass::Low);
        let _ = cache.load_u8(64);
        cache.set_requester_class(SecurityClass::High);

        let trace = CacheTrace::new_shared();
        let cache = SmtCacheMemory::new(64, 1, 1, 2, &mem).with_trace(trace.clone());
        cache.set_requester_class(SecurityClass::High);
        let _ = cache.load_u8(0);
        cache.set_requester_class(SecurityClass::Low);
        let _ = cache.load_u8(64);
        cache.set_requester_class(SecurityClass::High);
        let _ = cache.load_u8(0);

        let events = trace.drain();
        assert!(events.last().is_some_and(|event| event.hit));
    }
}
