//! What this build calls itself. A local or patched build sets `PDFCRAFT_BUILD_LABEL` at compile
//! time (`PDFCRAFT_BUILD_LABEL=titlebar cargo build`, or through `packaging/linux/package.sh`), so
//! it can be told from the release of the same version. The version itself stays the release's:
//! the update check compares it with published releases.

/// The release version from the workspace `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The compile-time build label, if any.
pub const LABEL: Option<&str> = option_env!("PDFCRAFT_BUILD_LABEL");

/// MaewPDF's own version, such as `1.0.0` (Thai edition). From 1.0.0 it counts on its own and no
/// longer follows the upstream release it is built on, which is [`VERSION`] (not shown: About
/// says MaewPDF is built on PdfCraft, without a version). `thai/VERSION` is the one place it is written; `thai/package-windows.ps1` names the
/// packages after it too. Releases before 1.0.0 were `<upstream>-th.<n>`, such as `0.21.1-th.3`.
const EDITION_FILE: &str = include_str!("../../../thai/VERSION");

/// MaewPDF's version, such as `1.0.0` (Thai edition).
pub fn edition_version() -> &'static str {
    EDITION_FILE.trim()
}

/// The version as shown to people: `1.0.0`, or `1.0.0 (titlebar)` for a labelled build (Thai
/// edition: MaewPDF's version, not the upstream one).
pub fn version() -> String {
    describe(edition_version(), LABEL)
}

fn describe(version: &str, label: Option<&str>) -> String {
    match label.map(str::trim).filter(|l| !l.is_empty()) {
        Some(label) => format!("{version} ({label})"),
        None => version.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{VERSION, describe, edition_version};

    #[test]
    fn a_label_follows_the_version_and_a_blank_one_is_ignored() {
        assert_eq!(describe("0.7.1", None), "0.7.1");
        assert_eq!(describe("0.7.1", Some("")), "0.7.1");
        assert_eq!(describe("0.7.1", Some("  ")), "0.7.1");
        assert_eq!(describe("0.7.1", Some("titlebar")), "0.7.1 (titlebar)");
    }

    /// MaewPDF numbers its own releases from 1.0.0 (Thai edition): `thai/VERSION` is
    /// `<major>.<minor>.<patch>`, all whole numbers, major at least 1, so the update check finds
    /// it newer than every `0.x.y-th.n` release before it.
    #[test]
    fn the_edition_version_is_maewpdfs_own() {
        let v = edition_version();
        let parts: Vec<Option<u64>> = v.split('.').map(|p| p.parse::<u64>().ok()).collect();
        assert_eq!(parts.len(), 3, "thai/VERSION is {v:?}, not <major>.<minor>.<patch>");
        assert!(parts.iter().all(Option::is_some), "thai/VERSION is {v:?}: each part must be a whole number");
        assert!(parts.first().copied().flatten().is_some_and(|major| major >= 1), "thai/VERSION is {v:?}: MaewPDF counts from 1.0.0");
        assert!(!VERSION.is_empty(), "the upstream version shown beside it is missing");
    }
}
