use std::cell::RefCell;

pub struct Clock {
    tick: RefCell<u64>,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            tick: RefCell::new(0),
        }
    }

    pub fn tick(&self) {
        *self.tick.borrow_mut() += 1;
    }

    pub fn advance(&self, n: u64) {
        *self.tick.borrow_mut() += n;
    }

    pub fn curr_tick(&self) -> u64 {
        *self.tick.borrow()
    }
}

#[cfg(test)]
mod tests {
    use super::Clock;

    #[test]
    fn test_tick() {
        let clock = Clock::new();
        assert_eq!(clock.curr_tick(), 0);

        clock.tick();
        assert_eq!(clock.curr_tick(), 1);

        clock.tick();
        clock.tick();
        clock.tick();
        assert_eq!(clock.curr_tick(), 4);
    }

    #[test]
    fn test_advance() {
        let clock = Clock::new();
        assert_eq!(clock.curr_tick(), 0);

        clock.advance(7);
        assert_eq!(clock.curr_tick(), 7);

        clock.advance(3);
        assert_eq!(clock.curr_tick(), 10);
    }
}

