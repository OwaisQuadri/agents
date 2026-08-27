pub fn xor_in_place(bytes: &mut [u8], key: u8) {
    for byte in bytes {
        *byte ^= key;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut bytes = b"codec".to_vec();
        xor_in_place(&mut bytes, 0x5a);
        xor_in_place(&mut bytes, 0x5a);
        assert_eq!(bytes, b"codec");
    }
}
