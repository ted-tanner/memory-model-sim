use std::{cell::RefCell, rc::Rc};

use crate::device::{Clock, ContextSwitchListener};

use super::memory::{CacheTiming, InvalidationListener, MemoryDevice};

#[derive(Clone, Copy, Debug)]
pub struct BackCachePolicy {
    pub min_enabled_lines: usize,
    pub max_enabled_lines: usize,
}

impl Default for BackCachePolicy {
    fn default() -> Self {
        Self {
            min_enabled_lines: 128,
            max_enabled_lines: 512,
        }
    }
}

#[derive(Clone)]
struct PrimaryLine {
    base_addr: usize,
    data: Box<[u8]>,
    dirty: bool,
    last_used: usize,
}

#[derive(Clone)]
struct BackupLine {
    base_addr: usize,
    data: Box<[u8]>,
    dirty: bool,
    last_used: usize,
    used: bool,
}

#[derive(Default)]
struct BackupSlot {
    enabled: bool,
    line: Option<BackupLine>,
}

type PrimarySets = Box<[Box<[Option<PrimaryLine>]>]>;
type BackupSlots = Box<[BackupSlot]>;

pub struct BackCacheMemory<'a> {
    line_size: usize,
    num_sets: usize,
    associativity: usize,
    primary: RefCell<PrimarySets>,
    backup: RefCell<BackupSlots>,
    use_counter: RefCell<usize>,
    rng_state: RefCell<u64>,
    resize_countdown: RefCell<usize>,
    backing_memory: &'a dyn MemoryDevice,
    clock: Option<Rc<Clock>>,
    timing: CacheTiming,
    policy: BackCachePolicy,
}

impl<'a> BackCacheMemory<'a> {
    pub fn new(
        line_size: usize,
        num_sets: usize,
        associativity: usize,
        backing_memory: &'a dyn MemoryDevice,
    ) -> Self {
        debug_assert!(line_size > 0, "BackCache line size must be > 0");
        debug_assert!(num_sets > 0, "BackCache num_sets must be > 0");
        debug_assert!(associativity > 0, "BackCache associativity must be > 0");

        let total_lines = num_sets * associativity;
        let primary: PrimarySets = (0..num_sets)
            .map(|_| {
                (0..associativity)
                    .map(|_| None)
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let backup: BackupSlots = (0..total_lines)
            .map(|_| BackupSlot::default())
            .collect::<Vec<_>>()
            .into_boxed_slice();

        let policy = BackCachePolicy::default();
        let initial_enabled = policy.max_enabled_lines.min(total_lines).max(1);

        let cache = Self {
            line_size,
            num_sets,
            associativity,
            primary: RefCell::new(primary),
            backup: RefCell::new(backup),
            use_counter: RefCell::new(0),
            rng_state: RefCell::new(0x4f1b_bc53_9e37_79b9),
            resize_countdown: RefCell::new(initial_enabled),
            backing_memory,
            clock: None,
            timing: CacheTiming::default(),
            policy,
        };
        cache.set_enabled_lines(initial_enabled);
        cache
    }

    pub fn with_clock(mut self, clock: Rc<Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    pub fn with_timing(mut self, timing: CacheTiming) -> Self {
        self.timing = timing;
        self
    }

    pub fn with_policy(mut self, policy: BackCachePolicy) -> Self {
        self.policy = policy;
        let enabled = self.random_enabled_count();
        self.set_enabled_lines(enabled);
        *self.resize_countdown.borrow_mut() = enabled;
        self
    }

    fn backup_capacity(&self) -> usize {
        self.num_sets * self.associativity
    }

    fn normalized_policy_bounds(&self) -> (usize, usize) {
        let capacity = self.backup_capacity();
        let min = self.policy.min_enabled_lines.clamp(1, capacity);
        let max = self.policy.max_enabled_lines.clamp(min, capacity);
        (min, max)
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

    fn random_enabled_count(&self) -> usize {
        let (min, max) = self.normalized_policy_bounds();
        min + self.random_index(max - min + 1)
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

    fn fill_line_from_backing(&self, base_addr: usize, last_used: usize) -> PrimaryLine {
        let mut data = vec![0u8; self.line_size].into_boxed_slice();
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = self.backing_memory.load_u8(base_addr + i);
        }
        PrimaryLine {
            base_addr,
            data,
            dirty: false,
            last_used,
        }
    }

    fn write_back_primary_line(&self, line: &PrimaryLine) {
        for (i, &byte) in line.data.iter().enumerate() {
            self.backing_memory.store_u8(line.base_addr + i, byte);
        }
    }

    fn write_back_backup_line(&self, line: &BackupLine) {
        for (i, &byte) in line.data.iter().enumerate() {
            self.backing_memory.store_u8(line.base_addr + i, byte);
        }
    }

    fn find_primary_way(&self, set_idx: usize, base_addr: usize) -> Option<usize> {
        let primary = self.primary.borrow();
        (0..self.associativity).find(|&way| {
            primary[set_idx][way]
                .as_ref()
                .is_some_and(|line| line.base_addr == base_addr)
        })
    }

    fn primary_lru_victim(&self, set_idx: usize) -> usize {
        let primary = self.primary.borrow();
        let set = &primary[set_idx];
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

    fn install_primary_line(&self, line: PrimaryLine) -> Option<PrimaryLine> {
        let set_idx = self.set_index(line.base_addr);
        let way = self.primary_lru_victim(set_idx);
        let mut primary = self.primary.borrow_mut();
        let evicted = primary[set_idx][way].take();
        primary[set_idx][way] = Some(line);
        evicted
    }

    fn find_backup_slot(&self, base_addr: usize) -> Option<usize> {
        let backup = self.backup.borrow();
        backup.iter().enumerate().find_map(|(idx, slot)| {
            if slot.enabled
                && slot
                    .line
                    .as_ref()
                    .is_some_and(|line| line.base_addr == base_addr)
            {
                Some(idx)
            } else {
                None
            }
        })
    }

    fn enabled_backup_indices(&self) -> Vec<usize> {
        let backup = self.backup.borrow();
        backup
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| slot.enabled.then_some(idx))
            .collect()
    }

    fn select_backup_victim(&self) -> usize {
        let backup = self.backup.borrow();
        let enabled: Vec<usize> = backup
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| slot.enabled.then_some(idx))
            .collect();
        debug_assert!(!enabled.is_empty());

        let empty: Vec<usize> = enabled
            .iter()
            .copied()
            .filter(|&idx| backup[idx].line.is_none())
            .collect();
        if !empty.is_empty() {
            return empty[self.random_index(empty.len())];
        }

        let all_used = enabled
            .iter()
            .all(|&idx| backup[idx].line.as_ref().is_some_and(|line| line.used));
        if all_used {
            return enabled
                .into_iter()
                .min_by_key(|&idx| backup[idx].line.as_ref().unwrap().last_used)
                .unwrap();
        }

        let used_true: Vec<usize> = enabled
            .iter()
            .copied()
            .filter(|&idx| backup[idx].line.as_ref().is_some_and(|line| line.used))
            .collect();
        if !used_true.is_empty() {
            return used_true[self.random_index(used_true.len())];
        }

        let used_false: Vec<usize> = enabled
            .iter()
            .copied()
            .filter(|&idx| backup[idx].line.as_ref().is_some_and(|line| !line.used))
            .collect();
        used_false[self.random_index(used_false.len())]
    }

    fn evict_backup_slot_if_needed(&self, slot_idx: usize) {
        let evicted = {
            let mut backup = self.backup.borrow_mut();
            backup[slot_idx].line.take()
        };
        if let Some(line) = evicted
            && line.dirty
        {
            self.write_back_backup_line(&line);
            self.tick(self.timing.write_back);
        }
    }

    fn insert_primary_eviction_into_backup(&self, line: PrimaryLine) {
        let last_used = line.last_used;
        if let Some(existing_idx) = self.find_backup_slot(line.base_addr) {
            let mut backup = self.backup.borrow_mut();
            backup[existing_idx].line = Some(BackupLine {
                base_addr: line.base_addr,
                data: line.data,
                dirty: line.dirty,
                last_used,
                used: false,
            });
            return;
        }

        let enabled = self.enabled_backup_indices();
        if enabled.is_empty() {
            if line.dirty {
                self.write_back_primary_line(&line);
                self.tick(self.timing.write_back);
            }
            return;
        }

        let victim = self.select_backup_victim();
        self.evict_backup_slot_if_needed(victim);
        let mut backup = self.backup.borrow_mut();
        backup[victim].line = Some(BackupLine {
            base_addr: line.base_addr,
            data: line.data,
            dirty: line.dirty,
            last_used,
            used: false,
        });
    }

    fn access_complete(&self) {
        let resize_now = {
            let mut countdown = self.resize_countdown.borrow_mut();
            if *countdown > 1 {
                *countdown -= 1;
                false
            } else {
                true
            }
        };
        if resize_now {
            let enabled = self.random_enabled_count();
            self.set_enabled_lines(enabled);
            *self.resize_countdown.borrow_mut() = enabled;
        }
    }

    fn current_enabled_lines(&self) -> usize {
        self.backup
            .borrow()
            .iter()
            .filter(|slot| slot.enabled)
            .count()
    }

    fn choose_enabled_slot_to_disable(&self) -> usize {
        self.select_backup_victim()
    }

    fn choose_disabled_slot_to_enable(&self) -> usize {
        let backup = self.backup.borrow();
        let disabled: Vec<usize> = backup
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| (!slot.enabled).then_some(idx))
            .collect();
        disabled[self.random_index(disabled.len())]
    }

    fn set_enabled_lines(&self, target: usize) {
        let capacity = self.backup_capacity();
        let target = target.clamp(1, capacity);

        while self.current_enabled_lines() > target {
            let idx = self.choose_enabled_slot_to_disable();
            self.evict_backup_slot_if_needed(idx);
            self.backup.borrow_mut()[idx].enabled = false;
        }

        while self.current_enabled_lines() < target {
            let idx = self.choose_disabled_slot_to_enable();
            let mut backup = self.backup.borrow_mut();
            backup[idx].enabled = true;
            backup[idx].line = None;
        }
    }

    fn clear_backup_used_bits(&self) {
        let mut backup = self.backup.borrow_mut();
        for slot in backup.iter_mut() {
            if let Some(line) = slot.line.as_mut() {
                line.used = false;
            }
        }
    }

    fn invalidate_matching_backup_line(&self, base_addr: usize) {
        let idx = self.find_backup_slot(base_addr);
        if let Some(slot_idx) = idx {
            let removed = {
                let mut backup = self.backup.borrow_mut();
                backup[slot_idx].line.take()
            };
            if let Some(line) = removed {
                if line.dirty {
                    self.write_back_backup_line(&line);
                    self.tick(self.timing.write_back);
                }
                self.tick(self.timing.invalidation_apply);
            }
        }
    }

    fn invalidate_matching_primary_line(&self, base_addr: usize) {
        let set_idx = self.set_index(base_addr);
        let removed = {
            let mut primary = self.primary.borrow_mut();
            let set = &mut primary[set_idx];
            let mut removed = None;
            for way in 0..self.associativity {
                if set[way]
                    .as_ref()
                    .is_some_and(|line| line.base_addr == base_addr)
                {
                    removed = set[way].take();
                    break;
                }
            }
            removed
        };
        if let Some(line) = removed {
            if line.dirty {
                self.write_back_primary_line(&line);
                self.tick(self.timing.write_back);
            }
            self.tick(self.timing.invalidation_apply);
        }
    }

    fn load_u8_internal(&self, addr: usize, charge_timing: bool) -> u8 {
        let base_addr = self.line_base(addr);
        let set_idx = self.set_index(addr);
        let offset = self.offset_in_line(addr);
        let use_count = self.next_use_counter();

        if let Some(way) = self.find_primary_way(set_idx, base_addr) {
            let mut primary = self.primary.borrow_mut();
            let line = primary[set_idx][way].as_mut().unwrap();
            line.last_used = use_count;
            let byte = line.data[offset];
            drop(primary);

            if let Some(slot_idx) = self.find_backup_slot(base_addr) {
                let mut backup = self.backup.borrow_mut();
                let backup_line = backup[slot_idx].line.as_mut().unwrap();
                backup_line.last_used = use_count;
                backup_line.used = true;
            }

            if charge_timing {
                self.tick(self.timing.load_hit);
            }
            self.access_complete();
            return byte;
        }

        if let Some(slot_idx) = self.find_backup_slot(base_addr) {
            let backup_line = {
                let mut backup = self.backup.borrow_mut();
                let line = backup[slot_idx].line.as_mut().unwrap();
                line.last_used = use_count;
                line.used = true;
                line.clone()
            };

            let byte = backup_line.data[offset];
            let evicted = self.install_primary_line(PrimaryLine {
                base_addr: backup_line.base_addr,
                data: backup_line.data.clone(),
                dirty: backup_line.dirty,
                last_used: use_count,
            });
            if let Some(line) = evicted {
                self.insert_primary_eviction_into_backup(line);
            }

            if charge_timing {
                self.tick(self.timing.load_hit);
            }
            self.access_complete();
            return byte;
        }

        let new_line = self.fill_line_from_backing(base_addr, use_count);
        let byte = new_line.data[offset];
        let evicted = self.install_primary_line(new_line);
        if let Some(line) = evicted {
            self.insert_primary_eviction_into_backup(line);
        }
        if charge_timing {
            self.tick(self.timing.load_miss);
        }
        self.access_complete();
        byte
    }

    fn store_u8_internal(&self, addr: usize, value: u8, charge_timing: bool) {
        let base_addr = self.line_base(addr);
        let set_idx = self.set_index(addr);
        let offset = self.offset_in_line(addr);
        let use_count = self.next_use_counter();

        if let Some(way) = self.find_primary_way(set_idx, base_addr) {
            {
                let mut primary = self.primary.borrow_mut();
                let line = primary[set_idx][way].as_mut().unwrap();
                line.data[offset] = value;
                line.dirty = true;
                line.last_used = use_count;
            }

            if let Some(slot_idx) = self.find_backup_slot(base_addr) {
                let mut backup = self.backup.borrow_mut();
                let line = backup[slot_idx].line.as_mut().unwrap();
                line.data[offset] = value;
                line.dirty = true;
                line.last_used = use_count;
                line.used = true;
            }

            if charge_timing {
                self.tick(self.timing.store_hit);
            }
            self.access_complete();
            return;
        }

        let mut refill = if let Some(slot_idx) = self.find_backup_slot(base_addr) {
            let mut backup = self.backup.borrow_mut();
            let line = backup[slot_idx].line.as_mut().unwrap();
            line.data[offset] = value;
            line.dirty = true;
            line.last_used = use_count;
            line.used = true;
            line.clone()
        } else {
            let mut line = self.fill_line_from_backing(base_addr, use_count);
            line.data[offset] = value;
            line.dirty = true;
            BackupLine {
                base_addr: line.base_addr,
                data: line.data,
                dirty: line.dirty,
                last_used: line.last_used,
                used: false,
            }
        };

        refill.last_used = use_count;
        let evicted = self.install_primary_line(PrimaryLine {
            base_addr: refill.base_addr,
            data: refill.data.clone(),
            dirty: refill.dirty,
            last_used: use_count,
        });
        if let Some(line) = evicted {
            self.insert_primary_eviction_into_backup(line);
        }

        if charge_timing {
            let timing = if self.find_backup_slot(base_addr).is_some() {
                self.timing.store_hit
            } else {
                self.timing.store_miss
            };
            self.tick(timing);
        }
        self.access_complete();
    }

    #[cfg(test)]
    fn debug_enabled_lines(&self) -> usize {
        self.current_enabled_lines()
    }

    #[cfg(test)]
    fn debug_resize_countdown(&self) -> usize {
        *self.resize_countdown.borrow()
    }

    #[cfg(test)]
    fn debug_backup_line(&self, base_addr: usize) -> Option<(bool, bool)> {
        let backup = self.backup.borrow();
        backup.iter().find_map(|slot| {
            slot.line
                .as_ref()
                .and_then(|line| (line.base_addr == base_addr).then_some((slot.enabled, line.used)))
        })
    }
}

impl<'a> ContextSwitchListener for BackCacheMemory<'a> {
    fn on_context_switch(&self) {
        self.clear_backup_used_bits();
    }
}

impl<'a> InvalidationListener for BackCacheMemory<'a> {
    fn invalidate_line(&self, base_addr: usize) {
        self.invalidate_matching_primary_line(base_addr);
        self.invalidate_matching_backup_line(base_addr);
    }
}

impl<'a> MemoryDevice for BackCacheMemory<'a> {
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
    fn backup_hit_is_charged_like_l1_hit() {
        let clock = Rc::new(Clock::new());
        let mem = MainMemory::new(256)
            .with_clock(clock.clone())
            .with_timing(MainMemoryTiming {
                load: 100,
                store: 100,
            });
        let cache = BackCacheMemory::new(1, 1, 1, &mem)
            .with_clock(clock.clone())
            .with_timing(CacheTiming {
                load_hit: 4,
                load_miss: 20,
                store_hit: 4,
                store_miss: 20,
                write_back: 8,
                invalidation_send: 1,
                invalidation_apply: 1,
            })
            .with_policy(BackCachePolicy {
                min_enabled_lines: 1,
                max_enabled_lines: 1,
            });

        let _ = cache.load_u8(0);
        assert_eq!(clock.curr_tick(), 120);
        let _ = cache.load_u8(1);
        assert_eq!(clock.curr_tick(), 240);
        let _ = cache.load_u8(0);
        assert_eq!(clock.curr_tick(), 244);
    }

    #[test]
    fn primary_eviction_enters_backup_with_used_cleared() {
        let mem = MainMemory::new(256);
        let cache = BackCacheMemory::new(1, 1, 1, &mem).with_policy(BackCachePolicy {
            min_enabled_lines: 1,
            max_enabled_lines: 1,
        });

        let _ = cache.load_u8(0);
        let _ = cache.load_u8(1);

        assert_eq!(cache.debug_backup_line(0), Some((true, false)));
    }

    #[test]
    fn context_switch_clears_backup_used_bits() {
        let mem = MainMemory::new(256);
        let cache = BackCacheMemory::new(1, 1, 2, &mem).with_policy(BackCachePolicy {
            min_enabled_lines: 2,
            max_enabled_lines: 2,
        });

        let _ = cache.load_u8(0);
        let _ = cache.load_u8(1);
        let _ = cache.load_u8(2);
        let _ = cache.load_u8(0);
        assert_eq!(cache.debug_backup_line(0), Some((true, true)));

        cache.on_context_switch();
        assert_eq!(cache.debug_backup_line(0), Some((true, false)));
    }

    #[test]
    fn dynamic_resize_stays_within_policy_bounds() {
        let mem = MainMemory::new(256);
        let cache = BackCacheMemory::new(1, 1, 8, &mem).with_policy(BackCachePolicy {
            min_enabled_lines: 2,
            max_enabled_lines: 4,
        });

        assert!((2..=4).contains(&cache.debug_enabled_lines()));
        for addr in 0..64 {
            let _ = cache.load_u8(addr);
            assert!((2..=4).contains(&cache.debug_enabled_lines()));
            assert!((1..=4).contains(&cache.debug_resize_countdown()));
        }
    }

    #[test]
    fn lower_level_invalidation_removes_primary_and_backup_copies() {
        let mem = MainMemory::new(256);
        let cache = BackCacheMemory::new(1, 1, 1, &mem).with_policy(BackCachePolicy {
            min_enabled_lines: 1,
            max_enabled_lines: 1,
        });

        let _ = cache.load_u8(0);
        let _ = cache.load_u8(1);
        assert_eq!(cache.debug_backup_line(0), Some((true, false)));

        cache.invalidate_line(0);
        assert_eq!(cache.debug_backup_line(0), None);
    }
}
