use std::path::PathBuf;

pub const DEFAULT_EXCLUDED: &[&str] = &["com.apple.finder"];

pub fn parse(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(String::from)
        .collect()
}

fn path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/killer/exclude"))
}

/// The exclusion list (defaults plus the user's file), and a warning for the UI if the file couldn't be read.
pub fn load() -> (Vec<String>, Option<String>) {
    let mut excluded: Vec<String> = DEFAULT_EXCLUDED.iter().map(|s| s.to_string()).collect();
    let Some(path) = path() else {
        return (excluded, None);
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => excluded.extend(parse(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return (excluded, Some(format!("Couldn't read {}: {e}", path.display()))),
    }
    (excluded, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skips_blanks_and_comments() {
        let text = "# my list\ncom.tinyspeck.slackmacgap\n\n  com.apple.Safari  \n";
        assert_eq!(parse(text), vec!["com.tinyspeck.slackmacgap", "com.apple.Safari"]);
    }
}
