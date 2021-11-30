use assert_cmd::cargo::cargo_bin;
use proc_mounts::MountIter;
use std::{
    fs,
    path::{Path, PathBuf},
    process, thread,
    time::Duration,
};
use tempfile::TempDir;
use zstd::decode_all;

pub fn sync_file<P>(path: P)
where
    P: AsRef<Path>,
{
    fs::File::open(path).unwrap().sync_all().unwrap();
}

pub fn get_compressed_content<P>(path: P) -> String
where
    P: AsRef<Path>,
{
    String::from_utf8(decode_all(fs::File::open(path).unwrap()).unwrap()).unwrap()
}

pub struct FuseZstdProcess {
    process: process::Child,
    data_dir: TempDir,
    mount_point: TempDir,
}

impl FuseZstdProcess {
    pub fn new(convert: bool) -> Self {
        let data_dir = TempDir::new_in("/tmp/").unwrap();
        let mount_point = TempDir::new_in("/tmp/").unwrap();
        let process = process::Command::new(cargo_bin("fuse-zstd"))
            .args(["--data-dir", data_dir.path().to_str().unwrap()])
            .args(["--mount-point", mount_point.path().to_str().unwrap()])
            .args(if convert { vec!["--convert"] } else { vec![] })
            .arg("-vvv")
            .spawn()
            .unwrap();

        // wait till mounted
        for _ in 0..50 {
            if FuseZstdProcess::check_mounted(mount_point.path()) {
                return Self {
                    process,
                    data_dir,
                    mount_point,
                };
            }
            thread::sleep(Duration::from_millis(200));
        }
        panic!("Not mounted");
    }

    fn check_mounted(mount_point: &Path) -> bool {
        MountIter::new()
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|mp| &mp.dest == mount_point)
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.path().to_path_buf()
    }

    pub fn mount_point(&self) -> PathBuf {
        self.mount_point.path().to_path_buf()
    }
}

impl Drop for FuseZstdProcess {
    fn drop(&mut self) {
        let _ = self.process.kill();
    }
}
