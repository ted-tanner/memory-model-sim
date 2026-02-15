use crate::{
    device::{Clock, memory::MemoryDevice},
    program::Program,
};

// TODO: Read a file in main that defines the machine (e.g. how much RAM, which caches, register names, etc)
pub struct Machine {
    clock: Clock,
    memory: [Box<dyn MemoryDevice>],
}

impl Machine {
    pub fn run(prog: Program) -> i32 {
        unimplemented!();
    }
}
