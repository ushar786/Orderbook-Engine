#[derive(Debug, Clone, Default)]
pub struct InMemorySequencer {
    current: u64,
}

impl InMemorySequencer {
    pub fn next_sequence(&mut self) -> u64 {
        self.current += 1;
        self.current
    }

    pub fn current(&self) -> u64 {
        self.current
    }

    pub fn advance_to(&mut self, sequence: u64) {
        self.current = self.current.max(sequence);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_monotonic_sequences() {
        let mut sequencer = InMemorySequencer::default();

        assert_eq!(sequencer.current(), 0);
        assert_eq!(sequencer.next_sequence(), 1);
        assert_eq!(sequencer.next_sequence(), 2);
    }

    #[test]
    fn advances_without_rewinding() {
        let mut sequencer = InMemorySequencer::default();

        sequencer.advance_to(10);
        sequencer.advance_to(4);

        assert_eq!(sequencer.current(), 10);
        assert_eq!(sequencer.next_sequence(), 11);
    }
}
