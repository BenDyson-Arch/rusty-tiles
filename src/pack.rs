//! Read-only archive utilities.
//!
//! These codec helpers retain their finite container inspection scope; they do
//! not certify semantic tileset correctness or inherit package F0 acceptance.
//! Packaging mutations belong to `package`, with a typed request and receipt.
pub use crate::archive3tz::TZ_INDEX_NAME;
use crate::{archive3tz, Error};
use std::path::Path;

pub fn list_zip_names(path: &Path) -> Result<Vec<String>, Error> {
    archive3tz::list_zip_names(path).map_err(Error::from)
}
pub fn validate_3tz(path: &Path) -> Result<(), Error> {
    archive3tz::validate_3tz(path).map_err(Error::from)
}
