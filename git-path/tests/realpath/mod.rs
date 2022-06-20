use git_path::{realpath::Error, realpath_opts};
use std::path::Path;
use tempfile::tempdir;

#[test]
fn assorted() {
    let cwd = tempdir().unwrap();
    let cwd = cwd.path();
    let symlinks_disabled = 0;

    assert!(
        matches!(realpath_opts("", cwd, symlinks_disabled), Err(Error::EmptyPath)),
        "Empty path is not allowed"
    );

    assert_eq!(
        realpath_opts("b/.git", cwd, symlinks_disabled).unwrap(),
        cwd.join("b").join(".git"),
        "relative paths are prefixed with current dir"
    );

    assert_eq!(
        realpath_opts("b//.git", cwd, symlinks_disabled).unwrap(),
        cwd.join("b").join(".git"),
        "empty path components are ignored"
    );

    assert_eq!(
        realpath_opts("./tmp/.git", cwd, symlinks_disabled).unwrap(),
        cwd.join("tmp").join(".git"),
        "path starting with dot is relative and is prefixed with current dir"
    );

    assert_eq!(
        realpath_opts("./tmp/a/./.git", cwd, symlinks_disabled).unwrap(),
        cwd.join("tmp").join("a").join(".git"),
        "all ./ path components are ignored unless they the one at the beginning of the path"
    );

    assert_eq!(
        realpath_opts("./b/../tmp/.git", cwd, symlinks_disabled).unwrap(),
        cwd.join("tmp").join(".git"),
        "dot dot goes to parent path component"
    );

    {
        #[cfg(not(target_os = "windows"))]
        let absolute_path = Path::new("/c/d/.git");
        #[cfg(target_os = "windows")]
        let absolute_path = Path::new("C:\\c\\d\\.git");
        assert_eq!(
            realpath_opts(absolute_path, cwd, symlinks_disabled).unwrap(),
            absolute_path,
            "absolute path without symlinks has nothing to resolve and remains unchanged"
        );
    }
}

#[test]
fn link_cycle_is_detected() {
    let tmp_dir = canonicalized_tempdir().unwrap();
    let dir = tmp_dir.path();
    let link_name = "link";
    let link_destination = dir.join(link_name);
    let link_path = dir.join(link_name);
    create_symlink(&link_path, &link_destination).unwrap();
    let max_symlinks = 8;

    assert!(
        matches!(
            realpath_opts(link_path.join(".git"), "", max_symlinks),
            Err(Error::MaxSymlinksExceeded { max_symlinks: 8 })
        ),
        "link cycle is detected"
    );
}

#[test]
fn symlink_with_absolute_path_gets_expanded() {
    let tmp_dir = canonicalized_tempdir().unwrap();
    let dir = tmp_dir.path();
    let link_from = dir.join("a").join("b").join("tmp_p_q_link");
    let link_to = dir.join("p").join("q");
    create_symlink(&link_from, &link_to).unwrap();
    let max_symlinks = 8;
    assert_eq!(
        realpath_opts(link_from.join(".git"), tmp_dir, max_symlinks).unwrap(),
        link_to.join(".git"),
        "symlink with absolute path gets expanded"
    );
}

#[test]
fn symlink_to_relative_path_gets_expanded_into_absolute_path() {
    let cwd = canonicalized_tempdir().unwrap();
    let dir = cwd.path();
    let link_name = "pq_link";
    create_symlink(&dir.join("r").join(link_name), &Path::new("p").join("q")).unwrap();
    assert_eq!(
        realpath_opts(Path::new(link_name).join(".git"), dir.join("r"), 8).unwrap(),
        dir.join("r").join("p").join("q").join(".git"),
        "symlink to relative path gets expanded into absolute path"
    );
}

#[test]
fn symlink_processing_is_disabled_if_the_value_is_zero() {
    let cwd = canonicalized_tempdir().unwrap();
    let link_name = "x_link";
    create_symlink(
        &cwd.path().join(link_name),
        Path::new("link destination does not exist"),
    )
    .unwrap();
    assert!(
        matches!(
            realpath_opts(&Path::new(link_name).join(".git"), &cwd, 0),
            Err(Error::MaxSymlinksExceeded { max_symlinks: 0 })
        ),
        "symlink processing is disabled if the value is zero"
    );
}

pub fn create_symlink(from: impl AsRef<Path>, to: impl AsRef<Path>) -> std::io::Result<()> {
    std::fs::create_dir_all(from.as_ref().parent().unwrap())?;

    #[cfg(not(target_os = "windows"))]
    {
        std::os::unix::fs::symlink(to, from)
    }

    #[cfg(target_os = "windows")]
    std::os::windows::fs::symlink_file(to, from)
}

fn canonicalized_tempdir() -> crate::Result<tempfile::TempDir> {
    #[cfg(windows)]
    let canonicalized_tempdir = std::env::temp_dir();

    #[cfg(not(windows))]
    let canonicalized_tempdir = std::env::temp_dir().canonicalize()?;

    Ok(tempfile::tempdir_in(canonicalized_tempdir)?)
}
