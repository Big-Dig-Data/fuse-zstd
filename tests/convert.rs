use std::{fs, os::linux::fs::MetadataExt, path};
use xattr::FileExt;
use zstd::block::compress;

#[path = "utils.rs"]
pub mod utils;

pub fn make_usecases<P>(path: P)
where
    P: AsRef<path::Path>,
{
    let p = path.as_ref();
    fs::create_dir_all(p.join("directory")).unwrap();

    // Compressed, but no realsize written to xattrs
    fs::write(
        p.join("already_compressed.txt.zst"),
        compress(b"compressed data", 0).unwrap(),
    )
    .unwrap();
    fs::write(
        p.join("directory/already_compressed.txt.zst"),
        compress(b"compressed data", 0).unwrap(),
    )
    .unwrap();

    // Uncompressed data
    fs::write(p.join("uncompressed.txt"), b"compressed data").unwrap();
    fs::write(p.join("directory/uncompressed.txt"), b"compressed data").unwrap();

    // Both compressed and uncompressed present
    fs::write(p.join("overlap.txt"), b"overlap plain").unwrap();
    fs::write(
        p.join("overlap.txt.zst"),
        compress(b"overlap compressed", 0).unwrap(),
    )
    .unwrap();
    fs::write(p.join("directory/overlap.txt"), b"overlap plain").unwrap();
    fs::write(
        p.join("directory/overlap.txt.zst"),
        compress(b"overlap compressed", 0).unwrap(),
    )
    .unwrap();
}

pub fn fill_in_size_test<P1, P2>(data_dir: P1, mount_point: P2)
where
    P1: AsRef<path::Path>,
    P2: AsRef<path::Path>,
{
    let dd = data_dir.as_ref();
    let mp = mount_point.as_ref();

    // Size is not filled
    assert_eq!(
        fs::metadata(mp.join("already_compressed.txt"))
            .unwrap()
            .st_size(),
        0
    );
    assert!(!dd.join("already_compressed.txt").exists());

    // Size is filled after open
    let _file = fs::File::open(mp.join("already_compressed.txt")).unwrap();

    let sfile = fs::File::open(dd.join("already_compressed.txt.zst")).unwrap();
    assert_eq!(
        sfile
            .get_xattr("user.real_size")
            .unwrap()
            .map(|e| u64::from_be_bytes(e.to_vec().try_into().unwrap()))
            .unwrap(),
        15
    );
    assert!(!dd.join("directory/already_compressed.txt").exists());

    // Size is not filled
    assert_eq!(
        fs::metadata(mp.join("directory/already_compressed.txt"))
            .unwrap()
            .st_size(),
        0
    );
    assert!(!dd.join("directory/already_compressed.txt").exists());

    // Size is filled after open
    let _file = fs::File::open(mp.join("directory/already_compressed.txt")).unwrap();

    let sfile = fs::File::open(dd.join("directory/already_compressed.txt.zst")).unwrap();
    assert_eq!(
        sfile
            .get_xattr("user.real_size")
            .unwrap()
            .map(|e| u64::from_be_bytes(e.to_vec().try_into().unwrap()))
            .unwrap(),
        15
    );
    assert!(!dd.join("directory/already_compressed.txt").exists());
}

mod no_convert {
    use super::utils;
    use rstest::*;
    use std::{fs, mem, os::linux::fs::MetadataExt};

    #[fixture]
    fn mounted_fs() -> utils::FuseZstdProcess {
        let zstd_process = utils::FuseZstdProcess::new(false);
        super::make_usecases(zstd_process.data_dir());
        zstd_process
    }

    #[rstest]
    fn already_compressed(mounted_fs: utils::FuseZstdProcess) {
        super::fill_in_size_test(mounted_fs.data_dir(), mounted_fs.mount_point());
    }

    #[rstest]
    fn uncompressed(mounted_fs: utils::FuseZstdProcess) {
        let dd = mounted_fs.data_dir();
        let mp = mounted_fs.mount_point();

        assert!(dd.join("uncompressed.txt").exists());
        assert!(!mp.join("uncompressed.txt").exists());

        assert!(fs::File::open(mp.join("uncompressed.txt")).is_err());

        assert!(dd.join("uncompressed.txt").exists());
        assert!(!mp.join("uncompressed.txt").exists());

        assert!(dd.join("directory/uncompressed.txt").exists());
        assert!(!mp.join("directory/uncompressed.txt").exists());

        assert!(fs::File::open(mp.join("directory/uncompressed.txt")).is_err());

        assert!(dd.join("directory/uncompressed.txt").exists());
        assert!(!mp.join("directory/uncompressed.txt").exists());
    }

    #[rstest]
    fn overlap(mounted_fs: utils::FuseZstdProcess) {
        let dd = mounted_fs.data_dir();
        let mp = mounted_fs.mount_point();

        std::thread::sleep(std::time::Duration::from_secs(1));
        assert!(dd.join("overlap.txt").exists());
        assert!(dd.join("overlap.txt.zst").exists());
        assert!(mp.join("overlap.txt").exists());
        assert!(!mp.join("overlap.txt.zst").exists());

        assert!(fs::File::open(mp.join("overlap.txt")).is_ok());

        assert!(dd.join("overlap.txt").exists());
        assert!(dd.join("overlap.txt.zst").exists());
        assert!(mp.join("overlap.txt").exists());
        assert!(!mp.join("overlap.txt.zst").exists());

        assert!(dd.join("directory/overlap.txt").exists());
        assert!(dd.join("directory/overlap.txt.zst").exists());
        assert!(mp.join("directory/overlap.txt").exists());
        assert!(!mp.join("directory/overlap.txt.zst").exists());

        assert!(fs::File::open(mp.join("overlap.txt")).is_ok());

        assert!(dd.join("directory/overlap.txt").exists());
        assert!(dd.join("directory/overlap.txt.zst").exists());
        assert!(mp.join("directory/overlap.txt").exists());
        assert!(!mp.join("directory/overlap.txt.zst").exists());
    }
}

mod convert {
    use super::utils;
    use rstest::*;
    use std::{fs, os::linux::fs::MetadataExt};

    #[fixture]
    fn mounted_fs() -> utils::FuseZstdProcess {
        let zstd_process = utils::FuseZstdProcess::new(true);
        super::make_usecases(zstd_process.data_dir());
        zstd_process
    }

    #[rstest]
    fn already_compressed(mounted_fs: utils::FuseZstdProcess) {
        super::fill_in_size_test(mounted_fs.data_dir(), mounted_fs.mount_point());
    }

    #[rstest]
    fn uncompressed(mounted_fs: utils::FuseZstdProcess) {
        let dd = mounted_fs.data_dir();
        let mp = mounted_fs.mount_point();

        // in root
        assert!(dd.join("uncompressed.txt").exists());
        assert!(mp.join("uncompressed.txt").exists());

        assert!(fs::File::open(mp.join("uncompressed.txt")).is_ok());

        assert!(!dd.join("uncompressed.txt").exists());
        assert!(dd.join("uncompressed.txt.zst").exists());
        assert!(mp.join("uncompressed.txt").exists());

        // in subfolder
        assert!(dd.join("directory/uncompressed.txt").exists());
        assert!(mp.join("directory/uncompressed.txt").exists());

        assert!(fs::File::open(mp.join("directory/uncompressed.txt")).is_ok());

        assert!(!dd.join("directory/uncompressed.txt").exists());
        assert!(dd.join("directory/uncompressed.txt.zst").exists());
        assert!(mp.join("directory/uncompressed.txt").exists());
    }

    #[rstest]
    fn overlap(mounted_fs: utils::FuseZstdProcess) {
        let dd = mounted_fs.data_dir();
        let mp = mounted_fs.mount_point();

        // in root
        assert!(dd.join("overlap.txt").exists());
        assert!(dd.join("overlap.txt.zst").exists());
        assert_eq!(
            fs::read_to_string(dd.join("overlap.txt")).unwrap(),
            "overlap plain"
        );
        assert_eq!(
            utils::get_compressed_content(dd.join("overlap.txt.zst")),
            "overlap compressed"
        );
        // mp.join will cause lookup
        assert!(mp.join("overlap.txt").exists());
        assert!(!mp.join("overlap.txt.zst").exists());

        // after lookup uncompressed file should be removed
        assert!(!dd.join("overlap.txt").exists());
        assert!(dd.join("overlap.txt.zst").exists());
        assert_eq!(
            utils::get_compressed_content(dd.join("overlap.txt.zst")),
            "overlap compressed"
        );

        // in directory
        assert!(dd.join("directory/overlap.txt").exists());
        assert!(dd.join("directory/overlap.txt.zst").exists());
        assert_eq!(
            fs::read_to_string(dd.join("directory/overlap.txt")).unwrap(),
            "overlap plain"
        );
        assert_eq!(
            utils::get_compressed_content(dd.join("directory/overlap.txt.zst")),
            "overlap compressed"
        );
        // mp.join will cause lookup
        assert!(mp.join("directory/overlap.txt").exists());
        assert!(!mp.join("directory/overlap.txt.zst").exists());

        // after lookup uncompressed file should be removed
        assert!(!dd.join("directory/overlap.txt").exists());
        assert!(dd.join("directory/overlap.txt.zst").exists());
        assert_eq!(
            utils::get_compressed_content(dd.join("directory/overlap.txt.zst")),
            "overlap compressed"
        );
    }
}
