//! Layer 2 — whole-payload parsing.
//!
//! A scrape response is a *sequence* of metric families. This module turns one
//! into the other, on top of the Layer 0 framing kernel and the existing
//! single-family parsers, in two shapes:
//!
//! * [`parse`] — a lazy, zero-copy [`Iterator`] over a complete in-memory
//!   buffer. Each family borrows the buffer; a malformed family yields one
//!   `Err` and parsing resumes at the next one.
//! * [`Decoder`] — an incremental, Sans-I/O engine: [`push`](Decoder::push)
//!   bytes as they arrive, [`finish`](Decoder::finish) at end of input, and pull
//!   families out. This is what a future async client wraps.
//!
//! The dialect is selected by [`Format`], so one entry point covers both the
//! text and protobuf exposition formats.

use bytes::{Buf, BytesMut};

use crate::Error;
use crate::borrowed::MetricFamily;
use crate::frame::{FrameStep, Scanner};
use crate::owned;
use crate::proto::scan::ProtoScanner;
use crate::text::TextFormat;
use crate::text::scan::TextScanner;

/// Which exposition format a payload is in. The [`TextFormat`] inside
/// [`Text`](Format::Text) only affects timestamp units (see [`TextFormat`]);
/// framing is identical across text dialects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// The Prometheus / OpenMetrics text exposition format.
    Text(TextFormat),
    /// The Prometheus protobuf exposition format (length-delimited
    /// `MetricFamily` messages).
    Protobuf,
}

fn scanner_for(format: Format) -> Scanner {
    match format {
        Format::Text(_) => Scanner::Text(TextScanner),
        Format::Protobuf => Scanner::Proto(ProtoScanner),
    }
}

/// Parse one already-framed family's bytes into a borrowed [`MetricFamily`].
fn parse_frame(format: Format, bytes: &[u8]) -> Result<MetricFamily<'_>, Error> {
    match format {
        Format::Text(tf) => {
            let text = std::str::from_utf8(bytes).map_err(Error::InvalidUtf8)?;
            crate::text::parse_family(text, tf)
        }
        Format::Protobuf => crate::proto::parse_family(bytes),
    }
}

/// Parse a complete in-memory payload into a lazy iterator of metric families.
///
/// Each yielded family borrows `buf` (zero-copy). Errors are per-family: a
/// malformed family produces one `Err` and the iterator continues at the next
/// frame, except for an unrecoverable framing error (e.g. a corrupt protobuf
/// length prefix), which ends the iterator after the `Err`.
///
/// To collect, owned: `parse(buf, fmt).map(|r| r.map(MetricFamily::into_owned))`.
pub fn parse(buf: &[u8], format: Format) -> Families<'_> {
    Families {
        buf,
        cursor: 0,
        scanner: scanner_for(format),
        format,
    }
}

/// The iterator returned by [`parse`]. See [`parse`] for semantics.
pub struct Families<'a> {
    buf: &'a [u8],
    cursor: usize,
    scanner: Scanner,
    format: Format,
}

impl<'a> Iterator for Families<'a> {
    type Item = Result<MetricFamily<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let rest: &'a [u8] = &self.buf[self.cursor..];
        match self.scanner.next_frame(rest, true) {
            FrameStep::Frame { consumed, bytes } => {
                self.cursor += consumed;
                Some(parse_frame(self.format, bytes))
            }
            FrameStep::Error { consumed, error } => {
                // consumed == 0 is unrecoverable: stop after this error.
                self.cursor = if consumed == 0 {
                    self.buf.len()
                } else {
                    self.cursor + consumed
                };
                Some(Err(error))
            }
            // `at_eof` is always true here, so NeedMore never occurs in practice.
            FrameStep::Done | FrameStep::NeedMore => {
                self.cursor = self.buf.len();
                None
            }
        }
    }
}

/// An incremental, Sans-I/O payload decoder.
///
/// Feed bytes with [`push`](Self::push) as they arrive and pull complete
/// families out with [`next_family`](Self::next_family) (zero-copy) or
/// [`next_owned`](Self::next_owned) (owned). Call [`finish`](Self::finish) once
/// the input has ended so the trailing family (text) or a truncation error
/// (protobuf) is surfaced.
///
/// Because a borrowed family aliases the internal buffer,
/// [`next_family`](Self::next_family) is a *lending* iterator: each family must
/// be dropped (or `into_owned`'d) before the next call. Use
/// [`next_owned`](Self::next_owned) / [`iter_owned`](Self::iter_owned) to
/// collect across calls.
pub struct Decoder {
    buf: BytesMut,
    scanner: Scanner,
    format: Format,
    eof: bool,
    /// Bytes of the just-returned frame, dropped lazily on the next call so a
    /// borrowed family stays valid until then.
    pending: usize,
    /// Set after an unrecoverable framing error: the stream is desynced and
    /// can't be resumed, so further pulls return `None` instead of looping.
    failed: bool,
}

impl Decoder {
    pub fn new(format: Format) -> Self {
        Decoder {
            buf: BytesMut::new(),
            scanner: scanner_for(format),
            format,
            eof: false,
            pending: 0,
            failed: false,
        }
    }

    /// Drop the previously-returned frame. Safe to call only when no borrowed
    /// family is outstanding (enforced by `&mut self` on the pull methods).
    fn drain_pending(&mut self) {
        if self.pending > 0 {
            self.buf.advance(self.pending);
            self.pending = 0;
        }
    }

    /// Append a chunk of input.
    pub fn push(&mut self, chunk: &[u8]) {
        self.drain_pending();
        self.buf.extend_from_slice(chunk);
    }

    /// Mark the input as complete: the trailing family is flushed and any
    /// truncated final frame becomes an error on the next pull.
    pub fn finish(&mut self) {
        self.eof = true;
    }

    /// Pull the next complete family, borrowing the internal buffer (zero-copy).
    ///
    /// Returns `None` when more input is needed (push more, then call again) or
    /// the stream is exhausted. The returned family borrows `self`, so it must
    /// be consumed before the next call.
    pub fn next_family(&mut self) -> Option<Result<MetricFamily<'_>, Error>> {
        if self.failed {
            return None;
        }
        self.drain_pending();
        match self.scanner.next_frame(&self.buf[..], self.eof) {
            FrameStep::Frame { consumed, bytes } => {
                // Defer the advance: `bytes` (hence the returned family) borrows
                // the buffer until the caller drops it before the next call.
                self.pending = consumed;
                Some(parse_frame(self.format, bytes))
            }
            FrameStep::Error { consumed, error } => {
                // Can't advance here — the Frame arm extends the buffer borrow
                // across the whole match — so defer it like a frame. `consumed
                // == 0` is unrecoverable: kill the stream so we don't re-emit.
                if consumed == 0 {
                    self.failed = true;
                } else {
                    self.pending = consumed;
                }
                Some(Err(error))
            }
            FrameStep::NeedMore | FrameStep::Done => None,
        }
    }

    /// Pull the next complete family as an owned value.
    ///
    /// Unlike [`next_family`](Self::next_family) the result owns its data, so
    /// families can be collected across calls (see [`iter_owned`](Self::iter_owned)).
    pub fn next_owned(&mut self) -> Option<Result<owned::MetricFamily, Error>> {
        if self.failed {
            return None;
        }
        self.drain_pending();
        match self.scanner.next_frame(&self.buf[..], self.eof) {
            FrameStep::Frame { consumed, bytes } => {
                // Owned result: the borrow ends at `into_owned`, so we can
                // advance eagerly here.
                let parsed = parse_frame(self.format, bytes).map(MetricFamily::into_owned);
                self.buf.advance(consumed);
                Some(parsed)
            }
            FrameStep::Error { consumed, error } => {
                if consumed == 0 {
                    self.failed = true;
                } else {
                    self.buf.advance(consumed);
                }
                Some(Err(error))
            }
            FrameStep::NeedMore | FrameStep::Done => None,
        }
    }

    /// Borrow the decoder as an [`Iterator`] of owned families, draining what is
    /// currently buffered (and the trailing family once [`finish`](Self::finish)
    /// has been called).
    pub fn iter_owned(&mut self) -> impl Iterator<Item = Result<owned::MetricFamily, Error>> + '_ {
        std::iter::from_fn(move || self.next_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owned::MetricType;

    const PAYLOAD: &str = "# TYPE a counter\na_total 1\n\
                           # TYPE b gauge\nb 2\n\
                           # TYPE c counter\nc_total 3\n";

    fn om() -> Format {
        Format::Text(TextFormat::OpenMetrics)
    }

    #[test]
    fn parse_yields_each_family() {
        let fams: Vec<_> = parse(PAYLOAD.as_bytes(), om())
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(fams.len(), 3);
        assert_eq!(fams[0].name, "a");
        assert_eq!(fams[0].r#type, MetricType::Counter);
        assert_eq!(fams[1].name, "b");
        assert_eq!(fams[1].r#type, MetricType::Gauge);
        assert_eq!(fams[2].name, "c");
    }

    #[test]
    fn malformed_family_is_isolated() {
        // The middle family has a non-numeric value.
        let payload = "# TYPE a gauge\na 1\n# TYPE b gauge\nb nope\n# TYPE c gauge\nc 3\n";
        let results: Vec<_> = parse(payload.as_bytes(), om()).collect();
        assert_eq!(results.len(), 3);
        assert!(results[0].is_ok());
        assert!(matches!(results[1], Err(Error::InvalidLine(_))));
        assert!(results[2].is_ok());
    }

    #[test]
    fn decoder_one_shot_matches_parse() {
        let mut dec = Decoder::new(om());
        dec.push(PAYLOAD.as_bytes());
        dec.finish();
        let names: Vec<_> = dec
            .iter_owned()
            .map(|r| r.unwrap().name)
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    /// The key streaming guarantee: feeding the payload split at *every* byte
    /// offset yields exactly the same families as the one-shot iterator.
    #[test]
    fn decoder_is_chunk_invariant() {
        let expected: Vec<String> = parse(PAYLOAD.as_bytes(), om())
            .map(|r| r.unwrap().name.into_owned())
            .collect();

        let bytes = PAYLOAD.as_bytes();
        for split in 0..=bytes.len() {
            let mut dec = Decoder::new(om());
            let mut got = Vec::new();
            // First chunk, drain, second chunk, finish, drain.
            dec.push(&bytes[..split]);
            while let Some(r) = dec.next_owned() {
                got.push(r.unwrap().name);
            }
            dec.push(&bytes[split..]);
            dec.finish();
            while let Some(r) = dec.next_owned() {
                got.push(r.unwrap().name);
            }
            assert_eq!(got, expected, "mismatch when split at byte {split}");
        }
    }

    #[test]
    fn next_family_is_zero_copy_lending() {
        let mut dec = Decoder::new(om());
        dec.push(PAYLOAD.as_bytes());
        dec.finish();
        let mut count = 0;
        while let Some(r) = dec.next_family() {
            // Borrow ends here, before the next call — the lending contract.
            assert!(r.is_ok());
            count += 1;
        }
        assert_eq!(count, 3);
    }

    // ---- protobuf ------------------------------------------------------

    use buffa::Message;
    use crate::proto::{
        Gauge, Metric as ProtoMetric, MetricFamily as ProtoFamily, MetricType as ProtoMetricType,
    };

    /// Encode a minimal single-gauge family to protobuf wire bytes.
    fn proto_family(name: &str, value: f64) -> Vec<u8> {
        ProtoFamily {
            name: Some(name.into()),
            r#type: Some(ProtoMetricType::GAUGE),
            metric: vec![ProtoMetric {
                gauge: Gauge {
                    value: Some(value),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            }],
            ..Default::default()
        }
        .encode_to_vec()
    }

    /// Prefix `msg` with its length varint — the length-delimited wire form.
    fn delimit(msg: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut len = msg.len() as u64;
        loop {
            let mut b = (len & 0x7f) as u8;
            len >>= 7;
            if len != 0 {
                b |= 0x80;
            }
            out.push(b);
            if len == 0 {
                break;
            }
        }
        out.extend_from_slice(msg);
        out
    }

    fn proto_stream() -> Vec<u8> {
        let mut buf = Vec::new();
        for (n, v) in [("a", 1.0), ("b", 2.0), ("c", 3.0)] {
            buf.extend(delimit(&proto_family(n, v)));
        }
        buf
    }

    #[test]
    fn proto_parse_yields_each_family() {
        let buf = proto_stream();
        let names: Vec<_> = parse(&buf, Format::Protobuf)
            .map(|r| r.unwrap().name.into_owned())
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn proto_decoder_is_chunk_invariant() {
        let buf = proto_stream();
        let expected = ["a", "b", "c"];
        for split in 0..=buf.len() {
            let mut dec = Decoder::new(Format::Protobuf);
            let mut got = Vec::new();
            dec.push(&buf[..split]);
            while let Some(r) = dec.next_owned() {
                got.push(r.unwrap().name);
            }
            dec.push(&buf[split..]);
            dec.finish();
            while let Some(r) = dec.next_owned() {
                got.push(r.unwrap().name);
            }
            assert_eq!(got, expected, "mismatch when split at byte {split}");
        }
    }

    #[test]
    fn proto_truncated_final_frame_errors() {
        let mut buf = delimit(&proto_family("a", 1.0));
        buf.pop(); // drop the last body byte
        let results: Vec<_> = parse(&buf, Format::Protobuf).collect();
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0], Err(Error::IncompleteFrame)));
    }

    #[test]
    fn proto_corrupt_body_resyncs() {
        // A frame with a valid length prefix but a garbage body (wire type 7),
        // followed by a good family: the bad one errors, the good one parses.
        let mut buf = delimit(&[0xff, 0xff, 0xff]);
        buf.extend(delimit(&proto_family("ok", 1.0)));
        let results: Vec<_> = parse(&buf, Format::Protobuf).collect();
        assert_eq!(results.len(), 2);
        assert!(results[0].is_err());
        assert_eq!(results[1].as_ref().unwrap().name, "ok");
    }
}
