//! Reads the model name out of a monitor's EDID block (the 128+ bytes every
//! monitor reports about itself). Pure parsing, no I/O, so it runs everywhere.

const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];

/// What we use from an EDID block.
#[derive(Debug, PartialEq, Clone)]
pub struct Edid {
    /// Three-letter manufacturer code, e.g. "DEL".
    pub manufacturer: String,
    pub product: u16,
    pub serial: u32,
    /// Monitor name from the "display product name" descriptor, if present.
    pub model: Option<String>,
}

fn checksum_ok(block: &[u8]) -> bool {
    block.iter().fold(0u8, |a, b| a.wrapping_add(*b)) == 0
}

/// Parse the first 128-byte block. None if it is not a valid EDID.
pub fn parse(data: &[u8]) -> Option<Edid> {
    if data.len() < 128 || data[..8] != HEADER || !checksum_ok(&data[..128]) {
        return None;
    }
    // Manufacturer: three 5-bit letters (1 = 'A') packed into two big-endian bytes.
    let m = u16::from_be_bytes([data[8], data[9]]);
    let letter = |v: u16| {
        let v = (v & 0x1f) as u8;
        if (1..=26).contains(&v) {
            (b'A' + v - 1) as char
        } else {
            '?'
        }
    };
    let manufacturer: String = [letter(m >> 10), letter(m >> 5), letter(m)].iter().collect();
    let product = u16::from_le_bytes([data[10], data[11]]);
    let serial = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);

    // Four 18-byte descriptors at offsets 54, 72, 90, 108. A descriptor that
    // starts with three zero bytes is text; tag 0xFC is the product name.
    let mut model = None;
    for i in 0..4 {
        let d = &data[54 + i * 18..54 + (i + 1) * 18];
        if d[0] == 0 && d[1] == 0 && d[2] == 0 && d[3] == 0xfc {
            let text: String = d[5..18]
                .iter()
                .take_while(|b| **b != 0x0a && **b != 0)
                .filter(|b| (0x20..0x7f).contains(*b))
                .map(|b| *b as char)
                .collect();
            let text = text.trim().to_string();
            if !text.is_empty() {
                model = Some(text);
            }
        }
    }
    Some(Edid { manufacturer, product, serial, model })
}

impl Edid {
    /// Name shown next to "Monitor N". Falls back to the manufacturer code and
    /// product code when the monitor reports no name.
    pub fn display_name(&self) -> String {
        match &self.model {
            Some(m) => m.clone(),
            None => format!("{} {:04X}", self.manufacturer, self.product),
        }
    }

    /// A stable id for this physical monitor: model plus serial number.
    #[cfg_attr(windows, allow(dead_code))]
    pub fn stable_id(&self) -> String {
        format!("{}{:04x}-{:08x}", self.manufacturer.to_ascii_lowercase(), self.product, self.serial)
    }
}

#[cfg(test)]
pub fn sample(manufacturer: &str, product: u16, serial: u32, name: Option<&str>) -> Vec<u8> {
    let mut d = vec![0u8; 128];
    d[..8].copy_from_slice(&HEADER);
    let l: Vec<u16> = manufacturer.bytes().map(|b| (b - b'A' + 1) as u16).collect();
    let m = (l[0] << 10) | (l[1] << 5) | l[2];
    d[8..10].copy_from_slice(&m.to_be_bytes());
    d[10..12].copy_from_slice(&product.to_le_bytes());
    d[12..16].copy_from_slice(&serial.to_le_bytes());
    if let Some(n) = name {
        let o = 54;
        d[o + 3] = 0xfc;
        let mut text = n.as_bytes().to_vec();
        if text.len() < 13 {
            text.push(0x0a);
        }
        while text.len() < 13 {
            text.push(0x20);
        }
        d[o + 5..o + 18].copy_from_slice(&text[..13]);
    }
    let sum = d[..127].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    d[127] = 0u8.wrapping_sub(sum);
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_model_manufacturer_and_serial() {
        let e = parse(&sample("DEL", 0x40e5, 0x1234_abcd, Some("DELL U2720Q"))).unwrap();
        assert_eq!(e.manufacturer, "DEL");
        assert_eq!(e.product, 0x40e5);
        assert_eq!(e.serial, 0x1234_abcd);
        assert_eq!(e.model.as_deref(), Some("DELL U2720Q"));
        assert_eq!(e.display_name(), "DELL U2720Q");
        assert_eq!(e.stable_id(), "del40e5-1234abcd");
    }

    #[test]
    fn full_length_name_without_terminator() {
        let e = parse(&sample("GSM", 1, 2, Some("LG ULTRAGEAR 27"))).unwrap();
        assert_eq!(e.model.as_deref(), Some("LG ULTRAGEAR"));
    }

    #[test]
    fn missing_name_falls_back_to_codes() {
        let e = parse(&sample("SAM", 0x0d1f, 7, None)).unwrap();
        assert_eq!(e.model, None);
        assert_eq!(e.display_name(), "SAM 0D1F");
    }

    #[test]
    fn rejects_bad_data() {
        assert!(parse(&[]).is_none());
        assert!(parse(&[0u8; 128]).is_none());
        let mut bad_sum = sample("DEL", 1, 1, Some("X"));
        bad_sum[20] ^= 0x01;
        assert!(parse(&bad_sum).is_none());
        let mut bad_header = sample("DEL", 1, 1, Some("X"));
        bad_header[0] = 1;
        assert!(parse(&bad_header).is_none());
        assert!(parse(&sample("DEL", 1, 1, Some("X"))[..100]).is_none());
    }

    #[test]
    fn name_with_junk_bytes_is_cleaned() {
        let mut d = sample("DEL", 1, 1, Some("AB"));
        d[54 + 5 + 3] = 0x07; // control byte inside the text
        d[54 + 5 + 4] = b'C';
        let sum = d[..127].iter().fold(0u8, |a, b| a.wrapping_add(*b));
        d[127] = 0u8.wrapping_sub(sum);
        let e = parse(&d).unwrap();
        assert!(e.model.as_deref().unwrap().starts_with("AB"));
    }
}
