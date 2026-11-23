//! Tick-to-bar synthesis (pillar LIV goal 528's software form): a
//! fixed-epoch OHLCV aggregator over the monotonic tick stream.
//!
//! Semantics: every tick lands in the epoch `ts - ts % bucket_ns`;
//! a tick in a LATER epoch completes the current bar (emitted as the
//! return value) and opens the next. Epochs with no ticks emit no
//! bar — a bar exists where ticks exist (the store replays facts,
//! not interpolations). The reference price is `bid_px` (our Tick is
//! a quote tick; omni-md's strategy trades the bid side — the
//! convention is consistent with the pipeline).
//!
//! Zero allocation, zero unsafe, zero panics.

use crate::Tick;
use core::mem;

/// One completed OHLCV bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bar {
    /// The epoch start (a multiple of `bucket_ns`).
    pub epoch_start_ns: u64,
    /// The first reference price in the epoch.
    pub open: u64,
    /// The highest reference price in the epoch.
    pub high: u64,
    /// The lowest reference price in the epoch.
    pub low: u64,
    /// The last reference price in the epoch.
    pub close: u64,
    /// The number of ticks in the epoch.
    pub volume: u64,
}

impl Bar {
    /// The bounds invariant of goal 528: every observed price sits
    /// inside [low, high], and open/close are observed prices.
    #[must_use]
    pub fn bounds_hold(&self) -> bool {
        self.low <= self.open
            && self.open <= self.high
            && self.low <= self.close
            && self.close <= self.high
    }
}

/// A partial bar under synthesis: the epoch, its extremes, and its
/// tick count.
#[derive(Default)]
struct Partial {
    epoch_start_ns: u64,
    open: u64,
    high: u64,
    low: u64,
    close: u64,
    volume: u64,
}

impl Partial {
    fn open(epoch_start_ns: u64, price: u64) -> Self {
        Self {
            epoch_start_ns,
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 1,
        }
    }

    fn absorb(&mut self, price: u64) {
        self.high = self.high.max(price);
        self.low = self.low.min(price);
        self.close = price;
        self.volume += 1;
    }

    fn finish(self) -> Bar {
        Bar {
            epoch_start_ns: self.epoch_start_ns,
            open: self.open,
            high: self.high,
            low: self.low,
            close: self.close,
            volume: self.volume,
        }
    }
}

/// The tick-to-bar synthesizer: feed ticks in (monotonic order —
/// `push_monotonic`'s output), completed bars come out at epoch
/// crossings.
pub struct BarAggregator {
    bucket_ns: u64,
    current: Option<Partial>,
}

impl BarAggregator {
    /// Creates an aggregator with `bucket_ns`-wide epochs. A zero
    /// bucket is rejected (every ts would start a new bar — the
    /// degenerate no-synthesis case).
    ///
    /// # Errors
    ///
    /// `None` when `bucket_ns` is 0.
    #[must_use]
    pub fn new(bucket_ns: u64) -> Option<Self> {
        if bucket_ns == 0 {
            return None;
        }
        Some(Self {
            bucket_ns,
            current: None,
        })
    }

    /// Feeds one tick; returns the completed bar when the tick
    /// crosses into a new epoch. Empty epochs between the last bar
    /// and the tick emit nothing (a bar exists where ticks exist).
    /// The tick's `bid_px` is the reference price.
    #[must_use]
    pub fn push(&mut self, tick: &Tick) -> Option<Bar> {
        let epoch = tick.ts_ns - tick.ts_ns % self.bucket_ns;
        match mem::take(&mut self.current) {
            None => {
                self.current = Some(Partial::open(epoch, tick.bid_px));
                None
            }
            Some(mut cur) if cur.epoch_start_ns == epoch => {
                cur.absorb(tick.bid_px);
                self.current = Some(cur);
                None
            }
            Some(cur) => {
                self.current = Some(Partial::open(epoch, tick.bid_px));
                Some(cur.finish())
            }
        }
    }

    /// Flushes an in-progress bar as complete (end-of-stream). The
    /// synthesizer is empty afterwards.
    #[must_use]
    pub fn flush(&mut self) -> Option<Bar> {
        mem::take(&mut self.current).map(Partial::finish)
    }
}
