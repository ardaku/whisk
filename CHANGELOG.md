# Changelog
All notable changes to `whisk` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://github.com/AldaronLau/semver).

## [0.4.0] - 2022-07-22
### Added
 - **`pasts`** feature for `Channel<T>` to implement `Notifier`
 - **`futures-core`** feature for `Channel<Option<T>>` to implement `Stream`
 - `Message` future type
 - `Channel::new()` for creating a channel
 - `Channel::send()` for sending a message on a channel
 - `Channel::recv()` for receiving a message from a channel

### Changed
 - Channels are now MPMC instead of SPSC
 - Less unsafe code, simpler implementation
 - Channels are now symmetrical

### Removed
 - `Sender` - Functionality built into `Channel` now
 - `Receiver` - Functionality built into `Message` now
 - `Worker` - Functionality built into `Message` now
 - `Tasker` - Functionality built into `Channel` now
 - **`std`** feature
 - `Channel::to_pair()` - Use `Channel::new()` instead
 - `Channel::pair()` - Use `Channel::new()` instead

## [0.3.0] - 2022-07-10
### Added
 - `Channel` struct, and `Channel::to_pair()` for reusing oneshot channels
 - `Sender` and `Receiver` oneshot-rendezvous channels
 - `Worker::stop()` to close a spsc channel

### Changed
 - Replaced `channel()`, with `Channel::pair()`
 - Renamed `Commander` to `Worker`, and `Messenger` to `Tasker`

### Removed
 - `Command`, `Message` types
 - `Commander::start()` and `Messenger::start()`, use `Worker::send()` and
   `Commander::recv_next()` instead

## [0.2.1] - 2022-04-19
### Changed
 - Small optimizations

## [0.2.0] - 2022-04-19
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
