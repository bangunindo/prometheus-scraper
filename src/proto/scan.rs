//! Layer 0 protobuf scanner.
//!
//! Prometheus protobuf exposition is the *length-delimited* stream: each
//! `MetricFamily` message is preceded by a base-128 varint giving its byte
//! length. `buffa` has no streaming view API and its `decode_view` cannot tell
//! "need more bytes" from "corrupt", so we do the framing ourselves — read the
//! varint, wait until the whole message is buffered, then hand exactly that
//! slice to [`proto::parse_family`](crate::proto::parse_family).
//!
//! Only the *framing* fails here (a malformed length prefix or a truncated final
//! frame). A corrupt message *body* still has a known length, so we frame it
//! normally and let the parse step report the decode error — the stream resyncs
//! at the next frame.

use crate::ParseError;
use crate::frame::{FrameScanner, FrameStep};

pub(crate) struct ProtoScanner;

enum Varint {
    Ok { value: u64, len: usize },
    /// Ran out of bytes mid-varint.
    Incomplete,
    /// More than 10 bytes without a terminator — not a valid u64 varint.
    Invalid,
}

/// Decode a base-128 varint from the front of `buf`, reporting how many bytes it
/// occupied. Mirrors `buffa`'s private `decode_varint_slice`.
fn read_varint(buf: &[u8]) -> Varint {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    for (i, &b) in buf.iter().enumerate() {
        if i == 10 {
            return Varint::Invalid;
        }
        value |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Varint::Ok { value, len: i + 1 };
        }
        shift += 7;
    }
    Varint::Incomplete
}

impl FrameScanner for ProtoScanner {
    fn next_frame<'a>(&self, buf: &'a [u8], at_eof: bool) -> FrameStep<'a> {
        if buf.is_empty() {
            return if at_eof {
                FrameStep::Done
            } else {
                FrameStep::NeedMore
            };
        }
        match read_varint(buf) {
            // A partial length prefix: more may arrive, otherwise it's truncated
            // trailing data we can't frame.
            Varint::Incomplete => {
                if at_eof {
                    FrameStep::Error {
                        consumed: buf.len(),
                        error: ParseError::IncompleteFrame,
                    }
                } else {
                    FrameStep::NeedMore
                }
            }
            // No length means no way to resync — stop.
            Varint::Invalid => FrameStep::Error {
                consumed: 0,
                error: ParseError::IncompleteFrame,
            },
            Varint::Ok { value, len } => {
                // Guard the `value as usize` cast (32-bit) and the offset add.
                let Some(total) = usize::try_from(value).ok().and_then(|m| len.checked_add(m)) else {
                    return FrameStep::Error {
                        consumed: 0,
                        error: ParseError::IncompleteFrame,
                    };
                };
                if buf.len() < total {
                    if at_eof {
                        FrameStep::Error {
                            consumed: buf.len(),
                            error: ParseError::IncompleteFrame,
                        }
                    } else {
                        FrameStep::NeedMore
                    }
                } else {
                    FrameStep::Frame {
                        consumed: total,
                        bytes: &buf[len..total],
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(buf: &[u8], at_eof: bool) -> FrameStep<'_> {
        ProtoScanner.next_frame(buf, at_eof)
    }

    /// Prefix `msg` with its length as a varint — the length-delimited wire form.
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

    #[test]
    fn frames_a_complete_message() {
        let buf = delimit(b"hello");
        let FrameStep::Frame { consumed, bytes } = frame(&buf, false) else {
            panic!("expected a frame");
        };
        assert_eq!(bytes, b"hello");
        assert_eq!(consumed, buf.len());
    }

    #[test]
    fn waits_for_the_rest_of_the_body() {
        let full = delimit(b"hello");
        // Only the length byte + 2 of 5 body bytes have arrived.
        assert!(matches!(frame(&full[..3], false), FrameStep::NeedMore));
        // At EOF the same truncated frame is an error.
        assert!(matches!(frame(&full[..3], true), FrameStep::Error { .. }));
    }

    #[test]
    fn empty_buffer() {
        assert!(matches!(frame(&[], false), FrameStep::NeedMore));
        assert!(matches!(frame(&[], true), FrameStep::Done));
    }

    #[test]
    fn back_to_back_frames_via_consumed() {
        let mut buf = delimit(b"aa");
        buf.extend(delimit(b"bbbb"));
        let FrameStep::Frame { consumed, bytes } = frame(&buf, true) else {
            panic!("first frame");
        };
        assert_eq!(bytes, b"aa");
        let FrameStep::Frame { bytes, .. } = frame(&buf[consumed..], true) else {
            panic!("second frame");
        };
        assert_eq!(bytes, b"bbbb");
    }

    #[test]
    fn oversized_varint_is_unrecoverable() {
        // Eleven continuation bytes: never terminates within a u64.
        let buf = [0x80u8; 11];
        assert!(matches!(
            frame(&buf, false),
            FrameStep::Error { consumed: 0, .. }
        ));
    }
}
