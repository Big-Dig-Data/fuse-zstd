use rstest::*;
use std::{fs, io::Write, mem, os::linux::fs::MetadataExt, path};

#[path = "utils.rs"]
pub mod utils;

#[fixture]
fn mounted_fs_no_convert() -> utils::FuseZstdProcess {
    let zstd_process = utils::FuseZstdProcess::new(false);
    zstd_process
}

#[fixture]
fn mounted_fs_convert() -> utils::FuseZstdProcess {
    let zstd_process = utils::FuseZstdProcess::new(true);
    zstd_process
}

#[rstest]
#[case::no_convert(mounted_fs_no_convert())]
#[case::convert(mounted_fs_convert())]
fn parallel_write(#[case] mounted_fs: utils::FuseZstdProcess) {
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
        file1.sync_all().unwrap();
        mem::drop(file1); // should close the file

        file3.write(b"THIRD").unwrap();
        file2.sync_all().unwrap();
        mem::drop(file2); // should close the file
        file3.sync_all().unwrap();
        mem::drop(file3); // should close the file

        fs::read_to_string(path.join("file.txt")).unwrap()
    };

    let mp_data = parallel_write(mp);
    let dd_data = parallel_write(dd);
    assert_eq!(dd_data, mp_data);
}

#[rstest]
#[case::no_convert(mounted_fs_no_convert())]
#[case::convert(mounted_fs_convert())]
fn append(#[case] mounted_fs: utils::FuseZstdProcess) {
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
        file.sync_all().unwrap();
        mem::drop(file);

        fs::read_to_string(path.join("file.txt")).unwrap()
    };

    let dd_data = append(dd);
    let mp_data = append(mp);
    assert_eq!(dd_data, mp_data);
}

#[rstest]
#[case::no_convert(mounted_fs_no_convert())]
#[case::convert(mounted_fs_convert())]
fn source_file_updates(#[case] mounted_fs: utils::FuseZstdProcess) {
    let mp = mounted_fs.mount_point();
    let dd = mounted_fs.data_dir();

    // Create file and make sure it is sync
    fs::write(mp.join("file.txt"), b"KEEP").unwrap();
    assert_eq!(fs::read_to_string(mp.join("file.txt")).unwrap(), "KEEP");

    // no write, no fsync
    let original_ino = fs::metadata(dd.join("file.txt.zst")).unwrap().st_ino();

    let file1 = fs::OpenOptions::new()
        .write(true)
        .open(mp.join("file.txt"))
        .unwrap();

    let file2 = fs::OpenOptions::new()
        .append(true)
        .open(mp.join("file.txt"))
        .unwrap();

    mem::drop(file1);
    mem::drop(file2);

    assert_eq!(
        original_ino,
        fs::metadata(dd.join("file.txt.zst")).unwrap().st_ino(),
        "Open in write modes, but no data written",
    );

    // fsync no write
    let file1 = fs::OpenOptions::new()
        .write(true)
        .open(mp.join("file.txt"))
        .unwrap();

    let file2 = fs::OpenOptions::new()
        .append(true)
        .open(mp.join("file.txt"))
        .unwrap();

    file1.sync_all().unwrap();
    file2.sync_all().unwrap();

    assert_ne!(
        original_ino,
        fs::metadata(dd.join("file.txt.zst")).unwrap().st_ino(),
        "Sync should be performed when manually called fsync",
    );
    mem::drop(file1);
    mem::drop(file2);

    // write, no sync
    let file1 = fs::OpenOptions::new()
        .write(true)
        .open(mp.join("file.txt"))
        .unwrap();

    let mut file2 = fs::OpenOptions::new()
        .append(true)
        .open(mp.join("file.txt"))
        .unwrap();

    file2.write(b"IT").unwrap();
    mem::drop(file1);
    assert_eq!(fs::read_to_string(mp.join("file.txt")).unwrap(), "KEEP");
    mem::drop(file2);
    assert_eq!(fs::read_to_string(mp.join("file.txt")).unwrap(), "KEEPIT");
}

#[rstest]
#[case::convert(mounted_fs_convert())]
fn remove_unconverted_file(#[case] mounted_fs: utils::FuseZstdProcess) {
    let mp = mounted_fs.mount_point();
    let dd = mounted_fs.data_dir();

    {
        let mut file1 = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(dd.join("file.txt"))
            .unwrap();
        file1.write(b"UNCONVERTED").unwrap();
        file1.sync_all().unwrap();
        fs::create_dir_all(dd.join("dir")).unwrap();

        let mut file2 = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(dd.join("dir/file.txt"))
            .unwrap();
        file2.write(b"UNCONVERTED").unwrap();
        file2.sync_all().unwrap();
    }
    assert!(fs::remove_file(mp.join("file.txt")).is_ok());
    assert!(!mp.join("file.txt").exists());
    assert!(fs::remove_file(mp.join("dir/file.txt")).is_ok());
    assert!(!mp.join("dir/file.txt").exists());
}

#[rstest]
#[case::no_convert(mounted_fs_no_convert())]
#[case::convert(mounted_fs_convert())]
fn flush(#[case] mounted_fs: utils::FuseZstdProcess) {
    let mp = mounted_fs.mount_point();
    let dd = mounted_fs.data_dir();

    // Create file and make sure it is synced
    fs::write(mp.join("file.txt"), b"ORIGINAL").unwrap();
    assert_eq!(fs::read_to_string(mp.join("file.txt")).unwrap(), "ORIGINAL");

    let original_ino = fs::metadata(dd.join("file.txt.zst")).unwrap().st_ino();

    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(mp.join("file.txt"))
        .unwrap();

    assert_eq!(
        original_ino,
        fs::metadata(dd.join("file.txt.zst")).unwrap().st_ino(),
        "Opening file doesn't touch compressed file",
    );
    assert_eq!(fs::read_to_string(mp.join("file.txt")).unwrap(), "ORIGINAL");

    file.write(b"OVERRIDE").unwrap();

    assert_eq!(
        original_ino,
        fs::metadata(dd.join("file.txt.zst")).unwrap().st_ino(),
        "Writing doesn't touch compressed file",
    );
    assert_eq!(fs::read_to_string(mp.join("file.txt")).unwrap(), "ORIGINAL");

    // closing cloned fd should trigger flush
    file.try_clone().unwrap();

    assert_ne!(
        original_ino,
        fs::metadata(dd.join("file.txt.zst")).unwrap().st_ino(),
        "Flushing changes compressed file",
    );
    assert_eq!(fs::read_to_string(mp.join("file.txt")).unwrap(), "OVERRIDE");
}

#[rstest]
//#[case::no_convert(mounted_fs_no_convert())]
#[case::convert(mounted_fs_convert())]
fn too_close_write_and_lookup(#[case] mounted_fs: utils::FuseZstdProcess) {
    let mp = mounted_fs.mount_point();

    let mut file1 = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(mp.join("file.txt"))
        .unwrap();
    file1.write(b"TOO CLOSE").unwrap();
    mem::drop(file1);
    assert_eq!(
        fs::read_to_string(mp.join("file.txt")).unwrap(),
        "TOO CLOSE"
    );
    let mut file1 = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(mp.join("file2.txt"))
        .unwrap();
    file1.write(b"2 CLOSE").unwrap();
    mem::drop(file1);
    assert_eq!(fs::read_to_string(mp.join("file2.txt")).unwrap(), "2 CLOSE");
}
