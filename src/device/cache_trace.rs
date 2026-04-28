use std::cell::RefCell;
use std::rc::Rc;

use super::secdcp_memory::SecurityClass;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheAccessKind {
    Load,
    Store,
    Invalidate,
    SliceReassign,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheAccessSource {
    L1D,
    BackCache,
    Lower,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheAccessEvent {
    pub architecture: &'static str,
    pub requester: SecurityClass,
    pub requester_pid: u32,
    pub requester_domain: u32,
    pub kind: CacheAccessKind,
    pub addr: usize,
    pub set: Option<usize>,
    pub hit: bool,
    pub source: CacheAccessSource,
    pub evicted_owner: Option<SecurityClass>,
    pub evicted_pid: Option<u32>,
    pub evicted_domain: Option<u32>,
    pub evicted_addr: Option<usize>,
    pub slice: Option<usize>,
    pub writebacks: u64,
}

#[derive(Default)]
pub struct CacheTrace {
    events: RefCell<Vec<CacheAccessEvent>>,
}

pub type SharedCacheTrace = Rc<CacheTrace>;

impl CacheTrace {
    pub fn new_shared() -> SharedCacheTrace {
        Rc::new(Self::default())
    }

    pub fn record(&self, event: CacheAccessEvent) {
        self.events.borrow_mut().push(event);
    }

    pub fn drain(&self) -> Vec<CacheAccessEvent> {
        self.events.borrow_mut().drain(..).collect()
    }
}
