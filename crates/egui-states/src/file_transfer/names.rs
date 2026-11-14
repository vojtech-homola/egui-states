//! Validation of the file names in an offer.
//!
//! A name in an offer is only a hint from the sender; the receiver chooses
//! where files go. These checks make sure a name can never point outside the
//! folder the receiver chose, and that the receiver's system can store it.

use std::collections::HashSet;

use crate::protocol::file_transfer::OfferBody;

/// Checks the names that every peer must accept: relative, `/` between
/// components, no `.` or `..`, no control characters, no duplicates, and no
/// file that is also the folder of another.
pub(crate) fn validate_offer(body: &OfferBody) -> Result<(), String> {
    if body.name.is_empty() {
        return Err("the transfer has no name".to_string());
    }
    if body.name.chars().any(char::is_control) {
        return Err("the transfer name contains control characters".to_string());
    }
    if body.files.is_empty() {
        return Err("the transfer has no files".to_string());
    }

    // Every total of the transfer must fit, or the accounting would wrap.
    body.files
        .iter()
        .try_fold(0u64, |total, file| total.checked_add(file.size))
        .ok_or("the files are too large together")?;

    let mut files = HashSet::with_capacity(body.files.len());
    for file in &body.files {
        validate_name(&file.name)?;
        if !files.insert(file.name.as_str()) {
            return Err(format!("the file name '{}' appears twice", file.name));
        }
    }
    // A file "a" and a file "a/b" cannot both exist.
    for file in &body.files {
        let mut prefix = file.name.as_str();
        while let Some((parent, _)) = prefix.rsplit_once('/') {
            if files.contains(parent) {
                return Err(format!(
                    "'{parent}' is both a file and the folder of '{}'",
                    file.name
                ));
            }
            prefix = parent;
        }
    }
    Ok(())
}

/// Checks one relative file name.
pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a file name is empty".to_string());
    }
    if name.chars().any(|c| c.is_control() || c == '\\') {
        return Err(format!(
            "the file name '{}' contains a backslash or control characters",
            name.escape_debug()
        ));
    }
    if name.starts_with('/') {
        return Err(format!("the file name '{name}' is absolute"));
    }
    for component in name.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(format!("the file name '{name}' has an invalid component"));
        }
    }
    // A drive prefix such as "C:" would make a Windows path absolute.
    if name.as_bytes().get(1) == Some(&b':') {
        return Err(format!("the file name '{name}' starts with a drive"));
    }
    Ok(())
}

/// Checks that this system can store every file of `body` under its name.
pub(crate) fn check_platform(body: &OfferBody) -> Result<(), String> {
    check_names(
        body.files.iter().map(|file| file.name.as_str()),
        cfg!(windows),
        cfg!(any(windows, target_os = "macos")),
    )
}

fn check_names<'a>(
    names: impl Iterator<Item = &'a str>,
    windows: bool,
    case_insensitive: bool,
) -> Result<(), String> {
    let mut folded = HashSet::new();
    for name in names {
        if windows {
            for component in name.split('/') {
                check_windows_component(name, component)?;
            }
        }
        if case_insensitive && !folded.insert(name.to_lowercase()) {
            return Err(format!(
                "the file name '{name}' differs from another only in letter case"
            ));
        }
    }
    Ok(())
}

fn check_windows_component(name: &str, component: &str) -> Result<(), String> {
    if let Some(c) = component
        .chars()
        .find(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err(format!(
            "the file name '{name}' contains '{c}', which Windows does not allow"
        ));
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return Err(format!(
            "the file name '{name}' ends a component with a dot or space, which Windows does not allow"
        ));
    }
    let stem = component.split('.').next().unwrap_or("").trim_end();
    let reserved = matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || {
        let upper = stem.to_ascii_uppercase();
        (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0'
    };
    if reserved {
        return Err(format!(
            "the file name '{name}' uses a device name that Windows reserves"
        ));
    }
    Ok(())
}

/// The name of a file saved by a browser, which cannot create folders.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn flatten(name: &str) -> String {
    name.replace('/', "_")
}

/// Checks that the flattened names of `body` stay distinct.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn check_flattened(body: &OfferBody) -> Result<(), String> {
    let mut names = HashSet::new();
    for file in &body.files {
        let flat = flatten(&file.name);
        if !names.insert(flat.to_lowercase()) {
            return Err(format!(
                "the file name '{}' collides with another once folders are removed",
                file.name
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::file_transfer::FileEntry;

    fn body(names: &[&str]) -> OfferBody {
        OfferBody {
            name: "set".to_string(),
            files: names
                .iter()
                .map(|name| FileEntry {
                    name: name.to_string(),
                    size: 1,
                    modified: None,
                })
                .collect(),
        }
    }

    #[test]
    fn structural_validation() {
        for good in [
            &["a.txt"][..],
            &["run42/0001.tiff", "run42/0002.tiff"],
            &["a b/c d.e"],
            &["ünïcode/名前.txt"],
        ] {
            assert!(validate_offer(&body(good)).is_ok(), "{good:?}");
        }
        for bad in [
            &[""][..],
            &["/etc/passwd"],
            &["../x"],
            &["a/../x"],
            &["a/./x"],
            &["a//x"],
            &["a/"],
            &["a\\b"],
            &["C:x"],
            &["a\nb"],
            &["a", "a"],
            &["a", "a/b"],
            &[],
        ] {
            assert!(validate_offer(&body(bad)).is_err(), "{bad:?}");
        }
        let mut unnamed = body(&["a"]);
        unnamed.name.clear();
        assert!(validate_offer(&unnamed).is_err());
    }

    #[test]
    fn windows_rules() {
        let check = |names: &[&str]| check_names(names.iter().copied(), true, true);
        assert!(check(&["a.txt", "dir/b.txt", "com0.txt", "console.txt"]).is_ok());
        for bad in [
            &["a:b"][..],
            &["a?"],
            &["dir/a*"],
            &["a."],
            &["a "],
            &["CON"],
            &["dir/nul.txt"],
            &["com1.log"],
            &["LPT9"],
            &["A.txt", "a.TXT"],
        ] {
            assert!(check(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn case_sensitive_systems_accept_case_variants() {
        assert!(check_names(["A", "a"].into_iter(), false, false).is_ok());
        assert!(check_names(["A", "a"].into_iter(), false, true).is_err());
    }

    #[test]
    fn flattening_keeps_names_distinct() {
        assert_eq!(flatten("run42/0001.tiff"), "run42_0001.tiff");
        assert!(check_flattened(&body(&["a/b", "c"])).is_ok());
        assert!(check_flattened(&body(&["a/b", "a_b"])).is_err());
    }
}
