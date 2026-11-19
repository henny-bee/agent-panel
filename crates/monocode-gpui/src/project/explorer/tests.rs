use super::{changed_range, typed_folder};
use std::path::Path;

#[test]
fn typed_folders_resolve_home_relative_and_absolute_paths() {
    assert_eq!(
        typed_folder("~/code", "/p", "/home/u"),
        Path::new("/home/u/code")
    );
    assert_eq!(
        typed_folder("sub/dir", "/p", "/home/u"),
        Path::new("/p/sub/dir")
    );
    assert_eq!(typed_folder("/mnt", "/p", "/home/u"), Path::new("/mnt"));
}

#[test]
fn expanding_a_folder_only_touches_the_inserted_rows() {
    let old = [1, 2, 3, 4];
    let new = [1, 2, 9, 9, 3, 4];
    assert_eq!(changed_range(&old, &new), (2, 2, 4));
}

#[test]
fn collapsing_and_flipping_a_row() {
    let old = [1, 2, 5, 6, 3];
    let new = [1, 7, 3];
    assert_eq!(changed_range(&old, &new), (1, 4, 2));
    assert_eq!(changed_range(&old, &old), (5, 5, 5));
    assert_eq!(changed_range::<i32>(&[], &[1]), (0, 0, 1));
}
