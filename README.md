# FUSE-zstd

A simple FUSE filesystem where existing folder with files compressed by zstd
is mapped to folder with uncompressed files.

## What it does?
It simply remounts a part of existing filesystem path.
```
file.txt.zst
directory/
directory/file.txt.zst
```
to
```
file.txt
directory/
directory/file.txt
```

### Note
When you add compressed files directly to the source folder, you need to reopen them
in mounted folder to recalculate the uncompressed size (e.g. using `head` cmd),
othewise the files in mounted folder will displayed as empty.

And also be sure that all the files in source folder will contain `.zst` extenstion,
otherwise the files won't be shown in the mounted dir.


## Requirements

* fuse3
* libfuse3

## Building from sources

### Insall rust

see https://www.rust-lang.org/tools/install


### Install dev libraries

Debian:
```
apt install fuse3 libfuse3-3 libfuse3-dev
```


### Compile it
```
cargo build --release
```


### Prepare a package

#### Debian
Install cargo-deb
```
cargo install cargo-deb
```
Build the package
```
cargo deb
```


## Usage
Make sure that option `user_allow_other` is enabled in your `/etc/fuse.conf`.

Make sure that both source and mount point directories exist and have proper permissions.
```
mkdir -p /tmp/fuse-zstd/ /tmp/fuse-zstd-compressed/
```

Run it.
```
cargo run -- --data-dir /tmp/fuse-zstd-compressed/ --mount-point /tmp/fuse-zstd/
```

Now every file you create in `mount-point` dir should appear as compressed file
with zst extension in `data-dir`.


## Limitations
* Source folder has to be only from a single FS (needs to have unique inodes).
* Source folder FS has to support extended file attributes (xattr) to store uncompressed size of the files.
* Source folder has to contain only files and directories (othewise fuse-zstd may crash).


## Motivation
Although there are some filesystem which support compression not all hosting services
support such filesystems. So imagine you have an ext4 FS with thousands of large JSON files.
In this situation FUSE-Zstd can be quite handy.
