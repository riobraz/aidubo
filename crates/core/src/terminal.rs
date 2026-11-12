//! Terminal output batching.
//!
//! The engine's socket pump and the view both want a bounded number of
//! updates: this policy is pure so the limits can be tested without IO.

use std::time::{Duration, Instant};

/// Output is flushed at this interval at the latest.
pub const TERMINAL_BATCH_INTERVAL: Duration = Duration::from_millis(16);
/// Output is flushed immediately once a batch reaches this size.
pub const TERMINAL_OUTPUT_LIMIT: usize = 64 * 1024;

/// Coalesces terminal output into chunks bounded by time and size.
#[derive(Debug, Default)]
pub struct TerminalBatch {
    bytes: Vec<u8>,
    started: Option<Instant>,
}

impl TerminalBatch {
    /// Append bytes and return any full 64 KiB chunks that are ready now.
    pub fn push(&mut self, bytes: &[u8], now: Instant) -> Vec<Vec<u8>> {
        if self.started.is_none() {
            self.started = Some(now);
        }
        self.bytes.extend_from_slice(bytes);
        let mut chunks = Vec::new();
        while self.bytes.len() >= TERMINAL_OUTPUT_LIMIT {
            let tail = self.bytes.split_off(TERMINAL_OUTPUT_LIMIT);
            chunks.push(std::mem::replace(&mut self.bytes, tail));
            // The next chunk gets its own full interval.
            self.started = Some(now);
        }
        if self.bytes.is_empty() {
            self.started = None;
        }
        chunks
    }

    /// When the buffered remainder must be flushed.
    pub fn deadline(&self) -> Option<Instant> {
        self.started
            .map(|started| started + TERMINAL_BATCH_INTERVAL)
    }

    pub fn take(&mut self) -> Vec<u8> {
        self.started = None;
        std::mem::take(&mut self.bytes)
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_batch_bounds_events() {
        let start = Instant::now();
        let mut batch = TerminalBatch::default();

        assert!(batch.push(b"hello", start).is_empty());
        assert_eq!(batch.deadline(), Some(start + TERMINAL_BATCH_INTERVAL));
        assert_eq!(batch.take(), b"hello");
        assert_eq!(batch.deadline(), None);

        let large = vec![b'x'; TERMINAL_OUTPUT_LIMIT + 10];
        let chunks = batch.push(&large, start);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), TERMINAL_OUTPUT_LIMIT);
        assert_eq!(batch.take().len(), 10);

        let burst = vec![b'y'; TERMINAL_OUTPUT_LIMIT * 3 + 7];
        let chunks = batch.push(&burst, start);
        assert_eq!(chunks.len(), 3);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.len() <= TERMINAL_OUTPUT_LIMIT)
        );
        assert_eq!(batch.take().len(), 7);
    }
}
