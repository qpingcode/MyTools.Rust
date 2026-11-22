//! The `platforms` field of plugin.json: the operating systems a plugin runs on, named
//! `windows`, `macos` and `linux`. Omitted or empty means every platform. The host skips a
//! plugin made for other systems, and packaging leaves it out.

/// The name of the system this build runs on.
pub const fn current() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// Whether a plugin declaring `platforms` runs on this system.
pub fn supports(platforms: &[String]) -> bool {
    supports_on(platforms, current())
}

fn supports_on(platforms: &[String], system: &str) -> bool {
    platforms.is_empty()
        || platforms
            .iter()
            .any(|name| name.eq_ignore_ascii_case(system))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_platforms_mean_everywhere() {
        assert!(supports(&[]));
        assert!(supports(&[current().to_owned()]));
    }

    #[test]
    fn listed_platforms_are_matched_by_name() {
        let windows = vec!["Windows".to_owned()];
        assert!(supports_on(&windows, "windows"));
        assert!(!supports_on(&windows, "macos"));
    }
}
