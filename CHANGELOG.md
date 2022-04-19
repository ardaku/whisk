# Changelog
All notable changes to `whisk` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://github.com/AldaronLau/semver).

## [0.2.0] - Unreleased
### Added
 - `start()` async methods on both `Commander` and `Messenger` for initiating
   the connection.

### Changed
 - `Commander` and `Messenger` changed from `Future`s to `Iterator`s.

### Removed
 - `close()` methods on both `Commander` and `Messenger`, now you can just rely
   on dropping to close channels.

### Fixed
 - Incorrect assumptions about wakers

## [0.1.1] - 2022-03-04
### Fixed
 - Deadlock condition caused by wrong drop order

## [0.1.0] - 2022-02-27
### Added
 - `Command`
 - `Commander`
 - `Message`
 - `Messenger`
 - `channel()`
