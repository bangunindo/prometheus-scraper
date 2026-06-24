//! Layer 0 text scanner.
//!
//! Splits a text-exposition payload into one frame per metric family, where a
//! frame is a run of whole lines handed verbatim to
//! [`text::parse_family`](super::parse_family). The scanner never parses values
//! — it only finds boundaries, working a line at a time and only on *complete*
//! lines (a `\n`, or end-of-input when `at_eof`).
//!
//! Boundary rules, in priority order, tracking the current family's base name:
//!
//! * A **descriptor** line (`# TYPE` / `# HELP` / `# UNIT`) starts a new family
//!   if it appears *after* a sample line (metadata always precedes samples
//!   within a family) or names a *different* family. This is the workhorse:
//!   standard exposition emits a descriptor block per family.
//! * A **sample** line whose name doesn't belong to the current family (it is
//!   neither the base name nor base + a known suffix) starts a new family.
//! * `# EOF` ends the stream; the bytes before it are the final frame.
//! * Blank lines and plain (non-descriptor) comment lines stay with the current
//!   family.
//!
//! **Limitation:** with no descriptor lines at all, families can only be
//! separated by sample base-name change, which is inherently best-effort. Real
//! Prometheus / OpenMetrics output always carries descriptors.

use std::borrow::Cow;

use crate::frame::{FrameScanner, FrameStep};

pub(crate) struct TextScanner;

/// Suffixes that bind a sample line to its family's base name. Mirrors the
/// suffixes recognized by `super::role`, plus `_info` (an `info` family `foo`
/// carries a `foo_info` sample, which `role` itself never needs to recognize).
const FAMILY_SUFFIXES: &[&str] = &[
    "_total", "_created", "_bucket", "_count", "_sum", "_gcount", "_gsum", "_info",
];

/// True when `name` is a sample of the family whose base name is `base`.
fn family_member(name: &str, base: &str) -> bool {
    name == base
        || name
            .strip_prefix(base)
            .is_some_and(|rest| FAMILY_SUFFIXES.contains(&rest))
}

/// Best-effort base name of a standalone sample (used only when no descriptor
/// has named the family yet): strip a recognized trailing suffix, if any.
fn base_of(name: &str) -> &str {
    for suffix in FAMILY_SUFFIXES {
        if let Some(base) = name.strip_suffix(suffix) {
            return base;
        }
    }
    name
}

enum Line<'a> {
    Blank,
    /// A `#` comment that is not a descriptor or `# EOF`.
    Comment,
    Eof,
    /// Looks like a sample but couldn't be classified (bad UTF-8, no name): keep
    /// it with the current family so the parse step surfaces the real error.
    Opaque,
    Descriptor(Cow<'a, str>),
    Sample(Cow<'a, str>),
}

fn classify(line: &[u8]) -> Line<'_> {
    let Ok(s) = std::str::from_utf8(line) else {
        return Line::Opaque;
    };
    let trimmed = s.trim_start();
    if trimmed.is_empty() {
        return Line::Blank;
    }
    if let Some(rest) = trimmed.strip_prefix('#') {
        if rest.trim() == "EOF" {
            return Line::Eof;
        }
        return match super::descriptor_name(s) {
            Some(name) => Line::Descriptor(name),
            None => Line::Comment,
        };
    }
    match super::sample_name(s) {
        Some(name) => Line::Sample(name),
        None => Line::Opaque,
    }
}

impl FrameScanner for TextScanner {
    fn next_frame<'a>(&self, buf: &'a [u8], at_eof: bool) -> FrameStep<'a> {
        if buf.is_empty() {
            return if at_eof {
                FrameStep::Done
            } else {
                FrameStep::NeedMore
            };
        }

        let mut name: Option<Cow<'a, str>> = None;
        let mut seen_sample = false;
        let mut pos = 0; // byte offset of the start of the current line

        while pos < buf.len() {
            let (line_bytes, next_pos, has_nl) = match buf[pos..].iter().position(|&b| b == b'\n') {
                Some(off) => (&buf[pos..pos + off], pos + off + 1, true),
                None => (&buf[pos..], buf.len(), false),
            };
            // A trailing partial line might continue in the next chunk — but if
            // we already have a family buffered, hold it (NeedMore) rather than
            // risk splitting on an unfinished line.
            if !has_nl && !at_eof {
                return FrameStep::NeedMore;
            }
            let line = strip_cr(line_bytes);

            match classify(line) {
                Line::Blank | Line::Comment => {}
                Line::Opaque => seen_sample = true,
                Line::Eof => {
                    return if pos == 0 {
                        FrameStep::Done
                    } else {
                        FrameStep::Frame {
                            consumed: pos,
                            bytes: &buf[..pos],
                        }
                    };
                }
                Line::Descriptor(n) => {
                    if seen_sample {
                        // metadata after samples => the next family begins here
                        return FrameStep::Frame {
                            consumed: pos,
                            bytes: &buf[..pos],
                        };
                    }
                    match &name {
                        None => name = Some(n),
                        Some(cur) if cur.as_ref() == n.as_ref() => {}
                        Some(_) => {
                            return FrameStep::Frame {
                                consumed: pos,
                                bytes: &buf[..pos],
                            };
                        }
                    }
                }
                Line::Sample(sn) => match &name {
                    None => {
                        name = Some(Cow::Owned(base_of(sn.as_ref()).to_owned()));
                        seen_sample = true;
                    }
                    Some(cur) if family_member(sn.as_ref(), cur.as_ref()) => seen_sample = true,
                    Some(_) => {
                        return FrameStep::Frame {
                            consumed: pos,
                            bytes: &buf[..pos],
                        };
                    }
                },
            }

            pos = next_pos;
        }

        // Reached the end of the buffer with no boundary.
        if at_eof {
            if name.is_some() || seen_sample {
                FrameStep::Frame {
                    consumed: buf.len(),
                    bytes: buf,
                }
            } else {
                FrameStep::Done // only blanks / comments
            }
        } else {
            // The family may continue — we can only emit once the next family's
            // first line (or EOF) arrives.
            FrameStep::NeedMore
        }
    }
}

fn strip_cr(line: &[u8]) -> &[u8] {
    match line.last() {
        Some(b'\r') => &line[..line.len() - 1],
        _ => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(buf: &str, at_eof: bool) -> FrameStep<'_> {
        TextScanner.next_frame(buf.as_bytes(), at_eof)
    }

    fn as_str(step: FrameStep<'_>) -> &str {
        match step {
            FrameStep::Frame { bytes, .. } => std::str::from_utf8(bytes).unwrap(),
            FrameStep::NeedMore => "<needmore>",
            FrameStep::Done => "<done>",
            FrameStep::Error { .. } => "<error>",
        }
    }

    #[test]
    fn splits_two_families_on_descriptor() {
        let buf = "# TYPE a counter\na_total 1\n# TYPE b gauge\nb 2\n";
        let FrameStep::Frame { consumed, bytes } = frame(buf, true) else {
            panic!("expected first family");
        };
        assert_eq!(std::str::from_utf8(bytes).unwrap(), "# TYPE a counter\na_total 1\n");
        // The remainder is the second family, flushed at EOF.
        assert_eq!(as_str(frame(&buf[consumed..], true)), "# TYPE b gauge\nb 2\n");
    }

    #[test]
    fn descriptor_after_sample_starts_new_family() {
        // Same shape, no name change relied upon — the boundary is metadata
        // appearing after a sample line.
        let buf = "# HELP a x\na 1\n# HELP b y\nb 2\n";
        assert_eq!(as_str(frame(buf, true)), "# HELP a x\na 1\n");
    }

    #[test]
    fn histogram_suffixes_stay_in_one_family() {
        let buf = "# TYPE rt histogram\nrt_bucket{le=\"1\"} 1\nrt_sum 2\nrt_count 1\n";
        // Whole block is one family (flushed at EOF).
        assert_eq!(as_str(frame(buf, true)), buf);
    }

    #[test]
    fn info_suffix_belongs_to_family() {
        // `# TYPE build info` names family `build`; `build_info` is its sample.
        let buf = "# TYPE build info\nbuild_info{v=\"1\"} 1\n";
        assert_eq!(as_str(frame(buf, true)), buf);
    }

    #[test]
    fn eof_marks_end() {
        let buf = "# TYPE a counter\na_total 1\n# EOF\n";
        let FrameStep::Frame { consumed, bytes } = frame(buf, true) else {
            panic!("expected family before EOF");
        };
        assert_eq!(std::str::from_utf8(bytes).unwrap(), "# TYPE a counter\na_total 1\n");
        assert!(matches!(frame(&buf[consumed..], true), FrameStep::Done));
    }

    #[test]
    fn comment_and_blank_lines_dont_split() {
        let buf = "# TYPE a counter\n# a stray comment\n\na_total 1\n";
        assert_eq!(as_str(frame(buf, true)), buf);
    }

    #[test]
    fn streaming_holds_until_next_family_seen() {
        // One full family but no following family or EOF yet -> NeedMore.
        let buf = "# TYPE a counter\na_total 1\n";
        assert!(matches!(frame(buf, false), FrameStep::NeedMore));
        // Once the next family's first line arrives, the first one is emitted.
        let buf2 = "# TYPE a counter\na_total 1\n# TYPE b gauge\n";
        assert_eq!(as_str(frame(buf2, false)), "# TYPE a counter\na_total 1\n");
    }

    #[test]
    fn partial_last_line_needs_more() {
        // The second family's descriptor line is cut mid-line.
        let buf = "# TYPE a counter\na_total 1\n# TYPE b ga";
        assert!(matches!(frame(buf, false), FrameStep::NeedMore));
    }

    #[test]
    fn descriptorless_splits_on_base_name_change() {
        let buf = "foo 1\nbar 2\n";
        assert_eq!(as_str(frame(buf, true)), "foo 1\n");
    }
}
