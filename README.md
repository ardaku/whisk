# Whisk
[![tests](https://github.com/ardaku/whisk/actions/workflows/ci.yml/badge.svg)](https://github.com/ardaku/whisk/actions/workflows/ci.yml)
[![GitHub commit activity](https://img.shields.io/github/commit-activity/y/ardaku/whisk)](https://github.com/ardaku/whisk/)
[![GitHub contributors](https://img.shields.io/github/contributors/ardaku/whisk)](https://github.com/ardaku/whisk/graphs/contributors)  
[![Crates.io](https://img.shields.io/crates/v/whisk)](https://crates.io/crates/whisk)
[![Crates.io](https://img.shields.io/crates/d/whisk)](https://crates.io/crates/whisk)
[![Crates.io (recent)](https://img.shields.io/crates/dr/whisk)](https://crates.io/crates/whisk)  
[![Crates.io](https://img.shields.io/crates/l/whisk)](https://github.com/ardaku/whisk/search?l=Text&q=license)
[![Docs.rs](https://docs.rs/whisk/badge.svg)](https://docs.rs/whisk/)

#### Simple and fast async channels
Whisk provides oneshot-rendezvous and spsc channels that can be used to
implement futures, streams, notifiers, and actors.

## MSRV
Whisk targets Rust 1.59.0 and later.

## Benchmarks
Benchmarks for v0.3.0 spsc on pasts runtime (compared to dynamic library):

```
Dynamic library: 6ns
Whisk (2-thread): 7.363µs
Flume (2-thread): 7.382µs
Whisk (1-thread): 180ns
Flume (1-thread): 285ns
```
