use assert_cmd::Command;
use rstest::*;
use std::fs;
use zstd::decode_all;

#[path = "utils.rs"]
mod utils;

#[fixture]
fn mounted_fs() -> utils::FuseZstdProcess {
    utils::FuseZstdProcess::new(false)
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
    let dd = populated_mounted_fs.data_dir();

    // new file
    Command::new("tee")
        .arg(&mp.join("first/second/file-new.txt"))
        .write_stdin("new file content")
        .assert()
        .success()
        .stdout("new file content");

    assert_eq!(
        fs::read_to_string(&mp.join("first/second/file-new.txt")).unwrap(),
        "new file content"
    );

    let zfile = dd.join("first/second/file-new.txt.zst");
    assert_eq!(utils::get_compressed_content(zfile), "new file content");

    // truncate
    Command::new("tee")
        .arg(&mp.join("first/file1.txt"))
        .write_stdin("truncated")
        .assert()
        .success()
        .stdout("truncated");

    assert_eq!(
        fs::read_to_string(&mp.join("first/file1.txt")).unwrap(),
        "truncated"
    );

    let zfile = dd.join("first/file1.txt.zst");
    assert_eq!(utils::get_compressed_content(zfile), "truncated");

    // append
    Command::new("tee")
        .arg("-a")
        .arg(&mp.join("first/file1.txt"))
        .write_stdin(" and appended")
        .assert()
        .success()
        .stdout(" and appended");

    assert_eq!(
        fs::read_to_string(&mp.join("first/file1.txt")).unwrap(),
        "truncated and appended"
    );

    let zfile = dd.join("first/file1.txt.zst");
    assert_eq!(
        utils::get_compressed_content(zfile),
        "truncated and appended"
    );
}

#[rstest]
fn mv(populated_mounted_fs: utils::FuseZstdProcess) {
    let mp = populated_mounted_fs.mount_point();
    let dd = populated_mounted_fs.data_dir();

    // move file within directory
    Command::new("mv")
        .arg(&mp.join("first/second/file1.txt"))
        .arg(&mp.join("first/second/fileI.txt"))
        .assert()
        .success();
    assert!(dd.join("first/second/fileI.txt.zst").exists());
    assert!(!dd.join("first/second/file1.txt.zst").exists());

    // move file to existing directory
    Command::new("mv")
        .arg(&mp.join("first/second/file2.txt"))
        .arg(&mp.join("first/file3.txt"))
        .assert()
        .success();
    assert!(!dd.join("first/second/file2.txt.zst").exists());
    assert!(dd.join("first/file3.txt.zst").exists());
    assert_eq!(
        utils::get_compressed_content(dd.join("first/file3.txt.zst")),
        "2nd file in second"
    );

    // move file to existing file
    Command::new("mv")
        .arg(&mp.join("first/file1.txt"))
        .arg(&mp.join("first/second/third/file1.txt"))
        .assert()
        .success();
    assert!(!dd.join("first/file1.txt.zst").exists());
    assert!(dd.join("first/second/third/file1.txt.zst").exists());
    assert_eq!(
        utils::get_compressed_content(dd.join("first/second/third/file1.txt.zst")),
        "1st file in first"
    );

    // move directory within directory
    Command::new("mv")
        .arg(&mp.join("first/second/empty"))
        .arg(&mp.join("first/second/void"))
        .assert()
        .success();
    assert!(!dd.join("first/second/empty").exists());
    assert!(dd.join("first/second/void").exists());

    // move directory to existing directory
    Command::new("mv")
        .arg(&mp.join("first/second"))
        .arg(&mp)
        .assert()
        .success();
    assert!(!dd.join("first/second").exists());
    assert!(dd.join("second/void").exists());

    // move directory to existing file
    Command::new("mv")
        .arg(&mp.join("second/third"))
        .arg(&mp.join("second/file3.txt"))
        .assert()
        .failure();
    assert!(dd.join("second/third").exists());
}

#[rstest]
fn rm(populated_mounted_fs: utils::FuseZstdProcess) {
    let mp = populated_mounted_fs.mount_point();
    let dd = populated_mounted_fs.data_dir();
    // existing
    Command::new("rm")
        .arg(&mp.join("first/file1.txt"))
        .assert()
        .success();
    assert!(!dd.join("first/file1.txt.zst").exists());

    // non-existing
    Command::new("rm")
        .arg(&mp.join("non-existing.txt"))
        .assert()
        .failure();
    assert!(!dd.join("non-existing.txt.zst").exists());

    // directory
    Command::new("rm").arg(&mp.join("first")).assert().failure();
    assert!(dd.join("first").exists());
}

#[rstest]
fn rmdir(populated_mounted_fs: utils::FuseZstdProcess) {
    let mp = populated_mounted_fs.mount_point();
    let dd = populated_mounted_fs.data_dir();

    // existing empty
    Command::new("rmdir")
        .arg(&mp.join("first/second/empty"))
        .assert()
        .success();
    assert!(!dd.join("first/second/empty").exists());

    // existing non-empty
    Command::new("rmdir")
        .arg(&mp.join("first/second/third"))
        .assert()
        .failure();
    assert!(dd.join("first/second/third").exists());

    // existing file
    Command::new("rmdir")
        .arg(&mp.join("first/file1.txt"))
        .assert()
        .failure();
    assert!(dd.join("first/file1.txt.zst").exists());

    // non-existing
    Command::new("rmdir")
        .arg(&mp.join("first/non-existing.txt"))
        .assert()
        .failure();
    assert!(!dd.join("first/non-existing.txt.zst").exists());
}
