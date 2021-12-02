# How does it work

This project simply implements required libfuse API calls.

Currently it doesn't use any background threads to process fuse
requests so it is not very fast. This means that this implementation
can process only one request at a time.

## inode storage
Most of fuse api calls containt inode number. 
However fuse-zstd doesn't containt any persistent storage for inodes. 
It only caches the inodes number which were found in the source folder.
Cache is implemented using in-memory lru cache or via disk KV storage
in tmp folder.

Cache record looks like this:
inode (u64) -> path-to-source-folder

## uncompressed file size
The files in source folder should be compressed and have .zst extension.
Otherwise they are ignored (or converted in convert mode).

There is a problem that the mounted folder should be transparent and display
the uncompressed size of its files. Otherwise e.g. only first x bytes of compressed
size may be read (depending on read implementation).

So uncompressed file size needs to be stored somewhere, otherwise files in mounted folder
may appear shorter. (Note full data can be still obtained from compressed files in the source folder).

To store the uncompressed size the extended file attributes (xattr) are used.
So each compressed files in original folder should contain xattr with its uncompressed size.
Note that looking up for the real size slows down some operations.

## opened files and consistency
When a file is opened. Compressed file in the source folder is decompressed as a tmp file.
The handle of this file is stored while it remains opened.

When the tmp file is closed a compression is performed, source file is overriden by the new
compressed file and xattr with the real file size is set.

Note that swapping of the old compressed file and new compressed file should be atomic (rename).
However the inode number of the file changes.

## convert mode
Works in the same way as a normal mode, but in lookup when the file is not found it tries to search for
`filename` instead of `filename.zst` in the source folder and if it succeeds it tries to compress it,
store it and remove uncompressed file.
