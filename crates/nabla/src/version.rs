//! What this build is, as `build.rs` works it out.

/// `v` and the package's version in a release build. In any other, the commit's short hash, or
/// `dev` with uncommitted changes (or outside of a git repository). A `&'static str`.
macro_rules! build_version {
    () => {
        env!("NABLA_BUILD_VERSION")
    };
}
pub(crate) use build_version;

#[cfg(test)]
mod tests {
    #[test]
    fn the_build_says_what_it_is() {
        let version = build_version!();
        let hash = version.len() >= 7 && version.chars().all(|c| c.is_ascii_hexdigit());
        let release = version == concat!("v", env!("CARGO_PKG_VERSION"));
        assert!(version == "dev" || hash || release, "{version}");
    }
}
