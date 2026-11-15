//! Validation of the file names in an offer.
//!
//! A name in an offer is only a hint from the sender; the receiver chooses
//! where files go. These checks make sure a name can never point outside the
//! folder the receiver chose, and that the receiver's system can store it.

use std::collections::HashSet;

use crate::protocol::file_transfer::OfferBody;

/// The longest name of a transfer or a file, in bytes. No system stores a
/// longer path, and the limit bounds the work a name from the other peer
/// can cause.
pub(crate) const NAME_MAX_LEN: usize = 4096;

/// Checks the names that every peer must accept: relative, `/` between
/// components, no `.` or `..`, no backslash or control characters, and none
/// longer than [`NAME_MAX_LEN`], for the files and the transfer alike; among
/// the files no duplicates and no file that is also the folder of another.
///
/// It runs on what the other peer sent, so its work grows only in proportion
/// to the size of the offer.
pub(crate) fn validate_offer(body: &OfferBody) -> Result<(), String> {
    // The name of the transfer is a file name too: the receiver may save
    // under it, so it cannot lead out of a folder either.
    validate_name(&body.name, "transfer name")?;
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
        validate_name(&file.name, "file name")?;
        if !files.insert(file.name.as_str()) {
            return Err(format!("the file name '{}' appears twice", file.name));
        }
    }
    // A file "a" and a file "a/b" cannot both exist.
    let mut sorted: Vec<&str> = files.into_iter().collect();
    sorted.sort_unstable();
    if let Some((file, below)) = file_and_folder(&sorted) {
        return Err(format!(
            "'{}' is both a file and the folder of '{}'",
            sorted[file], sorted[below]
        ));
    }
    Ok(())
}

/// Finds a name in `sorted` that is also the folder of another one; returns
/// the positions of both. The files of the folder "a" start at the first
/// name that is not before "a/", so one look per name finds them.
fn file_and_folder(sorted: &[&str]) -> Option<(usize, usize)> {
    sorted.iter().enumerate().find_map(|(file, name)| {
        let below = || name.bytes().chain(std::iter::once(b'/'));
        let first = sorted.partition_point(|other| other.bytes().lt(below()));
        let rest = sorted.get(first)?.strip_prefix(name)?;
        rest.starts_with('/').then_some((file, first))
    })
}

/// Checks one relative name; `what` it names, a "file name" or the
/// "transfer name", only words the error.
fn validate_name(name: &str, what: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err(format!("a {what} is empty"));
    }
    if name.len() > NAME_MAX_LEN {
        return Err(format!("a {what} is longer than {NAME_MAX_LEN} bytes"));
    }
    if name.chars().any(|c| c.is_control() || c == '\\') {
        return Err(format!(
            "the {what} '{}' contains a backslash or control characters",
            name.escape_debug()
        ));
    }
    if name.starts_with('/') {
        return Err(format!("the {what} '{name}' is absolute"));
    }
    for component in name.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(format!("the {what} '{name}' has an invalid component"));
        }
    }
    // A drive prefix such as "C:" would make a Windows path absolute.
    if name.as_bytes().get(1) == Some(&b':') {
        return Err(format!("the {what} '{name}' starts with a drive"));
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
    // Each name without its letter case, with the name itself.
    let mut folded = Vec::new();
    for name in names {
        if windows {
            for component in name.split('/') {
                check_windows_component(name, component)?;
            }
        }
        if case_insensitive {
            folded.push((name.to_lowercase(), name));
        }
    }
    folded.sort_unstable();
    if let Some(pair) = folded.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        return Err(format!(
            "the file name '{}' differs from another only in letter case",
            pair[1].1
        ));
    }
    // The offer has no file that is the folder of another, but here "Data"
    // is also the folder of "data/a".
    let keys: Vec<&str> = folded.iter().map(|(key, _)| key.as_str()).collect();
    if let Some((file, below)) = file_and_folder(&keys) {
        return Err(format!(
            "'{}' is a file and, in another letter case, the folder of '{}'",
            folded[file].1, folded[below].1
        ));
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
    fn a_file_cannot_be_the_folder_of_another() {
        // Names that sort between a file and the files of its folder do not
        // hide them.
        for bad in [
            &["a", "a-b", "a/b"][..],
            &["a/b", "a"],
            &["a/b/c/d", "x", "a/b"],
            &["a.b/c", "a", "a.b"],
        ] {
            let error = validate_offer(&body(bad)).unwrap_err();
            assert!(error.contains("is both a file and the folder"), "{error}");
        }
        for good in [
            &["a", "a-b", "ab/c", "a.b/c"][..],
            &["a/b", "a/c", "ab"],
            &["a/b/c", "a/b/d", "a/e"],
        ] {
            assert!(validate_offer(&body(good)).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn names_have_a_length_limit() {
        let longest = format!("{}ab", "a/".repeat(NAME_MAX_LEN / 2 - 1));
        assert_eq!(longest.len(), NAME_MAX_LEN);
        assert!(validate_offer(&body(&[&longest])).is_ok());

        // The error does not repeat a name of any length.
        let error = validate_offer(&body(&[&format!("{longest}c")])).unwrap_err();
        assert!(error.len() < 100, "{error}");

        let mut named = body(&["a"]);
        named.name = "a".repeat(NAME_MAX_LEN + 1);
        assert!(validate_offer(&named).is_err());

        // As many of the deepest names as an offer holds.
        let deep: Vec<String> = (0..240)
            .map(|i| format!("{i:04}/{}", "a/".repeat(NAME_MAX_LEN / 2 - 3)) + "a")
            .collect();
        let deep: Vec<&str> = deep.iter().map(String::as_str).collect();
        assert!(deep.iter().all(|name| name.len() == NAME_MAX_LEN));
        assert!(validate_offer(&body(&deep)).is_ok());
    }

    /// A receiver may save a transfer under its name, so the name cannot
    /// lead out of a folder any more than the name of a file.
    #[test]
    fn the_transfer_name_is_checked_like_a_file_name() {
        let named = |name: &str| {
            let mut body = body(&["a"]);
            body.name = name.to_string();
            validate_offer(&body)
        };
        for good in ["set", "run 42", "run42/results", "a.tar.gz"] {
            assert!(named(good).is_ok(), "{good:?}");
        }
        for bad in [
            "",
            ".",
            "../x",
            "a/../b",
            "/etc/passwd",
            "a\\b",
            "C:x",
            "a//b",
            "a\nb",
        ] {
            let error = named(bad).unwrap_err();
            assert!(error.contains("transfer name"), "{bad:?}: {error}");
        }
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

    /// On a system without letter case, a folder is the file of the same
    /// name in another case.
    #[test]
    fn a_file_cannot_be_the_folder_of_another_in_another_case() {
        let check =
            |names: &[&str], insensitive| check_names(names.iter().copied(), false, insensitive);
        for bad in [
            &["Data/a.bin", "data"][..],
            &["data", "Data/a.bin"],
            &["a/B", "x", "A/b/c"],
        ] {
            assert!(validate_offer(&body(bad)).is_ok(), "{bad:?}");
            assert!(check(bad, false).is_ok(), "{bad:?}");
            let error = check(bad, true).unwrap_err();
            assert!(error.contains("in another letter case"), "{error}");
        }
        // Folders that differ only in case are one folder there.
        assert!(check(&["Data/a", "data/b", "DATA-c"], true).is_ok());
    }

    #[test]
    fn flattening_keeps_names_distinct() {
        assert_eq!(flatten("run42/0001.tiff"), "run42_0001.tiff");
        assert!(check_flattened(&body(&["a/b", "c"])).is_ok());
        assert!(check_flattened(&body(&["a/b", "a_b"])).is_err());
    }
}
