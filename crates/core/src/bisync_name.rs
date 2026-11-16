// SPDX-License-Identifier: GPL-3.0-only
//! How bisync turns a local path into the part of its working-file names that identifies it.
//!
//! bisync names its listing and lock files after the session, which embeds both paths. For the
//! local path we observe (and the keeper has always assumed) that the leading slash is dropped
//! and every `/` becomes `_`. rclone's exact rule (it may also replace other characters) could
//! not be confirmed from its source while writing this, so the keeper's listing lookup and the
//! migration's renames must both go through [`flatten`]: then they can never disagree, and a
//! change in rclone's rule is one edit here.
//!
//! TODO(integrator): `keeper::listing_file` still has its own copy of this expression; switch
//! it to [`flatten`].

use std::path::Path;

/// The local path as it appears inside a bisync session name.
#[must_use]
pub fn flatten(p: &Path) -> String {
    p.to_string_lossy()
        .trim_start_matches('/')
        .replace('/', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_the_leading_slash_and_flattens_separators() {
        assert_eq!(flatten(Path::new("/a/b c/d")), "a_b c_d");
    }
}
