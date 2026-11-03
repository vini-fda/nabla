//! Messages on a stream, which is a sequence of bytes: each is sent as a frame of a length, in
//! four bytes, little-endian, and then that many bytes.
//!
//! This is the one thing the stream glue of every platform shares, so it is here, and sans-IO:
//! the native glue and the browser's both push in what they read and take frames out.

/// The frame for `payload`.
///
/// # Panics
/// If `payload` is longer than `u32::MAX` bytes, which no message is.
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let length = u32::try_from(payload.len()).expect("a message is far shorter than 4 GiB");
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// A peer announced a frame longer than the limit: the stream is not worth reading on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLong;

/// The frames in a stream, as bytes arrive in pieces of any size.
#[derive(Debug)]
pub struct FrameReader {
    buf: Vec<u8>,
    limit: usize,
}

impl FrameReader {
    /// A reader that refuses frames longer than `limit` bytes, which it learns of from the
    /// length alone: it never holds more than `limit + 4` bytes.
    pub const fn new(limit: usize) -> Self {
        Self {
            buf: Vec::new(),
            limit,
        }
    }

    /// Take in what the stream gave.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole frame, if one has arrived.
    ///
    /// # Errors
    /// [`TooLong`] if the next frame announces more than the limit.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, TooLong> {
        let Some(header) = self.buf.first_chunk::<4>() else {
            return Ok(None);
        };
        let length = u32::from_le_bytes(*header) as usize;
        if length > self.limit {
            return Err(TooLong);
        }
        if self.buf.len() < 4 + length {
            return Ok(None);
        }
        let rest = self.buf.split_off(4 + length);
        let mut frame = std::mem::replace(&mut self.buf, rest);
        frame.drain(..4);
        Ok(Some(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_survive_any_split() {
        let messages: Vec<Vec<u8>> = vec![vec![1, 2, 3], vec![], vec![9; 300], vec![7]];
        let wire: Vec<u8> = messages.iter().flat_map(|m| frame(m)).collect();
        for piece in [1, 2, 3, 5, 64, wire.len()] {
            let mut reader = FrameReader::new(1000);
            let mut got = Vec::new();
            for chunk in wire.chunks(piece) {
                reader.push(chunk);
                while let Some(frame) = reader.next_frame().expect("within the limit") {
                    got.push(frame);
                }
            }
            assert_eq!(got, messages, "pieces of {piece}");
        }
    }

    #[test]
    fn a_long_frame_is_refused_on_its_length_alone() {
        let mut reader = FrameReader::new(10);
        reader.push(&u32::MAX.to_le_bytes());
        assert_eq!(reader.next_frame(), Err(TooLong));
        let mut reader = FrameReader::new(10);
        reader.push(&frame(&[0; 10]));
        assert!(matches!(reader.next_frame(), Ok(Some(_))));
        reader.push(&frame(&[0; 11]));
        assert_eq!(reader.next_frame(), Err(TooLong));
    }
}
