//! Parse and evaluate server generation state for compose scenarios.

pub fn parse_current_generation(text: &str) -> Option<u64> {
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("current_generation") {
            continue;
        }
        let value = line.split('=').nth(1)?.trim().trim_matches('"');
        if let Ok(n) = value.parse::<u64>() {
            return Some(n);
        }
    }
    None
}

pub fn check_minimum(text: &str, minimum: u64) -> Result<u64, String> {
    match parse_current_generation(text) {
        Some(cur) if cur >= minimum => Ok(cur),
        Some(cur) => Err(format!(
            "current_generation={cur} is below minimum {minimum}"
        )),
        None => Err("current_generation not found in input".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_current_generation_from_toml() {
        let text = r"
current_generation = 4
";
        assert_eq!(parse_current_generation(text), Some(4));
    }

    #[test]
    fn check_minimum_passes_and_fails() {
        let text = "current_generation = 10\n";
        assert_eq!(check_minimum(text, 4).expect("ok"), 10);
        assert!(check_minimum(text, 11).is_err());
    }
}
