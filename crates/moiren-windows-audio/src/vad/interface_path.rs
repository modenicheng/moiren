use super::ProtocolError;

pub(super) const MAX_DETAIL_BYTES: usize = 128 * 1024;

pub(super) fn decode_detail_path(
    bytes: &[u8],
    header_size: u32,
) -> Result<Vec<u16>, ProtocolError> {
    // SetupAPI's cbSize is architecture-dependent (6 on x86, 8 on x64),
    // whereas DevicePath starts at byte 4 on both. RequiredSize includes NUL.
    if bytes.len() < 8 || bytes.len() > MAX_DETAIL_BYTES || !bytes.len().is_multiple_of(2) {
        return Err(ProtocolError::Inconsistent(
            "invalid interface detail length",
        ));
    }
    if !matches!(header_size, 6 | 8) || bytes[..4] != header_size.to_le_bytes() {
        return Err(ProtocolError::Inconsistent(
            "invalid interface detail cbSize",
        ));
    }
    let path: Vec<_> = bytes[4..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let terminator = path
        .iter()
        .position(|&unit| unit == 0)
        .ok_or(ProtocolError::Inconsistent("unterminated interface path"))?;
    if terminator == 0 || path[terminator + 1..].iter().any(|&unit| unit != 0) {
        return Err(ProtocolError::Inconsistent("invalid interface path suffix"));
    }
    Ok(path[..=terminator].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(path: &[u16], header_size: u32) -> Vec<u8> {
        let mut bytes = header_size.to_le_bytes().to_vec();
        for unit in path {
            bytes.extend(unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn variable_length_paths_are_not_limited_to_max_path() {
        for header_size in [6, 8] {
            let mut path = vec![b'x' as u16; 2048];
            path.push(0);
            assert_eq!(
                decode_detail_path(&detail(&path, header_size), header_size).unwrap(),
                path
            );
        }
    }

    #[test]
    fn malformed_interface_details_are_rejected() {
        for bytes in [
            vec![],
            vec![8, 0, 0, 0],
            detail(&[0], 8),
            detail(&[b'x' as u16], 8),
            detail(&[b'x' as u16, 0], 6),
            detail(&[b'x' as u16, 0, b'y' as u16], 8),
            vec![8, 0, 0, 0, b'x', 0, 0],
            vec![0; MAX_DETAIL_BYTES + 2],
        ] {
            assert!(decode_detail_path(&bytes, 8).is_err());
        }
    }

    #[test]
    fn wide_paths_preserve_unpaired_utf16_units() {
        let path = [b'x' as u16, 0xd800, 0];
        assert_eq!(decode_detail_path(&detail(&path, 8), 8).unwrap(), path);
    }
}
