//! Layer 0 — the framing kernel.
//!
//! A [`FrameScanner`] turns a byte buffer into a sequence of *frames*, each the
//! raw bytes of exactly one metric family, ready to hand to the existing
//! single-family parsers. Scanners are **stateless across calls**: every
//! [`next_frame`](FrameScanner::next_frame) re-scans the given slice for one
//! boundary and reports how far to advance. That keeps all lifetimes local,
//! which is what lets the incremental [`Decoder`](crate::payload::Decoder) reuse
//! the same code over a buffer that is compacted between calls.
//!
//! `at_eof` distinguishes the two callers: the in-memory
//! [`parse`](crate::payload::parse) iterator always passes `true` (the whole
//! payload is present), while the `Decoder` passes `false` until its input ends.
//! `NeedMore` is the streaming "feed me more bytes" signal — it is control flow,
//! never an [`Error`].

use crate::Error;
use crate::proto::scan::ProtoScanner;
use crate::text::scan::TextScanner;

/// The outcome of asking a [`FrameScanner`] for the next family-sized frame.
pub(crate) enum FrameStep<'a> {
    /// One complete family. `bytes` is handed to the parse step; the caller
    /// advances its input cursor by `consumed` (equal to `bytes.len()` for text,
    /// `varint_len + msg_len` for protobuf).
    Frame { consumed: usize, bytes: &'a [u8] },
    /// The next boundary isn't determinable yet — feed more bytes. Scanners only
    /// return this when `at_eof` is `false`.
    NeedMore,
    /// No more frames remain in this input.
    Done,
    /// The framing itself failed. `consumed > 0` means "skip these bytes and
    /// resync at the next frame"; `consumed == 0` means unrecoverable, so the
    /// caller emits the error and then stops.
    Error { consumed: usize, error: Error },
}

pub(crate) trait FrameScanner {
    fn next_frame<'a>(&self, buf: &'a [u8], at_eof: bool) -> FrameStep<'a>;
}

/// Dispatch over the concrete scanners so the public `Families`/`Decoder` stay
/// non-generic.
pub(crate) enum Scanner {
    Text(TextScanner),
    Proto(ProtoScanner),
}

impl Scanner {
    pub(crate) fn next_frame<'a>(&self, buf: &'a [u8], at_eof: bool) -> FrameStep<'a> {
        match self {
            Scanner::Text(s) => s.next_frame(buf, at_eof),
            Scanner::Proto(s) => s.next_frame(buf, at_eof),
        }
    }
}
