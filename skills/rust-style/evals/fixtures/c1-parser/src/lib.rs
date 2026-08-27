pub fn parse_port(input: &str) -> u16 {
    input.parse().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_port() {
        assert_eq!(parse_port("8080"), 8080);
    }
}
