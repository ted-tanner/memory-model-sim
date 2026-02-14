use crate::{device::{Clock, MainMemory}, program::Program};

pub struct Machine {
    clock: Clock,
    memory: MainMemory,
    // TODO: Use a trait for caches so we can have an array of arbitrary length
    //       for the caches. The L1 goes at index 0, L2 and index 1, etc.
}

impl Machine {
    fn run(prog: Program) -> i32 {
        unimplemented!();
    }
}
