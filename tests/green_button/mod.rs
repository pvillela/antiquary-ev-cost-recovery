mod fixtures_golden;
mod full_feed;
mod invoice;

use crate::common::{fixture_in, fixtures_dir_in};

const MODULE_NAME: &'static str = "green_button";

pub fn fixture(name: &str) -> std::path::PathBuf {
    fixture_in(MODULE_NAME, name)
}

pub fn fixtures_dir() -> std::path::PathBuf {
    fixtures_dir_in(MODULE_NAME)
}
