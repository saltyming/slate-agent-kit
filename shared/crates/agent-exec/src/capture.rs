//! Output capture under a policy: how much of stdout and stderr a record
//! keeps, which end, and whether the limit counts bytes or characters.
//!
//! Entry points: [`CapturePolicy`] (with the [`CapturePolicy::aside`] and
//! [`CapturePolicy::dispatch`] presets) chosen by the server; `read_capped`,
//! which `run` spawns once per stream so both are read concurrently; and
//! `clip`, which applies the failure cap to stdout already kept. A stream is
//! read to its end: bytes past the cap are drained, so the child never blocks
//! on a full pipe, and discarded; the loss is reported as `truncated`, never
//! written into the text. Character slicing never splits a code point.

use tokio::io::{AsyncRead, AsyncReadExt};

/// Which end of a stream a cap keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// The first `limit` units.
    Head,
    /// The last `limit` units.
    Tail,
}

/// What a cap's limit counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Bytes of the raw stream; the kept bytes are then decoded lossily.
    Bytes,
    /// Characters of the decoded stream.
    Chars,
}

/// One cap: keep `limit` units from the `keep` end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cap {
    /// How many units are kept.
    pub limit: usize,
    /// Which end is kept.
    pub keep: Keep,
    /// What `limit` counts.
    pub unit: Unit,
}

/// How much of a run's output a record keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapturePolicy {
    /// stdout of every run.
    pub stdout_cap: Cap,
    /// stderr of every run.
    pub stderr_cap: Cap,
    /// stdout of a run that exited non-zero, applied to what `stdout_cap`
    /// kept.
    pub failure_stdout_cap: Cap,
}

impl CapturePolicy {
    /// aside's policy: stdout the first 50 KB by character, stderr the last
    /// 2 KB by character, and on a non-zero exit stdout the first 2 KB by
    /// character (the CLIs that print their error on stdout print it first).
    pub fn aside() -> Self {
        CapturePolicy {
            stdout_cap: Cap {
                limit: 50 * 1024,
                keep: Keep::Head,
                unit: Unit::Chars,
            },
            stderr_cap: Cap {
                limit: 2 * 1024,
                keep: Keep::Tail,
                unit: Unit::Chars,
            },
            failure_stdout_cap: Cap {
                limit: 2 * 1024,
                keep: Keep::Head,
                unit: Unit::Chars,
            },
        }
    }

    /// dispatch's policy: stdout the first 200 KB by byte (dispatched runs are
    /// agent transcripts and can be verbose), stderr the first 16 KB by byte,
    /// and on a non-zero exit stdout the same as on success.
    pub fn dispatch() -> Self {
        let stdout = Cap {
            limit: 200 * 1024,
            keep: Keep::Head,
            unit: Unit::Bytes,
        };
        CapturePolicy {
            stdout_cap: stdout,
            stderr_cap: Cap {
                limit: 16 * 1024,
                keep: Keep::Head,
                unit: Unit::Bytes,
            },
            failure_stdout_cap: stdout,
        }
    }
}

/// What one stream left under its cap.
#[derive(Debug, Default)]
pub(crate) struct Captured {
    pub text: String,
    /// Bytes the stream produced in all.
    pub total: u64,
    /// Whether anything the stream produced is missing from `text`.
    pub truncated: bool,
}

/// Read `reader` to its end under `cap`. A read error ends the stream with
/// what was kept so far.
pub(crate) async fn read_capped<R: AsyncRead + Unpin>(reader: Option<R>, cap: Cap) -> Captured {
    let Some(mut r) = reader else {
        return Captured::default();
    };
    // A character is at most four UTF-8 bytes, so `4 * limit` bytes hold at
    // least `limit` whole characters; a tail keeps three more so that the
    // partial character its cut may start with can be dropped.
    let budget = match (cap.unit, cap.keep) {
        (Unit::Bytes, _) => cap.limit,
        (Unit::Chars, Keep::Head) => cap.limit.saturating_mul(4),
        (Unit::Chars, Keep::Tail) => cap.limit.saturating_mul(4).saturating_add(3),
    };
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut total: u64 = 0;
    loop {
        let n = match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        total += n as u64;
        match cap.keep {
            Keep::Head => {
                if buf.len() < budget {
                    let take = (budget - buf.len()).min(n);
                    buf.extend_from_slice(&chunk[..take]);
                }
                // else: drain and discard the overflow.
            }
            Keep::Tail => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > budget.saturating_mul(2) {
                    buf.drain(..buf.len() - budget);
                }
            }
        }
    }
    if cap.keep == Keep::Tail && buf.len() > budget {
        buf.drain(..buf.len() - budget);
    }
    let bytes_dropped = total > buf.len() as u64;
    if cap.unit == Unit::Chars && cap.keep == Keep::Tail && bytes_dropped {
        // The cut may have landed inside a character: skip its continuation
        // bytes rather than decode them as replacement characters.
        let skip = buf
            .iter()
            .take(3)
            .take_while(|b| (**b & 0xC0) == 0x80)
            .count();
        buf.drain(..skip);
    }
    let decoded = String::from_utf8_lossy(&buf).into_owned();
    let (text, chars_dropped) = match cap.unit {
        Unit::Bytes => (decoded, false),
        Unit::Chars => clip(&decoded, cap),
    };
    Captured {
        text,
        total,
        truncated: bytes_dropped || chars_dropped,
    }
}

/// Apply `cap` to text already decoded; returns the kept text and whether
/// anything was cut. A byte cap cuts at the nearest character boundary inside
/// the limit.
pub(crate) fn clip(text: &str, cap: Cap) -> (String, bool) {
    match cap.unit {
        Unit::Chars => {
            let n = text.chars().count();
            if n <= cap.limit {
                return (text.to_string(), false);
            }
            let kept: String = match cap.keep {
                Keep::Head => text.chars().take(cap.limit).collect(),
                Keep::Tail => text.chars().skip(n - cap.limit).collect(),
            };
            (kept, true)
        }
        Unit::Bytes => {
            if text.len() <= cap.limit {
                return (text.to_string(), false);
            }
            match cap.keep {
                Keep::Head => {
                    let mut end = cap.limit;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    (text[..end].to_string(), true)
                }
                Keep::Tail => {
                    let mut start = text.len() - cap.limit;
                    while !text.is_char_boundary(start) {
                        start += 1;
                    }
                    (text[start..].to_string(), true)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(limit: usize, keep: Keep, unit: Unit) -> Cap {
        Cap { limit, keep, unit }
    }

    async fn read(input: &[u8], c: Cap) -> Captured {
        read_capped(Some(input), c).await
    }

    #[tokio::test]
    async fn byte_head_keeps_prefix_and_counts_total() {
        let got = read(b"abcdefghij", cap(4, Keep::Head, Unit::Bytes)).await;
        assert_eq!(got.text, "abcd");
        assert_eq!(got.total, 10);
        assert!(got.truncated);

        let fits = read(b"abc", cap(4, Keep::Head, Unit::Bytes)).await;
        assert_eq!(fits.text, "abc");
        assert!(!fits.truncated);
    }

    #[tokio::test]
    async fn byte_tail_keeps_suffix_across_many_chunks() {
        let input: Vec<u8> = (0..50_000u32).map(|i| b'a' + (i % 26) as u8).collect();
        let got = read(&input, cap(100, Keep::Tail, Unit::Bytes)).await;
        assert_eq!(got.text.as_bytes(), &input[input.len() - 100..]);
        assert_eq!(got.total, 50_000);
        assert!(got.truncated);
    }

    #[tokio::test]
    async fn char_head_counts_characters_not_bytes() {
        // 10 two-byte characters = 20 bytes; a 10-char cap keeps all of them.
        let s = "é".repeat(10);
        let got = read(s.as_bytes(), cap(10, Keep::Head, Unit::Chars)).await;
        assert_eq!(got.text, s);
        assert!(!got.truncated);
        assert_eq!(got.total, 20);

        let got = read(s.as_bytes(), cap(3, Keep::Head, Unit::Chars)).await;
        assert_eq!(got.text, "ééé");
        assert!(got.truncated);

        // four-byte characters far past the byte budget: whole characters only
        let wide = "😀".repeat(1000);
        let got = read(wide.as_bytes(), cap(7, Keep::Head, Unit::Chars)).await;
        assert_eq!(got.text, "😀".repeat(7));
        assert!(got.truncated);
    }

    #[tokio::test]
    async fn char_tail_never_starts_mid_character() {
        let wide = format!("{}end", "😀".repeat(1000));
        let got = read(wide.as_bytes(), cap(5, Keep::Tail, Unit::Chars)).await;
        assert_eq!(got.text, "😀😀end");
        assert!(got.truncated);
        assert!(!got.text.contains('\u{FFFD}'));

        let mixed = format!("{}{}", "a".repeat(3), "é".repeat(4));
        let got = read(mixed.as_bytes(), cap(4, Keep::Tail, Unit::Chars)).await;
        assert_eq!(got.text, "éééé");
        assert!(got.truncated);
    }

    #[tokio::test]
    async fn aside_preset_numbers() {
        let p = CapturePolicy::aside();
        let big = "x".repeat(60 * 1024);
        let out = read(big.as_bytes(), p.stdout_cap).await;
        assert_eq!(out.text.chars().count(), 50 * 1024);
        assert!(out.truncated);
        let err = read(format!("{}TAIL", "y".repeat(5000)).as_bytes(), p.stderr_cap).await;
        assert_eq!(err.text.chars().count(), 2 * 1024);
        assert!(err.text.ends_with("TAIL"));
        let (fail, cut) = clip(&out.text, p.failure_stdout_cap);
        assert_eq!(fail.chars().count(), 2 * 1024);
        assert!(cut);
    }

    #[tokio::test]
    async fn dispatch_preset_numbers() {
        let p = CapturePolicy::dispatch();
        let big = "x".repeat(300 * 1024);
        let out = read(big.as_bytes(), p.stdout_cap).await;
        assert_eq!(out.text.len(), 200 * 1024);
        assert_eq!(out.total, 300 * 1024);
        assert!(out.truncated);
        let err = read(
            format!("HEAD{}", "y".repeat(20 * 1024)).as_bytes(),
            p.stderr_cap,
        )
        .await;
        assert_eq!(err.text.len(), 16 * 1024);
        assert!(err.text.starts_with("HEAD"));
        assert_eq!(p.failure_stdout_cap, p.stdout_cap);
        let (same, cut) = clip(&out.text, p.failure_stdout_cap);
        assert_eq!(same.len(), out.text.len());
        assert!(!cut);
    }

    #[test]
    fn clip_bytes_respects_character_boundaries() {
        let s = "aé"; // 1 + 2 bytes
        assert_eq!(
            clip(s, cap(2, Keep::Head, Unit::Bytes)),
            ("a".to_string(), true)
        );
        assert_eq!(
            clip(s, cap(1, Keep::Tail, Unit::Bytes)),
            ("".to_string(), true)
        );
        assert_eq!(
            clip(s, cap(2, Keep::Tail, Unit::Bytes)),
            ("é".to_string(), true)
        );
        assert_eq!(
            clip(s, cap(3, Keep::Head, Unit::Bytes)),
            (s.to_string(), false)
        );
    }

    #[tokio::test]
    async fn missing_stream_is_empty() {
        let got = read_capped::<&[u8]>(None, cap(4, Keep::Head, Unit::Bytes)).await;
        assert_eq!(got.text, "");
        assert_eq!(got.total, 0);
        assert!(!got.truncated);
    }
}
