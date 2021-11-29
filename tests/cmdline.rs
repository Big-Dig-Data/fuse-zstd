use assert_cmd::Command;
use rstest::*;
use std::{fs, path, thread, time};
use zstd::decode_all;

#[path = "utils.rs"]
mod utils;

#[fixture]
fn mounted_fs() -> utils::FuseZstdProcess {
    utils::FuseZstdProcess::new()
}

#[fixture]
fn populated_mounted_fs(mounted_fs: utils::FuseZstdProcess) -> utils::FuseZstdProcess {
    let mp = mounted_fs.mount_point();
    fs::create_dir_all(mp.join("first/second/third")).unwrap();
    fs::create_dir_all(mp.join("first/second/empty")).unwrap();
    fs::write(mp.join("file1.txt"), b"1st file in root").unwrap();
    fs::write(mp.join("first/file1.txt"), b"1st file in first").unwrap();
    fs::write(mp.join("first/file2.txt"), b"2nd file in first").unwrap();
    fs::write(mp.join("first/second/file1.txt"), b"1st file in second").unwrap();
    fs::write(mp.join("first/second/file2.txt"), b"2nd file in second").unwrap();
    fs::write(mp.join("first/second/file3.txt"), b"3rd file in second").unwrap();
    fs::write(
        mp.join("first/second/third/file1.txt"),
        b"1st file in third",
    )
    .unwrap();
    mounted_fs
}

#[rstest]
fn touch(mounted_fs: utils::FuseZstdProcess) {
    Command::new("touch")
        .arg(mounted_fs.mount_point().join("file.txt"))
        .assert()
        .success();

    let zfile = mounted_fs.data_dir().join("file.txt.zst");
    assert!(zfile.exists());
    assert_eq!(decode_all(fs::File::open(zfile).unwrap()).unwrap(), b"");
}

#[rstest]
fn mkdir(mounted_fs: utils::FuseZstdProcess) {
    Command::new("mkdir")
        .arg(mounted_fs.mount_point().join("directory"))
        .assert()
        .success();

    let zdir = mounted_fs.data_dir().join("directory");
    assert!(zdir.exists());
}

#[rstest]
fn ls(populated_mounted_fs: utils::FuseZstdProcess) {
    let mp = populated_mounted_fs.mount_point();
    Command::new("ls")
        .arg("-1")
        .arg(&mp)
        .assert()
        .success()
        .stdout(["file1.txt", "first"].join("\n") + "\n");

    Command::new("ls")
        .arg("-1")
        .arg(&mp.join("first"))
        .assert()
        .success()
        .stdout(["file1.txt", "file2.txt", "second"].join("\n") + "\n");

    Command::new("ls")
        .arg("-1")
        .arg(&mp.join("first/second"))
        .assert()
        .success()
        .stdout(["empty", "file1.txt", "file2.txt", "file3.txt", "third"].join("\n") + "\n");

    Command::new("ls")
        .arg("-1")
        .arg(&mp.join("first/second/third"))
        .assert()
        .success()
        .stdout(["file1.txt"].join("\n") + "\n");

    Command::new("ls")
        .arg("-1")
        .arg(&mp.join("first/second/empty"))
        .assert()
        .success()
        .stdout("");
}

#[rstest]
fn cat(populated_mounted_fs: utils::FuseZstdProcess) {
    let mp = populated_mounted_fs.mount_point();
    Command::new("cat")
        .arg(&mp.join("first/second/third/file1.txt"))
        .assert()
        .success()
        .stdout("1st file in third");

    Command::new("cat")
        .arg(&mp.join("first/file1.txt"))
        .assert()
        .success()
        .stdout("1st file in first");

    Command::new("cat")
        .arg(&mp.join("file1.txt"))
        .assert()
        .success()
        .stdout("1st file in root");
}

#[rstest]
fn tee(populated_mounted_fs: utils::FuseZstdProcess) {
    let mp = populated_mounted_fs.mount_point();
    // new file
    Command::new("tee")
        .arg(&mp.join("first/second/file-new.txt"))
        .write_stdin("new file content")
        .assert()
        .success()
        .stdout("new file content");

    let dd = populated_mounted_fs.data_dir();
    // Make sure that all the changes are written
    // Target directory needs to be synced due to inode update
    fs::File::open(dd.join("first/second/"))
        .unwrap()
        .sync_all()
        .unwrap();

    let zfile = dd.join("first/second/file-new.txt.zst");
    assert_eq!(
        String::from_utf8(decode_all(fs::File::open(zfile).unwrap()).unwrap()).unwrap(),
        "new file content"
    );
}
