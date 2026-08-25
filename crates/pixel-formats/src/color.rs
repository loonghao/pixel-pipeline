//! Small shared color helpers used across contracts.

/// Parse a `#rrggbb` or `#rrggbbaa` hex color into RGBA bytes.
pub fn parse_hex_color(s: &str) -> Result<[u8; 4], String> {
    let h = s.strip_prefix('#').unwrap_or(s);
    let bytes = match h.len() {
        6 => {
            let v = u32::from_str_radix(h, 16).map_err(|e| e.to_string())?;
            [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]
        }
        8 => {
            let v = u32::from_str_radix(h, 16).map_err(|e| e.to_string())?;
            [(v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8]
        }
        _ => {
            return Err(format!(
                "invalid hex color '{s}' (expected #rrggbb or #rrggbbaa)"
            ))
        }
    };
    Ok(bytes)
}

/// Format RGB bytes as an uppercase `#RRGGBB` string.
pub fn to_hex_rgb(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rgb_and_rgba() {
        assert_eq!(parse_hex_color("#2b1009").unwrap(), [0x2b, 0x10, 0x09, 255]);
        assert_eq!(parse_hex_color("#01020304").unwrap(), [1, 2, 3, 4]);
    }

    #[test]
    fn rejects_bad_len() {
        assert!(parse_hex_color("#fff").is_err());
    }

    #[test]
    fn roundtrip_hex() {
        assert_eq!(to_hex_rgb([0x2b, 0x10, 0x09]), "#2B1009");
    }
}
