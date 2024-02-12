mod cache;
mod errors;
mod file;

use clap::{crate_authors, crate_name, crate_version, Arg, ArgAction, Command};
use errors::convert_io_error;
use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    Request, FUSE_ROOT_ID,
};
use log::{debug, info, warn, LevelFilter};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, Seek, SeekFrom},
    os::{
        linux::fs::MetadataExt,
        unix::fs::{FileExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};
use xattr::FileExt as XattrFileExt;

pub const TTL: Duration = Duration::from_secs(1); // dcache lifetime

type Inode = u64;

struct FileAttrWrapper {
    file_attr: FileAttr,
}

impl From<FileAttrWrapper> for FileAttr {
    fn from(faw: FileAttrWrapper) -> Self {
        faw.file_attr
    }
}

impl FileAttrWrapper {
    fn update_realsize(&mut self, file: &File) -> Result<(), libc::c_int> {
        self.file_attr.size = file
            .get_xattr("user.real_size")
            .map_err(convert_io_error)?
            .map(|e| u64::from_be_bytes(e.to_vec().try_into().unwrap()))
            .unwrap_or(0);
        Ok(())
    }
}

fn convert_ft(ft: fs::FileType) -> io::Result<fuser::FileType> {
    match ft {
        e if e.is_dir() => Ok(fuser::FileType::Directory),
        e if e.is_file() => Ok(fuser::FileType::RegularFile),
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unsupported filetype",
        )),
    }
}

fn access_all(fa: &mut FileAttr) {
    match fa.kind {
        FileType::Directory => {
            fa.perm = 0o777;
        }
        FileType::RegularFile => {
            fa.perm = 0o666;
        }
        _ => {}
    }
}

impl TryFrom<fs::DirEntry> for FileAttrWrapper {
    type Error = io::Error;
    fn try_from(dir_entry: fs::DirEntry) -> Result<Self, Self::Error> {
        let metadata = dir_entry.metadata()?;
        metadata.try_into()
    }
}

impl TryFrom<fs::Metadata> for FileAttrWrapper {
    type Error = io::Error;
    fn try_from(metadata: fs::Metadata) -> Result<Self, Self::Error> {
        Ok(Self {
            file_attr: FileAttr {
                ino: metadata.st_ino(),
                size: metadata.st_size(),
                blocks: metadata.st_blocks(),
                atime: UNIX_EPOCH + Duration::from_secs(metadata.st_atime() as u64),
                ctime: UNIX_EPOCH + Duration::from_secs(metadata.st_ctime() as u64),
                mtime: UNIX_EPOCH + Duration::from_secs(metadata.st_mtime() as u64),
                crtime: UNIX_EPOCH + Duration::from_secs(metadata.st_ctime() as u64), // creation time on macos
                kind: convert_ft(metadata.file_type())?,
                perm: metadata.permissions().mode() as u16,
                nlink: metadata.st_nlink() as u32,
                uid: metadata.st_uid(),
                gid: metadata.st_gid(),
                rdev: metadata.st_rdev() as u32,
                flags: 0, // macos only
                blksize: metadata.st_blksize() as u32,
            },
        })
    }
}

struct ZstdFS {
    compression_level: u8,
    data_dir: PathBuf,
    opened_files: file::OpenedFiles,
    inode_idx: u64,
    inode_cache: Option<cache::InodeCache>,
    /// Convert uncompressed data from original directory
    /// to compressed files
    convert: bool,
}

impl ZstdFS {
    fn new(
        data_dir: String,
        compression_level: u8,
        convert: bool,
        inode_idx: u64,
    ) -> io::Result<ZstdFS> {
        Ok(Self {
            compression_level,
            inode_cache: None,
            data_dir: data_dir.into(),
            opened_files: file::OpenedFiles::new(),
            convert,
            inode_idx,
        })
    }

    fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn cache_path(&self) -> PathBuf {
        self.data_dir().join(".fuse-zstd-inode_cache")
    }

    #[inline]
    fn icache(&mut self) -> &mut cache::InodeCache {
        self.inode_cache.as_mut().unwrap()
    }

    fn get_path(&mut self, ino: Inode) -> Result<PathBuf, libc::c_int> {
        if ino == FUSE_ROOT_ID {
            Ok(self.data_dir().clone())
        } else {
            if let Ok(path) = self.icache().get_inode_path(ino) {
                return Ok(Path::new(&path).to_path_buf());
            }

            // Try to search through opened file descriptios
            if let Some(fhs) = self
                .opened_files
                .get_fhs_from_mount_point_inode(ino)
                .map(|e| e.to_owned())
            {
                for fh in fhs {
                    if let Some(handler) = self.opened_files.get(fh) {
                        if let Some(refs) = handler.refs.as_ref() {
                            return Ok(refs.path.clone());
                        }
                    }
                }
            }

            Err(libc::ENOENT)
        }
    }

    fn sync_to_fs(&mut self, fh: u64, close: bool, force_sync: bool) -> Result<(), libc::c_int> {
        let (refs, needs_sync, file) = if close {
            let fh = self.opened_files.close(fh).ok_or(libc::EBADF)?;
            (
                fh.refs.clone(),
                fh.needs_sync,
                fh.file.try_clone().map_err(convert_io_error)?,
            )
        } else {
            let fh = self.opened_files.get(fh).ok_or(libc::ENOENT)?;
            (
                fh.refs.clone(),
                fh.needs_sync,
                fh.file.try_clone().map_err(convert_io_error)?,
            )
        };

        if needs_sync || force_sync {
            if let Some(refs) = refs {
                let source_path = refs.path;
                let dir_path = source_path.parent().unwrap().to_path_buf();

                self.store_to_source_file(
                    &file,
                    &dir_path,
                    source_path.file_name().unwrap(),
                    self.compression_level,
                )?;

                // update needs_update because the file was synced
                if !close {
                    let fh = self.opened_files.get_mut(fh).unwrap();
                    fh.needs_sync = false;
                } else {
                }
            }
        }

        Ok(())
    }

    fn lookup_wrapper(&mut self, parent: u64, name: &OsStr) -> Result<FileAttr, libc::c_int> {
        let path = self.get_path(parent)?;
        let entries = fs::read_dir(&path).map_err(convert_io_error)?;
        let name = name.to_string_lossy().to_string();
        let cache_path = self.cache_path();

        for entry in entries {
            let entry = entry.map_err(convert_io_error)?;

            // add prefix .zstd for regular files
            let filename = if entry.file_type().map_err(convert_io_error)?.is_file() {
                format!("{}.zst", &name)
            } else {
                name.clone()
            };

            // skip cache_dir from root
            if parent == FUSE_ROOT_ID && cache_path == path.join(entry.file_name()) {
                continue;
            }

            if entry.file_name().to_string_lossy() == filename {
                // Try to check the cache first
                let mut faw = FileAttrWrapper::try_from(entry).map_err(convert_io_error)?;
                // Update size from extended attributes
                let file = fs::File::open(path.join(&filename)).map_err(convert_io_error)?;
                faw.update_realsize(&file)?;
                let ino = self.update_inode(&file).map_err(convert_io_error)?;
                // Touch cache
                self.icache().set_inode_path(ino, &path, filename)?;

                let mut attrs: FileAttr = faw.into();
                // allow access to all
                access_all(&mut attrs);

                // cleanup uncompressed files in convert move
                if self.convert && attrs.kind == FileType::RegularFile {
                    let _ = fs::remove_file(path.join(&name));
                }

                // Update ino mp inodes
                attrs.ino = ino;

                return Ok(attrs);
            }
        }

        if self.convert && !name.ends_with(".zst") {
            // Uncompressed file may exist lets try to find it and compress it
            //
            // note that in convert mode every only files without .zst extension
            // can be converted
            let entries = fs::read_dir(&path).map_err(convert_io_error)?;
            for entry in entries {
                let entry = entry.map_err(convert_io_error)?;
                if entry.file_name().to_string_lossy() == name
                    && entry.file_type().map_err(convert_io_error)?.is_file()
                {
                    let zname = format!("{}.zst", &name);
                    let source_file = fs::File::open(path.join(&name)).map_err(convert_io_error)?;
                    let (file, ino) = self.store_to_source_file(
                        &source_file,
                        &path,
                        &zname,
                        self.compression_level,
                    )?;

                    // File was copied now we can remove the original
                    let _ = fs::remove_file(path.join(&name));

                    let mut faw = FileAttrWrapper::try_from(
                        source_file.metadata().map_err(convert_io_error)?,
                    )
                    .map_err(convert_io_error)?;
                    faw.update_realsize(&file)?;

                    // Touch cache
                    self.icache().set_inode_path(ino, path, zname)?;

                    let mut attrs: FileAttr = faw.into();
                    // allow access to all
                    access_all(&mut attrs);

                    attrs.ino = ino;

                    return Ok(attrs);
                }
            }
        }
        Err(libc::ENOENT)
    }

    fn readdir_wrapper(
        &mut self,
        ino: u64,
        _fh: u64,
        offset: i64,
        reply: &mut ReplyDirectory,
    ) -> Result<(), libc::c_int> {
        let file_path = self.get_path(ino)?;
        let cache_path = self.cache_path();
        let metadata = fs::metadata(&file_path).map_err(convert_io_error)?;
        if !metadata.is_dir() {
            return Err(libc::ENOTDIR);
        }

        let entries = fs::read_dir(&file_path).map_err(convert_io_error)?;

        for (i, entry) in entries.skip(offset as usize).enumerate() {
            let entry = entry.map_err(convert_io_error)?;

            let file_type = convert_ft(entry.file_type().map_err(convert_io_error)?)
                .map_err(convert_io_error)?;

            let orig_file_name = entry.file_name().to_string_lossy().to_string();

            // skip cache_dir from root
            if ino == FUSE_ROOT_ID && cache_path == file_path.join(&orig_file_name) {
                continue;
            }

            let file_name = match file_type {
                FileType::RegularFile => {
                    if !orig_file_name.ends_with(".zst") {
                        if !self.convert {
                            // Hide non-zstd file in non converting mode
                            continue;
                        } else {
                            orig_file_name.clone()
                        }
                    } else {
                        orig_file_name.strip_suffix(".zst").unwrap().to_string()
                    }
                }
                FileType::Directory => orig_file_name.clone(),
                _ => {
                    // skip other types
                    continue;
                }
            };

            // read ino from extended attributes
            let entry_path = file_path.join(&orig_file_name);
            let entry_ino_opt = xattr::get(&entry_path, "user.ino")
                .map_err(convert_io_error)?
                .map(|e| u64::from_be_bytes(e.try_into().unwrap()));
            let entry_ino = if let Some(ino) = entry_ino_opt {
                // Ino exists
                ino
            } else {
                // Make new inode
                let ino = self.update_inode_idx().map_err(convert_io_error)?;
                xattr::set(entry_path, "user.ino", &ino.to_be_bytes()).map_err(convert_io_error)?;
                ino
            };

            // update the cache
            self.icache()
                .set_inode_path(entry_ino, &file_path, orig_file_name)?;

            debug!(
                "Entry 0x{:016x}, {}, {:?}, {:?}",
                entry_ino,
                offset + i as i64 + 1,
                &file_type,
                &file_name,
            );
            if reply.add(entry_ino, offset + i as i64 + 1, file_type, file_name) {
                break;
            }
        }
        Ok(())
    }

    fn getattr_wrapper(&mut self, ino: u64) -> Result<FileAttr, libc::c_int> {
        let file_path = self.get_path(ino)?;
        let file = fs::File::open(file_path).map_err(convert_io_error)?;
        let metadata = file.metadata().map_err(convert_io_error)?;
        let mut faw: FileAttrWrapper = metadata.try_into().map_err(convert_io_error)?;
        // Update size from ext attr
        faw.update_realsize(&file)?;
        let mut attrs: FileAttr = faw.into();

        // Allow access to all
        access_all(&mut attrs);

        // override to mp ino
        attrs.ino = ino;

        Ok(attrs)
    }

    #[allow(clippy::too_many_arguments)]
    fn setattr_wrapper(
        &mut self,
        ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<std::time::SystemTime>,
        fh: Option<u64>,
        _crtime: Option<std::time::SystemTime>,
        _chgtime: Option<std::time::SystemTime>,
        _bkuptime: Option<std::time::SystemTime>,
        _flags: Option<u32>,
    ) -> Result<FileAttr, libc::c_int> {
        // TODO allow setting other arguments

        // Truncate if required
        if let Some(size) = size {
            if let Some(fh) = fh {
                if let Some(file_handler) = self.opened_files.get(fh) {
                    file_handler.file.set_len(size).map_err(convert_io_error)?;
                }
            }

            if let Some(fhs) = self.opened_files.get_fhs_from_mount_point_inode(ino) {
                fhs.to_owned()
                    .into_iter()
                    .filter_map(|fh| {
                        if let Some(file_handler) = self.opened_files.get_mut(fh) {
                            Some(file_handler.file.set_len(size))
                        } else {
                            None
                        }
                    })
                    .collect::<io::Result<Vec<_>>>()
                    .map_err(convert_io_error)?;
            }
        }
        self.getattr_wrapper(ino)
    }

    fn open_wrapper(&mut self, ino: u64, flags: i32) -> Result<u64, libc::c_int> {
        // Already opened by some other process
        if let Some(fh) = self
            .opened_files
            .duplicate(ino, flags)
            .map_err(convert_io_error)?
        {
            return Ok(fh);
        }
        let file_path = self.get_path(ino)?;
        let source_file = fs::File::open(&file_path).map_err(convert_io_error)?;
        let mut target_file = tempfile::tempfile().map_err(convert_io_error)?;
        zstd::stream::copy_decode(
            source_file.try_clone().map_err(convert_io_error)?,
            target_file.try_clone().map_err(convert_io_error)?,
        )
        .map_err(|_| libc::EFAULT)?;
        target_file
            .seek(SeekFrom::Start(0))
            .map_err(convert_io_error)?;

        // update real file size to xattr of original file
        source_file
            .set_xattr(
                "user.real_size",
                &target_file
                    .metadata()
                    .map_err(convert_io_error)?
                    .st_size()
                    .to_be_bytes(),
            )
            .map_err(convert_io_error)?;
        // Make sure that new size is written to original directory
        source_file.sync_all().map_err(convert_io_error)?;

        // Store info about newly opened file
        let fh = self
            .opened_files
            .insert(ino, flags, target_file, file_path)
            .ok_or(libc::EBUSY)?;

        Ok(fh)
    }

    fn read_wrapper(
        &mut self,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
    ) -> Result<Vec<u8>, libc::c_int> {
        // Hit the cache
        let _ = self.get_path(ino);

        let file_handler = self.opened_files.get_mut(fh).ok_or(libc::ENOENT)?;
        let mut res = vec![0; size as usize];
        let read_size = file_handler
            .file
            .read_at(&mut res, offset as u64)
            .map_err(convert_io_error)?;
        res.truncate(read_size);
        Ok(res)
    }

    fn create_wrapper(
        &mut self,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        flags: i32,
    ) -> Result<(FileAttr, u64), libc::c_int> {
        // Create emtpy file in the tree dir
        let name = name.to_string_lossy().to_string() + ".zst";
        let parent_path = self.get_path(parent)?;

        let opened_file = tempfile::tempfile().map_err(convert_io_error)?;

        // Write new file to source directory
        let (source_file, ino) =
            self.store_to_source_file(&opened_file, &parent_path, &name, self.compression_level)?;

        // Obtain attrs of the new file
        let faw = FileAttrWrapper::try_from(source_file.metadata().map_err(convert_io_error)?)
            .map_err(convert_io_error)?;
        let mut attrs: FileAttr = faw.into();

        // allow access to all
        access_all(&mut attrs);
        // user.ino has to be se in store_to_source_file()
        // so we need to read it here
        attrs.ino = ino;

        // add inode to map
        self.icache()
            .set_inode_path(attrs.ino, &parent_path, &name)?;

        // New file handler
        let fh = self
            .opened_files
            .insert(attrs.ino, flags, opened_file, parent_path.join(&name))
            .ok_or(libc::EBUSY)?;

        Ok((attrs, fh as u64))
    }

    #[allow(clippy::too_many_arguments)]
    fn write_wrapper(
        &mut self,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
    ) -> Result<usize, libc::c_int> {
        // Hit the cache
        let _ = self.get_path(ino);

        let file_handler = self.opened_files.get_mut(fh).ok_or(libc::EBADF)?;

        // File should be synced to source dir
        file_handler.needs_sync = true;

        let offset = if file_handler.flags & libc::O_APPEND != 0 {
            // We need to append to file -> we need to get end position
            file_handler
                .file
                .seek(SeekFrom::Start(offset as u64))
                .map_err(convert_io_error)?;
            file_handler
                .file
                .seek(SeekFrom::End(0))
                .map_err(convert_io_error)?
        } else {
            offset as u64
        };
        file_handler
            .file
            .write_at(data, offset)
            .map_err(convert_io_error)
    }

    fn release_wrapper(&mut self, _ino: u64, fh: u64) -> Result<(), libc::c_int> {
        // file will be closed and freed once this function ends
        self.sync_to_fs(fh, true, false)?;
        Ok(())
    }

    fn mkdir_wrapper(
        &mut self,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
    ) -> Result<FileAttr, libc::c_int> {
        let parent_path = self.get_path(parent)?;
        let path = parent_path.join(name);
        fs::create_dir(&path).map_err(convert_io_error)?;
        let metadata = fs::metadata(&path).map_err(convert_io_error)?;

        let faw: FileAttrWrapper = metadata.try_into().map_err(convert_io_error)?;
        let mut attrs: FileAttr = faw.into();
        // allow access to all
        access_all(&mut attrs);
        attrs.ino = self.update_inode_idx().map_err(convert_io_error)?;

        // store ino
        xattr::set(path, "user.ino", &attrs.ino.to_be_bytes()).map_err(convert_io_error)?;

        // update inode map
        self.icache()
            .set_inode_path(attrs.ino, parent_path, name.to_string_lossy())?;

        Ok(attrs)
    }

    fn unlink_wrapper(&mut self, parent: u64, name: &OsStr) -> Result<(), libc::c_int> {
        let parent_path = self.get_path(parent)?;
        let path = parent_path.join(name.to_string_lossy().to_string() + ".zst");
        if let Some(ino_data) = xattr::get(&path, "user.ino").map_err(convert_io_error)? {
            let ino = u64::from_be_bytes(ino_data.try_into().unwrap());
            self.icache().del_inode_path(ino)?;
            self.opened_files.unlink(ino);
        }
        fs::remove_file(path).map_err(convert_io_error)?;
        Ok(())
    }

    fn rmdir_wrapper(&mut self, parent: u64, name: &OsStr) -> Result<(), libc::c_int> {
        let parent_path = self.get_path(parent)?;
        let path = parent_path.join(name.to_string_lossy().to_string());
        if let Some(ino_data) = xattr::get(&path, "user.ino").map_err(convert_io_error)? {
            let ino = u64::from_be_bytes(ino_data.try_into().unwrap());
            self.icache().del_inode_path(ino)?;
            self.opened_files.unlink(ino);
        }

        // Don't try to remove cache dir
        let cache_dir = self.cache_path();
        if cache_dir == parent_path.join(name) {
            warn!("Cache dir '{}' can't be removed!", cache_dir.display());
            return Err(libc::ENOENT);
        }

        fs::remove_dir(path).map_err(convert_io_error)?;
        Ok(())
    }

    fn rename_wrapper(
        &mut self,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
    ) -> Result<(), libc::c_int> {
        // First we should check filetype of source file
        // and add .zst extension to both names
        let (name, newname, ino) = {
            let attrs = self.lookup_wrapper(parent, name)?;
            if matches!(attrs.kind, FileType::RegularFile) {
                (
                    format!("{}.zst", name.to_string_lossy()),
                    format!("{}.zst", newname.to_string_lossy()),
                    attrs.ino,
                )
            } else {
                (
                    name.to_string_lossy().to_string(),
                    newname.to_string_lossy().to_string(),
                    attrs.ino,
                )
            }
        };

        let from_path = self.get_path(parent)?.join(name);

        let to_parent_path = self.get_path(newparent)?;
        let to_path = to_parent_path.join(&newname);

        if let Some(orig_ino) = fs::metadata(&to_path).ok().map(|e| e.st_ino()) {
            self.icache().del_inode_path(orig_ino)?;
            self.opened_files.unlink(orig_ino);
        }

        fs::rename(from_path, &to_path).map_err(convert_io_error)?;

        // Update inode mapping
        self.icache().set_inode_path(ino, to_parent_path, newname)?;

        // TODO update opened files to match path
        // without update the opened files will be written to old location

        Ok(())
    }

    fn fsync_wrapper(&mut self, _ino: u64, fh: u64, _datasync: bool) -> Result<(), libc::c_int> {
        self.sync_to_fs(fh, false, true)?;
        Ok(())
    }

    fn flush_wrapper(&mut self, _ino: u64, fh: u64, _lock_owner: u64) -> Result<(), libc::c_int> {
        self.sync_to_fs(fh, false, false)?;
        Ok(())
    }

    fn update_inode_idx(&mut self) -> io::Result<u64> {
        let res = self.inode_idx;

        if self.inode_idx - 1 <= FUSE_ROOT_ID {
            log::warn!("Fuse inodes counter rotated");
            // Note that this should never happen
            // fuse-zstd should be running for ceturies to achieve this...
            self.inode_idx = u64::MAX;
        }
        self.inode_idx -= 1;

        debug!(
            "Updating 'ino_idx' at root {} to 0x{:016x}",
            self.data_dir().display(),
            self.inode_idx
        );
        xattr::set(
            &self.data_dir,
            "user.ino_idx",
            &self.inode_idx.to_be_bytes(),
        )?;

        Ok(res)
    }

    fn update_inode(&mut self, f: &fs::File) -> io::Result<Inode>
where {
        if let Some(data) = f.get_xattr("user.ino")? {
            Ok(u64::from_be_bytes(data.try_into().unwrap()))
        } else {
            let ino = self.update_inode_idx()?;
            f.set_xattr("user.ino", &ino.to_be_bytes())?;
            Ok(ino)
        }
    }

    fn store_to_source_file<P1, P2>(
        &mut self,
        source: &fs::File,
        dir_path: P1,
        name: P2,
        compression_level: u8,
    ) -> Result<(fs::File, u64), libc::c_int>
    where
        P1: AsRef<Path>,
        P2: AsRef<Path>,
    {
        // Atomically creates file in source directory
        let tmp_file =
            tempfile::NamedTempFile::new_in(dir_path.as_ref()).map_err(convert_io_error)?;
        let path = dir_path.as_ref().join(name.as_ref());

        source.sync_all().map_err(convert_io_error)?;

        let real_size = source.metadata().map_err(convert_io_error)?.st_size();
        debug!("Before compression {}", real_size);

        let mut cloned_source = source.try_clone().map_err(convert_io_error)?;
        cloned_source
            .seek(SeekFrom::Start(0))
            .map_err(convert_io_error)?;
        // Compress file
        let mut encoder = zstd::stream::Encoder::new(
            tmp_file.reopen().map_err(convert_io_error)?,
            compression_level as i32,
        )
        .map_err(convert_io_error)?;
        encoder
            .set_pledged_src_size(Some(real_size))
            .map_err(convert_io_error)?;
        encoder.include_checksum(true).map_err(convert_io_error)?;
        io::copy(&mut cloned_source, &mut encoder).map_err(convert_io_error)?;
        encoder.finish().map_err(convert_io_error)?;

        // Try to update the ino of tmp file
        let ino = match xattr::get(&path, "user.ino") {
            Ok(Some(ino_data)) => {
                tmp_file
                    .as_file()
                    .set_xattr("user.ino", &ino_data)
                    .map_err(convert_io_error)?;

                u64::from_be_bytes(ino_data.try_into().unwrap())
            }
            _ => {
                // Error or None -> create new ino
                let new_ino = self.update_inode_idx().map_err(convert_io_error)?;
                tmp_file
                    .as_file()
                    .set_xattr("user.ino", &new_ino.to_be_bytes())
                    .map_err(convert_io_error)?;

                tmp_file.as_file().sync_all().map_err(convert_io_error)?;

                new_ino
            }
        };

        // Should atomically move file to its destination
        let file = tmp_file.persist(&path).map_err(convert_io_error)?;

        // update filesize in xattrs
        file.set_xattr("user.real_size", &real_size.to_be_bytes())
            .map_err(convert_io_error)?;

        // Sync it
        file.sync_all().map_err(convert_io_error)?;
        debug!(
            "After compression {}",
            file.metadata().map_err(convert_io_error)?.st_size()
        );

        Ok((file, ino))
    }
}

impl Filesystem for ZstdFS {
    fn init(
        &mut self,
        _req: &Request<'_>,
        _config: &mut fuser::KernelConfig,
    ) -> Result<(), libc::c_int> {
        fs::create_dir_all(Path::new(&self.data_dir())).map_err(convert_io_error)?;

        let cache_root = self.cache_path();
        if fs::remove_dir_all(&cache_root)
            .map_err(convert_io_error)
            .is_ok()
        {
            debug!("Clearing root cache directory {}", cache_root.display());
        }
        debug!("Creating cache root directory {}", cache_root.display());
        fs::create_dir_all(&cache_root).map_err(convert_io_error)?;

        let cache = cache::InodeCache::new(&cache_root)?;
        let cache_path = cache.cache_data_dir().path().display();

        debug!("Initializing inode cache at '{}'", cache_path);

        self.inode_cache = Some(cache);

        Ok(())
    }

    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        debug!(
            "Lookup (iparent=0x{:016x}, name='{}')",
            parent,
            name.to_str().unwrap_or_default()
        );
        match self.lookup_wrapper(parent, name) {
            Ok(attrs) => {
                debug!("Lookup OK (inode=0x{:016x})", attrs.ino);
                reply.entry(&TTL, &attrs, 0);
            }
            Err(err) => {
                debug!("Lookup Err (code={})", err);
                reply.error(err);
            }
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        debug!("Getattr (inode=0x{:016x})", ino);
        match self.getattr_wrapper(ino) {
            Ok(attrs) => {
                debug!("getattr ok");
                reply.attr(&TTL, &attrs);
            }
            Err(err) => {
                debug!("getattr error({})", err);
                reply.error(err)
            }
        }
    }

    fn setattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<fuser::TimeOrNow>,
        mtime: Option<fuser::TimeOrNow>,
        ctime: Option<std::time::SystemTime>,
        fh: Option<u64>,
        crtime: Option<std::time::SystemTime>,
        chgtime: Option<std::time::SystemTime>,
        bkuptime: Option<std::time::SystemTime>,
        flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        debug!(
            "Setattr (inode=0x{:016x}, fh={:?}, mode={:?}, uid={:?}, gid={:?}, ...)",
            ino, fh, mode, uid, gid,
        );
        match self.setattr_wrapper(
            ino, mode, uid, gid, size, atime, mtime, ctime, fh, crtime, chgtime, bkuptime, flags,
        ) {
            Ok(attrs) => {
                debug!("setattr ok");
                reply.attr(&TTL, &attrs);
            }
            Err(err) => {
                debug!("setattr error({})", err);
                reply.error(err)
            }
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        debug!(
            "Read (inode=0x{:016x}, offset={}, size={}, fh={})",
            ino, offset, size, fh
        );
        match self.read_wrapper(ino, fh, offset, size) {
            Ok(data) => {
                debug!("read {}", data.len());
                reply.data(&data);
            }
            Err(err) => {
                debug!("read error({})", err);
                reply.error(err);
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!(
            "Readdir (inode=0x{:016x}, offset={}, fh={})",
            ino, offset, fh
        );
        match self.readdir_wrapper(ino, fh, offset, &mut reply) {
            Ok(_) => {
                reply.ok();
            }
            Err(err) => {
                reply.error(err);
            }
        }
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: fuser::ReplyOpen) {
        debug!("Open (inode=0x{:016x}, flags={:x})", ino, flags);
        match self.open_wrapper(ino, flags) {
            Ok(fh) => {
                debug!("opened (fh={})", fh);
                reply.opened(fh, 0);
            }
            Err(err) => {
                debug!("open error (err={})", err);
                reply.error(err);
            }
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        debug!("Release (inode=0x{:016x}, fh={})", ino, fh);
        match self.release_wrapper(ino, fh) {
            Ok(()) => {
                debug!("released");
                reply.ok();
            }
            Err(libc::EBADF) => {
                debug!("Already released (inode=0x{:016x}, fh={})", ino, fh);
                reply.ok();
            }
            Err(err) => {
                warn!("Release error (inode=0x{:016x}, fh={})", ino, fh);
                reply.error(err);
            }
        }
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        reply: fuser::ReplyCreate,
    ) {
        debug!(
            "Create (iparent=0x{:016x}, name={:?}, mode={:o}, umask={:o}, flags={:x})",
            parent, name, mode, umask, flags
        );
        match self.create_wrapper(parent, name, mode, umask, flags) {
            Ok((attrs, fh)) => {
                debug!("created (inode=0x{:016x}, fh={})", attrs.ino, fh);
                reply.created(&TTL, &attrs, 0, fh, flags as u32);
            }
            Err(err) => {
                debug!("create failed (err={})", err);
                reply.error(err);
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        write_flags: u32,
        flags: i32,
        lock_owner: Option<u64>,
        reply: fuser::ReplyWrite,
    ) {
        debug!(
            "Write (ino=0x{:016x}, fh={}, offset={}, data_len={}, write_flags={:x}, flags={:x}), lock={:?}",
            ino, fh, offset, data.len(), write_flags, flags, lock_owner
        );
        match self.write_wrapper(ino, fh, offset, data, write_flags, flags, lock_owner) {
            Ok(size) => {
                debug!("written (size={})", size);
                reply.written(size as u32);
            }
            Err(err) => {
                reply.error(err);
            }
        }
    }

    fn mkdir(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        debug!(
            "Mkdir (iparent=0x{:016x}, name={:?}, mode={:o}, umask={:o})",
            parent, name, mode, umask
        );
        match self.mkdir_wrapper(parent, name, mode, umask) {
            Ok(attrs) => {
                debug!("mkdir passed (ino=0x{:016x})", attrs.ino);
                reply.entry(&TTL, &attrs, 0);
            }
            Err(err) => {
                debug!("mkdir failed (err={})", err);
                reply.error(err);
            }
        }
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        debug!("Unlink (iparent=0x{:016x}, name={:?})", parent, name,);
        match self.unlink_wrapper(parent, name) {
            Ok(()) => {
                debug!("unlink passed");
                reply.ok();
            }
            Err(err) => {
                debug!("unlink failed (err={})", err);
                reply.error(err);
            }
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        debug!("Rmdir (iparent=0x{:016x}, name={:?})", parent, name,);
        match self.rmdir_wrapper(parent, name) {
            Ok(()) => {
                debug!("rmdir passed");
                reply.ok();
            }
            Err(err) => {
                debug!("rmdir failed (err={})", err);
                reply.error(err);
            }
        }
    }

    fn rename(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        flags: u32,
        reply: fuser::ReplyEmpty,
    ) {
        debug!(
            "Rename (from_iparent=0x{:016x}, from_name={:?}, to_iparent=0x{:016x}, to_iname={:?}, flags={:x})",
            parent, name, newparent, newname, flags
        );
        match self.rename_wrapper(parent, name, newparent, newname, flags) {
            Ok(()) => {
                debug!("rename passed");
                reply.ok();
            }
            Err(err) => {
                debug!("rename failed (err={})", err);
                reply.error(err);
            }
        }
    }

    fn fsync(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        datasync: bool,
        reply: fuser::ReplyEmpty,
    ) {
        debug!(
            "Fsync (ino=0x{:016x}, fh={:?}, datasync={:?})",
            ino, fh, datasync
        );
        match self.fsync_wrapper(ino, fh, datasync) {
            Ok(()) => {
                debug!("fsync passed");
                reply.ok();
            }
            Err(err) => {
                debug!("fsync failed (err={})", err);
                reply.error(err);
            }
        }
    }

    fn flush(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        lock_owner: u64,
        reply: fuser::ReplyEmpty,
    ) {
        debug!(
            "Flush (ino=0x{:016x}, fh={:?}, lock_owner={:?}",
            ino, fh, lock_owner
        );
        match self.flush_wrapper(ino, fh, lock_owner) {
            Ok(()) => {
                debug!("flush passed");
                reply.ok();
            }
            Err(err) => {
                debug!("flush failed (err={})", err);
                reply.error(err);
            }
        }
    }

    fn destroy(&mut self) {
        let cache_dir = self.icache().cache_data_dir().path().to_owned();
        debug!("Discarding inode cache at '{}'", cache_dir.display());
        // Should drop the cache and delete tmp directory
        self.inode_cache = None;
    }
}

fn main() -> io::Result<()> {
    let app = Command::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .arg(
            Arg::new("mount-point")
                .long("mount-point")
                .value_name("MOUNT_POINT")
                .default_value("")
                .help("Where FUSE fs shall be mounted")
                .env("FUSE_ZSTD_MOUNT_POINT")
                .num_args(1),
        )
        .arg(
            Arg::new("data-dir")
                .long("data-dir")
                .value_name("DATA_DIR")
                .default_value("/tmp/zstdfs/")
                .help("Directory from which ZSTD files will be decompressed")
                .env("FUSE_ZSTD_DATA_DIR")
                .action(ArgAction::Set)
                .num_args(1),
        )
        .arg(
            Arg::new("compression-level")
            .short('c')
            .long("compression-level")
            .value_name("LEVEL")
            .default_value("0")
            .help("Set compression level of zstd (0-19), 0 means use default value provided by library")
            .env("FUSE_ZSTD_COMPRESSION_LEVEL")
            .num_args(1)
        )
        .arg(
            Arg::new("v")
                .short('v')
                .action(ArgAction::Count)
                .help("Sets the level of verbosity"),
        )
        .arg(
            Arg::new("convert")
                .long("convert")
                .action(ArgAction::SetTrue)
                .help("Will convert files uncompressed files from data dir"),
        );

    #[cfg(feature = "with_sentry")]
    let app = app.arg(
        Arg::new("sentry-url")
            .long("sentry-url")
            .default_value("")
            .help("Sentry url where events will be sent")
            .env("FUSE_ZSTD_SENTRY_URL")
            .action(ArgAction::Set)
            .num_args(1),
    );

    let matches = app.get_matches();

    let verbosity: u8 = matches.get_count("v");
    let convert: bool = matches.get_flag("convert");
    let log_level = match verbosity {
        0 => LevelFilter::Error,
        1 => LevelFilter::Warn,
        2 => LevelFilter::Info,
        3 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    let data_dir: String = matches
        .get_one("data-dir")
        .map(String::to_owned)
        .unwrap_or_default()
        .to_string();
    let compression_level = matches
        .get_one("compression-level")
        .map(String::to_owned)
        .unwrap_or_default();
    let compression_level = compression_level.parse::<u8>().unwrap_or_else(|_| {
        warn!("Error parsing compression level. Using default.");
        0
    });
    let compression_level = if compression_level > 19 {
        warn!("Wrong compression level. Using default.");
        0
    } else {
        compression_level
    };

    #[cfg(feature = "with_sentry")]
    let _guard = if let Some(url) = matches.get_one("sentry-url").map(String::to_owned) {
        let mut log_builder = env_logger::builder();
        log_builder.filter_level(log_level);
        let logger = sentry_log::SentryLogger::with_dest(log_builder.build());

        log::set_boxed_logger(Box::new(logger)).unwrap();
        log::set_max_level(log_level);
        Some(sentry::init((
            url,
            sentry::ClientOptions {
                release: sentry::release_name!(),
                ..Default::default()
            },
        )))
    } else {
        env_logger::builder().filter_level(log_level).init();
        None
    };
    #[cfg(not(feature = "with_sentry"))]
    env_logger::builder().filter_level(log_level).init();

    let mountpoint: String = matches
        .get_one("mount-point")
        .map(String::to_owned)
        .unwrap_or_default()
        .to_string();
    let options = vec![
        MountOption::RW,
        MountOption::FSName(data_dir.clone()),
        MountOption::AutoUnmount,
        MountOption::AllowOther,
    ];
    info!(
        "Starting fuse-zstd ({}) with compression level={}, convert={}",
        crate_version!(),
        compression_level,
        convert,
    );

    // Read fuse-zstd inode index from
    let inode_idx = xattr::get(&data_dir, "user.ino_idx")?
        .map(|e| u64::from_be_bytes(e.to_vec().try_into().unwrap()))
        .unwrap_or(u64::MAX);
    debug!("Root inode index 0x{:016x}", inode_idx);

    fuser::mount2(
        ZstdFS::new(data_dir, compression_level, convert, inode_idx)?,
        mountpoint,
        &options,
    )
}
