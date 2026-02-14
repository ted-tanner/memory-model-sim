use std::cell::{Ref, RefCell};

pub struct MainMemory {
    buf: RefCell<Box<[u8]>>,
    size: usize,
}

impl MainMemory {
    pub fn new(size: usize) -> Self {
        Self {
            buf: RefCell::new(vec![0; size].into_boxed_slice()),
            size,
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn get(&self, pos: usize) -> u8 {
        self.buf.borrow()[pos]
    }

    pub fn set(&self, pos: usize, n: u8) {
        self.buf.borrow_mut()[pos] = n;
    }

    pub fn get_range(&self, start: usize, end: usize) -> Ref<'_, [u8]> {
        Ref::map(self.buf.borrow(), |b| &b[start..end])
    }

    pub fn set_range(&self, start: usize, buf: &[u8]) {
        let end = start + buf.len();
        self.buf.borrow_mut()[start..end].copy_from_slice(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::MainMemory;

    #[test]
    fn test_get_set() {
        let mem = MainMemory::new(32);

        mem.set(20, 42);
        assert_eq!(mem.get(20), 42);

        mem.set(3, 3);
        mem.set(17, 6);
        mem.set(30, 9);
        assert_eq!(mem.get(20), 42);
        assert_eq!(mem.get(3), 3);
        assert_eq!(mem.get(17), 6);
        assert_eq!(mem.get(30), 9);
    }

    #[test]
    fn test_get_set_range() {
        let mem = MainMemory::new(32);

        mem.set_range(20, &[4, 6, 8, 10]);
        assert_eq!(*mem.get_range(20, 24), [4, 6, 8, 10]);

        mem.set_range(17, &[50, 60, 70, 80]);
        assert_eq!(*mem.get_range(17, 24), [50, 60, 70, 80, 6, 8, 10]);

        assert_eq!(mem.get(20), 80);
        assert_eq!(mem.get(17), 50);
        assert_eq!(mem.get(23), 10);
    }
}
