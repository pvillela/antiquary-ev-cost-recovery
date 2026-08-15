use std::path::{Path, PathBuf};

pub fn fixtures_dir_in(project: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(project);
    assert!(p.exists(), "missing fixtures directory {p:?}");
    p
}

/// Resolve a fixture path, failing loudly if it is not there.
pub fn fixture_in(project: &str, name: &str) -> PathBuf {
    let p = fixtures_dir_in(project).join(name);
    assert!(p.exists(), "missing fixture {p:?}");
    p
}
