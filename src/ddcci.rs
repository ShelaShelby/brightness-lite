//! The DDC/CI wire format used on Linux (I2C address 0x37): building a
//! "get VCP" / "set VCP" request and checking a reply. Pure functions, so they
//! are tested everywhere. Only VCP codes 0x10 and 0x12 are ever passed in.

/// 7-bit I2C address of the DDC/CI channel.
pub const ADDRESS: u16 = 0x37;
/// The same address as the first byte on the wire (shifted left by one).
const DEST: u8 = 0x6e;
/// Source byte the host puts first in every request.
const HOST: u8 = 0x51;
/// Virtual host address, used in the checksum of replies.
const REPLY_SEED: u8 = 0x50;

pub const VCP_BRIGHTNESS: u8 = 0x10;
pub const VCP_CONTRAST: u8 = 0x12;
/// Length of a "get VCP" reply.
pub const REPLY_LEN: usize = 11;

/// Request: read the value of `code`.
pub fn get_request(code: u8) -> [u8; 5] {
    let mut r = [HOST, 0x82, 0x01, code, 0];
    r[4] = r[..4].iter().fold(DEST, |a, b| a ^ b);
    r
}

/// Request: set `code` to `value`.
pub fn set_request(code: u8, value: u16) -> [u8; 7] {
    let [hi, lo] = value.to_be_bytes();
    let mut r = [HOST, 0x84, 0x03, code, hi, lo, 0];
    r[6] = r[..6].iter().fold(DEST, |a, b| a ^ b);
    r
}

#[derive(Debug, PartialEq)]
pub enum ReplyError {
    TooShort,
    BadChecksum,
    /// The monitor does not support this code.
    Unsupported,
    /// Anything else that is not a normal reply.
    Malformed,
}

/// Parse a "get VCP" reply into (current, maximum).
pub fn parse_get_reply(buf: &[u8], code: u8) -> Result<(u32, u32), ReplyError> {
    if buf.len() < REPLY_LEN {
        return Err(ReplyError::TooShort);
    }
    let b = &buf[..REPLY_LEN];
    if b[..REPLY_LEN - 1].iter().fold(REPLY_SEED, |a, x| a ^ x) != b[REPLY_LEN - 1] {
        return Err(ReplyError::BadChecksum);
    }
    // b[0] source (0x6e), b[1] 0x80|len, b[2] opcode 0x02, b[3] result, b[4] code,
    // b[5] type, b[6..8] max, b[8..10] current.
    if b[1] != 0x88 || b[2] != 0x02 || b[4] != code {
        return Err(ReplyError::Malformed);
    }
    if b[3] != 0 {
        return Err(ReplyError::Unsupported);
    }
    let max = u16::from_be_bytes([b[6], b[7]]) as u32;
    let cur = u16::from_be_bytes([b[8], b[9]]) as u32;
    Ok((cur, max))
}

#[cfg(test)]
pub fn make_reply(code: u8, result: u8, max: u16, cur: u16) -> Vec<u8> {
    let mut r = vec![DEST, 0x88, 0x02, result, code, 0x00];
    r.extend_from_slice(&max.to_be_bytes());
    r.extend_from_slice(&cur.to_be_bytes());
    let c = r.iter().fold(REPLY_SEED, |a, b| a ^ b);
    r.push(c);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_request_matches_the_known_bytes() {
        // Same bytes ddcutil sends to read brightness.
        assert_eq!(get_request(0x10), [0x51, 0x82, 0x01, 0x10, 0xac]);
    }

    #[test]
    fn set_request_matches_the_known_bytes() {
        // Brightness 50: 51 84 03 10 00 32, checksum 6e^...
        let r = set_request(0x10, 50);
        assert_eq!(&r[..6], &[0x51, 0x84, 0x03, 0x10, 0x00, 0x32]);
        assert_eq!(r[6], 0x6e ^ 0x51 ^ 0x84 ^ 0x03 ^ 0x10 ^ 0x32);
    }

    #[test]
    fn set_request_carries_both_value_bytes() {
        let r = set_request(0x12, 0x0123);
        assert_eq!((r[4], r[5]), (0x01, 0x23));
        assert_eq!(r[3], 0x12);
    }

    #[test]
    fn reply_roundtrip() {
        let r = make_reply(0x10, 0, 100, 37);
        assert_eq!(parse_get_reply(&r, 0x10), Ok((37, 100)));
    }

    #[test]
    fn reply_errors() {
        let good = make_reply(0x10, 0, 100, 37);
        assert_eq!(parse_get_reply(&good[..5], 0x10), Err(ReplyError::TooShort));
        let mut bad = good.clone();
        bad[8] ^= 1;
        assert_eq!(parse_get_reply(&bad, 0x10), Err(ReplyError::BadChecksum));
        assert_eq!(parse_get_reply(&make_reply(0x10, 1, 0, 0), 0x10), Err(ReplyError::Unsupported));
        assert_eq!(parse_get_reply(&good, 0x12), Err(ReplyError::Malformed));
        let mut wrong_op = good.clone();
        wrong_op[2] = 0x03;
        wrong_op[10] = wrong_op[..10].iter().fold(REPLY_SEED, |a, b| a ^ b);
        assert_eq!(parse_get_reply(&wrong_op, 0x10), Err(ReplyError::Malformed));
    }

    #[test]
    fn extra_bytes_after_the_reply_are_ignored() {
        let mut r = make_reply(0x12, 0, 100, 5);
        r.extend_from_slice(&[0xff, 0xff]);
        assert_eq!(parse_get_reply(&r, 0x12), Ok((5, 100)));
    }
}
