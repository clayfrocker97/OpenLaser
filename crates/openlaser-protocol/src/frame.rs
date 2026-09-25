// SPDX-License-Identifier: GPL-3.0-or-later

//! The datagram layout shared by every exchange with the controller.
//!
//! The card speaks a private variant of Modbus over UDP. A frame is a six-byte
//! header followed by a body:
//!
//! | Bytes | Field | Encoding |
//! |---|---|---|
//! | 0..2 | transaction | big-endian; the reply echoes it |
//! | 2..4 | checksum of bytes 4.. | CRC-16/MODBUS; big-endian in requests, **little-endian in responses** |
//! | 4..6 | declared length | big-endian; the number of bytes after byte 6 |
//! | 6 | unit | always 0 |
//! | 7 | function | `0x30` read words, `0x40` write words |
//! | 8..12 | register address | little-endian |
//! | 12..14 | word count | little-endian |
//! | 14.. | values | little-endian 32-bit words |
//!
//! A read request and a write response carry no values. A write request and a
//! read response carry exactly `count` words.
//!
//! This codec verifies checksums on replies.

/// Upper bound on the word count of one frame.
pub const MAX_WORDS: u16 = 4096;
/// Largest frame the codec accepts: a body carrying [`MAX_WORDS`] values.
pub const MAX_FRAME_BYTES: usize = MIN_FRAME_BYTES + MAX_WORDS as usize * WORD_BYTES;

/// Smallest possible frame: the header plus a body with no values.
const MIN_FRAME_BYTES: usize = HEADER_BYTES + BODY_HEADER_BYTES;
const HEADER_BYTES: usize = 6;
const BODY_HEADER_BYTES: usize = 8;
const WORD_BYTES: usize = 4;
const UNIT: u8 = 0;

/// What a frame asks the controller to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Function {
    /// Read `count` words starting at the address.
    Read,
    /// Write the carried words starting at the address.
    Write,
}

impl Function {
    /// The function as it appears in byte 7 of the frame.
    #[must_use]
    pub const fn byte(self) -> u8 {
        match self {
            Self::Read => 0x30,
            Self::Write => 0x40,
        }
    }

    const fn from_byte(byte: u8) -> Result<Self, FrameError> {
        match byte {
            0x30 => Ok(Self::Read),
            0x40 => Ok(Self::Write),
            other => Err(FrameError::Function(other)),
        }
    }
}

/// Which way a frame travels. The checksum byte order depends on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// From the host to the controller.
    Request,
    /// From the controller to the host.
    Response,
}

impl Direction {
    /// Whether a frame travelling this way with this function carries values.
    const fn carries_values(self, function: Function) -> bool {
        matches!(
            (self, function),
            (Self::Request, Function::Write) | (Self::Response, Function::Read)
        )
    }

    const fn checksum_bytes(self, checksum: u16) -> [u8; 2] {
        match self {
            Self::Request => checksum.to_be_bytes(),
            Self::Response => checksum.to_le_bytes(),
        }
    }

    const fn read_checksum(self, bytes: [u8; 2]) -> u16 {
        match self {
            Self::Request => u16::from_be_bytes(bytes),
            Self::Response => u16::from_le_bytes(bytes),
        }
    }
}

/// One datagram, decoded.
///
/// `count` is the number of words the frame refers to. For a read request it is
/// the number requested and `values` is empty; for every other shape it equals
/// `values.len()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Which way the frame travels.
    pub direction: Direction,
    /// Correlates a response with its request.
    pub transaction: u16,
    /// Read or write.
    pub function: Function,
    /// The first register the frame refers to.
    pub address: u32,
    /// How many 32-bit words the frame refers to.
    pub count: u16,
    /// The carried words, if the shape carries any.
    pub values: Vec<u32>,
}

impl Frame {
    /// A request to read `count` words starting at `address`.
    pub fn read(transaction: u16, address: u32, count: u16) -> Result<Self, FrameError> {
        Self {
            direction: Direction::Request,
            transaction,
            function: Function::Read,
            address,
            count,
            values: Vec::new(),
        }
        .checked()
    }

    /// A request to write `values` starting at `address`.
    pub fn write(transaction: u16, address: u32, values: Vec<u32>) -> Result<Self, FrameError> {
        let count = u16::try_from(values.len()).map_err(|_| FrameError::Count(u16::MAX))?;
        Self {
            direction: Direction::Request,
            transaction,
            function: Function::Write,
            address,
            count,
            values,
        }
        .checked()
    }

    /// The controller's reply to this request: a read reply carries `values`,
    /// a write reply carries none.
    pub fn response(&self, values: Vec<u32>) -> Result<Self, FrameError> {
        Self {
            direction: Direction::Response,
            transaction: self.transaction,
            function: self.function,
            address: self.address,
            count: self.count,
            values,
        }
        .checked()
    }

    /// The bytes that go on the wire.
    pub fn encode(&self) -> Result<Vec<u8>, FrameError> {
        self.check_shape()?;
        let body_len = BODY_HEADER_BYTES + self.values.len() * WORD_BYTES;
        let declared = u16::try_from(body_len)
            .map_err(|_| FrameError::TooLarge { actual: HEADER_BYTES + body_len })?;
        let mut bytes = Vec::with_capacity(HEADER_BYTES + body_len);
        bytes.extend_from_slice(&self.transaction.to_be_bytes());
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&declared.to_be_bytes());
        bytes.push(UNIT);
        bytes.push(self.function.byte());
        bytes.extend_from_slice(&self.address.to_le_bytes());
        bytes.extend_from_slice(&self.count.to_le_bytes());
        for value in &self.values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let checksum = crc16(&bytes[4..]);
        bytes[2..4].copy_from_slice(&self.direction.checksum_bytes(checksum));
        Ok(bytes)
    }

    /// Parses bytes that travelled in `direction`, verifying length, checksum
    /// and shape.
    pub fn decode(bytes: &[u8], direction: Direction) -> Result<Self, FrameError> {
        if let Some(refusal) = exception(bytes, direction) {
            return Err(refusal);
        }
        if bytes.len() < MIN_FRAME_BYTES {
            return Err(FrameError::TooShort { actual: bytes.len() });
        }
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(FrameError::TooLarge { actual: bytes.len() });
        }
        let declared = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
        let actual = bytes.len() - HEADER_BYTES;
        if declared != actual {
            return Err(FrameError::Length { declared, actual });
        }
        let expected = crc16(&bytes[4..]);
        let found = direction.read_checksum([bytes[2], bytes[3]]);
        if expected != found {
            return Err(FrameError::Checksum { expected, found });
        }
        if bytes[6] != UNIT {
            return Err(FrameError::Unit(bytes[6]));
        }
        let data = &bytes[MIN_FRAME_BYTES..];
        if !data.len().is_multiple_of(WORD_BYTES) {
            return Err(FrameError::Shape("value bytes are not whole words"));
        }
        Self {
            direction,
            transaction: u16::from_be_bytes([bytes[0], bytes[1]]),
            function: Function::from_byte(bytes[7])?,
            address: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            count: u16::from_le_bytes([bytes[12], bytes[13]]),
            values: data
                .chunks_exact(WORD_BYTES)
                .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
                .collect(),
        }
        .checked()
    }

    /// Checks that `response` answers this request: same transaction,
    /// function, address and count.
    pub fn check_response(&self, response: &Self) -> Result<(), FrameError> {
        if self.direction != Direction::Request || response.direction != Direction::Response {
            return Err(FrameError::Shape("matching needs a request and a response"));
        }
        let fields = [
            ("transaction", u32::from(self.transaction), u32::from(response.transaction)),
            ("function", u32::from(self.function.byte()), u32::from(response.function.byte())),
            ("address", self.address, response.address),
            ("count", u32::from(self.count), u32::from(response.count)),
        ];
        for (field, expected, found) in fields {
            if expected != found {
                return Err(FrameError::Mismatch { field, expected, found });
            }
        }
        Ok(())
    }

    fn checked(self) -> Result<Self, FrameError> {
        self.check_shape()?;
        Ok(self)
    }

    fn check_shape(&self) -> Result<(), FrameError> {
        if self.count == 0 || self.count > MAX_WORDS {
            return Err(FrameError::Count(self.count));
        }
        let carried = self.values.len();
        if self.direction.carries_values(self.function) {
            if carried != usize::from(self.count) {
                return Err(FrameError::Shape("carried words must equal the count"));
            }
        } else if carried != 0 {
            return Err(FrameError::Shape("read requests and write responses carry no words"));
        }
        Ok(())
    }
}

/// A Modbus exception reply: the header, the unit, the request's function
/// with its high bit set, and one code byte. The controller sends one when
/// it refuses a request, which it then has not carried out.
fn exception(bytes: &[u8], direction: Direction) -> Option<FrameError> {
    let [t0, t1, c0, c1, 0, 3, UNIT, function, code] = *bytes else { return None };
    if function & 0x80 == 0 || direction.read_checksum([c0, c1]) != crc16(&bytes[4..]) {
        return None;
    }
    Some(FrameError::Exception {
        transaction: u16::from_be_bytes([t0, t1]),
        function: function & 0x7f,
        code,
    })
}

/// What a Modbus exception code means, in words.
fn exception_meaning(code: u8) -> &'static str {
    match code {
        1 => "function not supported",
        2 => "register address not accepted",
        3 => "value not accepted",
        4 => "controller failure",
        5 => "accepted, still working",
        6 => "controller busy",
        _ => "unknown exception",
    }
}

/// Why bytes could not be read as a frame, or a frame could not be written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The controller refused the request, and did not carry it out.
    #[error("the controller refused the request: Modbus exception {code} ({})", exception_meaning(*code))]
    Exception {
        /// The refused request's transaction.
        transaction: u16,
        /// The refused request's function byte.
        function: u8,
        /// The exception code.
        code: u8,
    },
    /// Fewer bytes than the smallest frame.
    #[error("frame is {actual} bytes; the minimum is {MIN_FRAME_BYTES}")]
    TooShort {
        /// The length seen.
        actual: usize,
    },
    /// More bytes than the largest frame.
    #[error("frame is {actual} bytes; the maximum is {MAX_FRAME_BYTES}")]
    TooLarge {
        /// The length seen.
        actual: usize,
    },
    /// The declared length disagrees with the bytes present.
    #[error("declared length {declared} does not match the {actual} bytes after the header")]
    Length {
        /// The value in bytes 4..6.
        declared: usize,
        /// The bytes present after byte 6.
        actual: usize,
    },
    /// The checksum does not cover the bytes present.
    #[error("checksum {found:#06x} does not match the computed {expected:#06x}")]
    Checksum {
        /// CRC-16/MODBUS over bytes 4..
        expected: u16,
        /// The value in bytes 2..4, read in the direction's byte order.
        found: u16,
    },
    /// The unit byte is not zero.
    #[error("unit byte is {0:#04x}; the controller answers only unit 0")]
    Unit(u8),
    /// The function byte is neither read nor write.
    #[error("function byte {0:#04x} is neither read (0x30) nor write (0x40)")]
    Function(u8),
    /// The word count is zero or above [`MAX_WORDS`].
    #[error("word count {0} is outside 1..={MAX_WORDS}")]
    Count(u16),
    /// The carried words do not fit the direction and function.
    #[error("{0}")]
    Shape(&'static str),
    /// A response field differs from its request.
    #[error("response {field} is {found}, the request had {expected}")]
    Mismatch {
        /// Which field differs.
        field: &'static str,
        /// The request's value.
        expected: u32,
        /// The response's value.
        found: u32,
    },
}

/// CRC-16/MODBUS: reflected polynomial `0xA001`, initial value `0xFFFF`, no
/// final inversion. The controller computes it over bytes 4.. of a frame.
#[must_use]
pub fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0xffff, |crc, &byte| {
        (0..8).fold(crc ^ u16::from(byte), |crc, _| {
            if crc & 1 == 0 { crc >> 1 } else { (crc >> 1) ^ 0xa001 }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first request in the idle capture: read 18 words at 10000.
    const IDLE_READ: [u8; 14] =
        [0xd5, 0x24, 0xf3, 0x1d, 0x00, 0x08, 0x00, 0x30, 0x10, 0x27, 0x00, 0x00, 0x12, 0x00];

    /// The CRC-16/MODBUS check value for the standard test string.
    #[test]
    fn crc16_matches_the_modbus_check_value() {
        assert_eq!(crc16(b"123456789"), 0x4b37);
    }

    /// A read request built here is byte-identical to the first request in the idle capture.
    #[test]
    fn read_request_encodes_like_the_capture() {
        let frame = Frame::read(0xd524, 10000, 18).unwrap();
        assert_eq!(frame.encode().unwrap(), IDLE_READ);
    }

    /// The same captured bytes decode back to the request that built them.
    #[test]
    fn captured_request_decodes() {
        let frame = Frame::decode(&IDLE_READ, Direction::Request).unwrap();
        assert_eq!(frame, Frame::read(0xd524, 10000, 18).unwrap());
    }

    /// A response stores its checksum little-endian, decodes in the response direction, and is rejected as a request.
    #[test]
    fn responses_store_the_checksum_little_endian() {
        let request = Frame::read(7, 1000, 2).unwrap();
        let response = request.response(vec![1, 2]).unwrap();
        let bytes = response.encode().unwrap();
        let checksum = crc16(&bytes[4..]);
        assert_eq!([bytes[2], bytes[3]], checksum.to_le_bytes());
        assert_eq!(Frame::decode(&bytes, Direction::Response).unwrap(), response);
        assert!(matches!(
            Frame::decode(&bytes, Direction::Request),
            Err(FrameError::Checksum { .. })
        ));
    }

    /// A write request carries its words and survives an encode and decode round trip.
    #[test]
    fn write_request_round_trips() {
        let frame = Frame::write(0x1234, 101, vec![118, 5, 1]).unwrap();
        let bytes = frame.encode().unwrap();
        assert_eq!(bytes.len(), 14 + 3 * 4);
        assert_eq!(Frame::decode(&bytes, Direction::Request).unwrap(), frame);
    }

    /// Zero or oversized counts and words that do not fit the direction and function are refused.
    #[test]
    fn shapes_are_enforced() {
        assert!(matches!(Frame::read(1, 1000, 0), Err(FrameError::Count(0))));
        assert!(matches!(Frame::read(1, 1000, MAX_WORDS + 1), Err(FrameError::Count(_))));
        assert!(matches!(Frame::write(1, 101, vec![]), Err(FrameError::Count(0))));
        let read = Frame::read(1, 1000, 2).unwrap();
        assert!(matches!(read.response(vec![1]), Err(FrameError::Shape(_))));
        let write = Frame::write(1, 101, vec![1]).unwrap();
        assert!(matches!(write.response(vec![1]), Err(FrameError::Shape(_))));
        assert!(write.response(vec![]).is_ok());
    }

    /// A nine-byte refusal, the write function with its high bit set and
    /// one code byte, is read as the controller's exception, not as a
    /// truncated frame; with a bad checksum it stays a truncated frame.
    #[test]
    fn exception_replies_are_read_as_refusals() {
        let mut bytes = vec![0x12, 0x34, 0, 0, 0, 3, 0, 0xC0, 6];
        let checksum = crc16(&bytes[4..]);
        bytes[2..4].copy_from_slice(&Direction::Response.checksum_bytes(checksum));
        let refusal = Frame::decode(&bytes, Direction::Response).unwrap_err();
        assert_eq!(refusal, FrameError::Exception { transaction: 0x1234, function: 0x40, code: 6 });
        assert!(refusal.to_string().contains("controller busy"), "{refusal}");
        bytes[2] ^= 0xff;
        assert_eq!(
            Frame::decode(&bytes, Direction::Response),
            Err(FrameError::TooShort { actual: 9 })
        );
    }

    /// Truncated frames, wrong declared lengths, bad unit or function bytes, zero counts and stray words are each rejected with the matching error.
    #[test]
    fn malformed_bytes_are_rejected() {
        assert!(matches!(
            Frame::decode(&IDLE_READ[..13], Direction::Request),
            Err(FrameError::TooShort { actual: 13 })
        ));
        let mut short_length = IDLE_READ;
        short_length[5] = 0x07;
        assert!(matches!(
            Frame::decode(&short_length, Direction::Request),
            Err(FrameError::Length { declared: 7, actual: 8 })
        ));
        assert!(matches!(decode_with(6, 0xff), Err(FrameError::Unit(0xff))));
        assert!(matches!(decode_with(7, 0xff), Err(FrameError::Function(0xff))));
        assert!(matches!(decode_with(12, 0x00), Err(FrameError::Count(0))));
        // A read request that carries a word.
        let mut with_word = IDLE_READ.to_vec();
        with_word.extend_from_slice(&[0, 0, 0, 0]);
        with_word[5] = 0x0c;
        let checksum = crc16(&with_word[4..]).to_be_bytes();
        with_word[2..4].copy_from_slice(&checksum);
        assert!(matches!(Frame::decode(&with_word, Direction::Request), Err(FrameError::Shape(_))));
    }

    /// The captured read request with one byte changed and the checksum redone.
    fn decode_with(index: usize, byte: u8) -> Result<Frame, FrameError> {
        let mut bytes = IDLE_READ;
        bytes[index] = byte;
        let checksum = crc16(&bytes[4..]).to_be_bytes();
        bytes[2..4].copy_from_slice(&checksum);
        Frame::decode(&bytes, Direction::Request)
    }

    /// A response must echo transaction, function, address and count, and matching needs a request first.
    #[test]
    fn responses_must_answer_their_request() {
        let request = Frame::read(9, 1000, 36).unwrap();
        let reply = request.response(vec![0; 36]).unwrap();
        assert!(request.check_response(&reply).is_ok());
        let other = Frame::read(10, 1000, 36).unwrap().response(vec![0; 36]).unwrap();
        assert!(matches!(
            request.check_response(&other),
            Err(FrameError::Mismatch { field: "transaction", expected: 9, found: 10 })
        ));
        assert!(matches!(reply.check_response(&request), Err(FrameError::Shape(_))));
    }
}
