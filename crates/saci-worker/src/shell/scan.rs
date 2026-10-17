//! Finds the end-of-command marker in a shell output stream.

use saci_core::redaction::{OutputCapture, Redactor};

/// Separator around marker payloads (ASCII record separator).
pub const SEPARATOR: u8 = 0x1e;

/// Splits a stream into command output and the marker that ends it.
#[derive(Debug)]
pub struct Scanner {
    marker: Vec<u8>,
    held: Vec<u8>,
    payload: Option<Vec<u8>>,
}

/// Result of feeding a chunk.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Fed {
    /// Bytes that belong to the command's output.
    pub output: Vec<u8>,
    /// The marker and its payload have been read completely.
    pub complete: bool,
}

impl Scanner {
    /// A scanner for `marker`, which is followed by a payload and
    /// [`SEPARATOR`].
    #[must_use]
    pub fn new(marker: &[u8]) -> Self {
        Self {
            marker: marker.to_vec(),
            held: Vec::new(),
            payload: None,
        }
    }

    /// Feeds the next chunk of the stream.
    pub fn feed(&mut self, chunk: &[u8]) -> Fed {
        if let Some(payload) = &mut self.payload {
            return match chunk.iter().position(|byte| *byte == SEPARATOR) {
                Some(end) => {
                    payload.extend_from_slice(&chunk[..end]);
                    Fed {
                        output: Vec::new(),
                        complete: true,
                    }
                }
                None => {
                    payload.extend_from_slice(chunk);
                    Fed::default()
                }
            };
        }
        self.held.extend_from_slice(chunk);
        if let Some(start) = find(&self.held, &self.marker) {
            let rest = self.held.split_off(start + self.marker.len());
            self.held.truncate(start);
            let output = std::mem::take(&mut self.held);
            self.payload = Some(Vec::new());
            let fed = self.feed(&rest);
            return Fed { output, ..fed };
        }
        let keep = self.marker.len().saturating_sub(1).min(self.held.len());
        let tail = self.held.split_off(self.held.len() - keep);
        Fed {
            output: std::mem::replace(&mut self.held, tail),
            complete: false,
        }
    }

    /// Bytes held back in case they start a marker; output once the stream
    /// ends without one.
    pub fn flush(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.held)
    }

    /// The marker payload read so far.
    #[must_use]
    pub fn payload(&self) -> Option<&[u8]> {
        self.payload.as_deref()
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// One output stream of a running command.
#[derive(Debug)]
pub struct Stream {
    scanner: Scanner,
    capture: OutputCapture,
    /// The end marker was read.
    pub done: bool,
    /// The pipe closed.
    pub closed: bool,
}

impl Stream {
    /// A stream ended by `marker`, keeping `limit` bytes.
    #[must_use]
    pub fn new(marker: &[u8], limit: u64, redactor: &Redactor) -> Self {
        Self {
            scanner: Scanner::new(marker),
            capture: OutputCapture::new(limit, redactor),
            done: false,
            closed: false,
        }
    }

    /// Handles a chunk, or the end of the pipe for `None`.
    pub fn receive(&mut self, chunk: Option<Vec<u8>>) {
        let Some(chunk) = chunk else {
            self.closed = true;
            let rest = self.scanner.flush();
            self.capture.push(&rest);
            return;
        };
        if self.done {
            return;
        }
        let fed = self.scanner.feed(&chunk);
        self.capture.push(&fed.output);
        self.done = fed.complete;
    }

    /// Whether the command is still producing this stream.
    #[must_use]
    pub const fn open(&self) -> bool {
        !self.done && !self.closed
    }

    /// The exit status carried by the marker, if one was read.
    #[must_use]
    pub fn status(&self) -> Option<i32> {
        let payload = self.scanner.payload()?;
        std::str::from_utf8(payload).ok()?.trim().parse().ok()
    }

    /// The captured, redacted output.
    #[must_use]
    pub fn finish(self, redactor: &Redactor) -> saci_core::evidence::CapturedText {
        self.capture.finish(redactor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKER: &[u8] = b"\x1eEND:";

    #[test]
    fn output_before_the_marker_is_kept_and_the_payload_parsed() {
        let mut stream = Stream::new(MARKER, 100, &Redactor::new());
        stream.receive(Some(b"hello\n\x1eEN".to_vec()));
        assert!(stream.open());
        stream.receive(Some(b"D:4".to_vec()));
        stream.receive(Some(b"2\x1elater".to_vec()));
        assert!(stream.done);
        assert_eq!(stream.status(), Some(42));
        assert_eq!(stream.finish(&Redactor::new()).text, "hello\n");
    }

    #[test]
    fn partial_markers_are_released_when_they_do_not_match() {
        let mut scanner = Scanner::new(MARKER);
        assert!(scanner.feed(b"ab\x1eE").output.is_empty());
        assert_eq!(scanner.feed(b"x").output, b"a");
        assert_eq!(scanner.flush(), b"b\x1eEx");
    }

    #[test]
    fn a_closed_stream_keeps_held_bytes() {
        let mut stream = Stream::new(MARKER, 100, &Redactor::new());
        stream.receive(Some(b"bye\x1eE".to_vec()));
        stream.receive(None);
        assert!(!stream.open());
        assert_eq!(stream.status(), None);
        assert_eq!(stream.finish(&Redactor::new()).text, "bye\u{1e}E");
    }
}
