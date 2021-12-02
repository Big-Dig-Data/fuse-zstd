use rstest::*;
use std::{
    fs,
    io::Write,
    mem,
    os::linux::fs::MetadataExt,
    os::unix::{
        fs::FileExt,
        io::{AsRawFd, RawFd},
    },
    path,
};
use zstd::block::compress;

#[path = "utils.rs"]
pub mod utils;

#[fixture]
fn mounted_fs() -> utils::FuseZstdProcess {
    let zstd_process = utils::FuseZstdProcess::new(false);
    zstd_process
}

#[rstest]
fn parallel_write(mounted_fs: utils::FuseZstdProcess) {
    // parallel open should behave in the same way as in data_dir
    let mp = mounted_fs.mount_point();
    let dd = mounted_fs.data_dir();

    let parallel_write = |path: path::PathBuf| {
        let mut file1 = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(path.join("file.txt"))
            .unwrap();

        let mut file2 = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(path.join("file.txt"))
            .unwrap();

        let mut file3 = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(path.join("file.txt"))
            .unwrap();

        file2.write(b"SECOND").unwrap();
        file1.write(b"FIRST").unwrap();
        mem::drop(file1); // should close the file

        file3.write(b"THIRD").unwrap();
        mem::drop(file2); // should close the file
        mem::drop(file3); // should close the file

        fs::read_to_string(path.join("file.txt")).unwrap()
    };

    let dd_data = parallel_write(dd);
    let mp_data = parallel_write(mp);
    assert_eq!(dd_data, mp_data);
}

#[rstest]
fn append(mounted_fs: utils::FuseZstdProcess) {
    // parallel open should behave in the same way as in data_dir
    let mp = mounted_fs.mount_point();
    let dd = mounted_fs.data_dir();

    let append = |path: path::PathBuf| {
        fs::write(path.join("file.txt"), b"BASIC").unwrap();

        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(path.join("file.txt"))
            .unwrap();

        file.write(b"APPENDED").unwrap();
        mem::drop(file);

        fs::read_to_string(path.join("file.txt")).unwrap()
    };

    let dd_data = append(dd);
    let mp_data = append(mp);
    assert_eq!(dd_data, mp_data);
}
