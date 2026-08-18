mod report_rendering;
mod segment_tiling;

use crate::common::{fixture_in, fixtures_dir_in};

const MODULE_NAME: &str = "sessions";

pub fn fixture(name: &str) -> std::path::PathBuf {
    fixture_in(MODULE_NAME, name)
}

pub fn fixtures_dir() -> std::path::PathBuf {
    fixtures_dir_in(MODULE_NAME)
}
