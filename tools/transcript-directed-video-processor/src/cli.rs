// Manual `--flag value` argument parsing, matching the repo's existing pattern
// (tools/dispatch-baseline, tools/mcp-sync) — no `clap` dependency.

use std::collections::HashMap;

pub struct Flags {
    values: HashMap<String, String>,
}

impl Flags {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut values = HashMap::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            let key = arg
                .strip_prefix("--")
                .ok_or_else(|| format!("unexpected argument: {arg}"))?;
            let value = iter
                .next()
                .ok_or_else(|| format!("--{key} requires a value"))?;
            values.insert(key.to_string(), value.clone());
        }
        Ok(Flags { values })
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn require(&self, key: &str) -> Result<&str, String> {
        self.get(key).ok_or_else(|| format!("--{key} is required"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_flag_value_pairs() {
        let flags = Flags::parse(&args(&["--url", "https://x", "--out", "/tmp/x"])).unwrap();
        assert_eq!(flags.get("url"), Some("https://x"));
        assert_eq!(flags.get("out"), Some("/tmp/x"));
    }

    #[test]
    fn missing_required_flag_is_an_error() {
        let flags = Flags::parse(&args(&["--out", "/tmp/x"])).unwrap();
        assert!(flags.require("url").is_err());
    }

    #[test]
    fn a_dangling_flag_with_no_value_is_an_error() {
        assert!(Flags::parse(&args(&["--out"])).is_err());
    }
}
