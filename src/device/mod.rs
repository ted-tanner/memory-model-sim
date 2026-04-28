#![allow(unused)]

mod clock;
pub use clock::Clock;

pub mod cache_trace;

pub trait ContextSwitchListener {
    fn on_context_switch(&self);
}

pub mod backcache_memory;
pub mod memory;
pub mod newcache_memory;
pub mod secdcp_memory;
pub mod smtcache_memory;
