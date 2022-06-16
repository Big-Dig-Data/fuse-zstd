
# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [1.1.0] (2022-06-16)

### Changed
* using sled (persistent disk kv storage) to store inode cache


## [1.0.0] (2022-05-27)

### Changed
* persistent fuse-zstd inodes


## [0.5.2] (2022-05-26)

### Fixed
* time based inode cache expires too early


## [0.5.1] (2022-04-11)

### Fixed
* file mapping updates during opened file duplication

### Changed
* Show inodes in hexa in logs


## [0.5.0] (2022-04-08)

### Changed
* try to search file handlers when the cache misses


## [0.4.1] (2022-04-08)

### Fixed
* fix swapped inodes
* add cache hits to various places


## [0.4.0] (2022-04-08)

### Fixed
* use sentry feature properly as `with_sentry`

### Changed
* using inodes number generated within fuse-zstd
* new inode cache implemented

### Removed
* support for sled in-disk cache


## [0.3.1] (2022-04-05)

### Fixed
* fix building of deb package


## [0.3.0] (2022-04-05)

### Fixed
* better handling of opened tmp files

### Added
* flush() function implemented


## [0.2.2] (2022-01-11)

### Fixed
* store proper uncompressed size within zstd header
* set log level fix
* unlinking of a file which hasn't been compressed yet (in convert mode)


## [0.2.1] (2021-12-16)

### Fixed
* put sentry releases to gitlab CI


## [0.2.0] (2021-12-13)

### Added
* sentry integration


## [0.1.1] (2021-12-03)

### Fixes
* use proper inode in create
* cache overriden inodes before kernel dcache is expired
* opened files are handled in more robust way


## [0.1.0] (2021-12-02)

### Added
* implemented fsync call
* make append mode while writing working properly
* better handling of parallel writes
* design docs


## [0.0.0] (2021-12-01)

### Added
* basic functions
* some few tests
* added convert mode which will convert uncompressed files to compressed
