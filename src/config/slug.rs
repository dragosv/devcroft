const MAX_LEN: usize = 32;

/// `[sandbox].name` becomes a hostname label (`<name>.devcroft`) and a state
/// directory component, so it is restricted to `[a-z0-9][a-z0-9-]{0,31}`.
pub fn is_valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_LEN {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Best-effort slug suggestion for a name that fails [`is_valid_name`],
/// e.g. `"My Project"` -> `"my-project"`.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        let lower = c.to_ascii_lowercase();
        if lower.is_ascii_lowercase() || lower.is_ascii_digit() {
            slug.push(lower);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.truncate(MAX_LEN);
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "sandbox".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_names() {
        assert!(is_valid_name("myproj"));
        assert!(is_valid_name("my-proj-2"));
        assert!(is_valid_name("a"));
        assert!(is_valid_name(&"a".repeat(32)));
    }

    #[test]
    fn invalid_names() {
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("My Project"));
        assert!(!is_valid_name("-leading-dash"));
        assert!(!is_valid_name("Has_Underscore"));
        assert!(!is_valid_name(&"a".repeat(33)));
    }

    #[test]
    fn slugify_matches_spec_example() {
        assert_eq!(slugify("My Project"), "my-project");
    }

    #[test]
    fn slugify_collapses_and_trims() {
        assert_eq!(slugify("  Foo__Bar!! "), "foo-bar");
        assert_eq!(slugify("---"), "sandbox");
        assert_eq!(slugify(""), "sandbox");
    }

    #[test]
    fn slugify_result_is_always_valid() {
        for input in ["My Project", "  Foo__Bar!! ", "---", "", "already-ok"] {
            assert!(is_valid_name(&slugify(input)));
        }
    }
}
