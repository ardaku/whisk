# Whisk
[![tests](https://github.com/AldaronLau/whisk/actions/workflows/ci.yml/badge.svg)](https://github.com/AldaronLau/whisk/actions/workflows/ci.yml)
[![GitHub commit activity](https://img.shields.io/github/commit-activity/y/AldaronLau/whisk)](https://github.com/AldaronLau/whisk/)
[![GitHub contributors](https://img.shields.io/github/contributors/AldaronLau/whisk)](https://github.com/AldaronLau/whisk/graphs/contributors)  
[![Crates.io](https://img.shields.io/crates/v/whisk)](https://crates.io/crates/whisk)
[![Crates.io](https://img.shields.io/crates/d/whisk)](https://crates.io/crates/whisk)
[![Crates.io (recent)](https://img.shields.io/crates/dr/whisk)](https://crates.io/crates/whisk)  
[![Crates.io](https://img.shields.io/crates/l/whisk)](https://github.com/AldaronLau/whisk/search?l=Text&q=license)
[![Docs.rs](https://docs.rs/whisk/badge.svg)](https://docs.rs/whisk/)

A simple and fast two-way async channel.

## MSRV
Whisk targets Rust 1.59.0 and later.

## Benchmarks
Benchmarks for v0.2.0:

```
function/call           [1.5749 ns 1.5818 ns 1.5887 ns]
extern/call             [1.5249 ns 1.5355 ns 1.5452 ns]
ffi/call                [2.9104 ns 3.3309 ns 3.8884 ns]
whisk/call              [213.22 ns 214.15 ns 215.03 ns]
flume/call              [512.15 ns 537.62 ns 572.21 ns]
whisk/threads           [10.455 us 12.039 us 14.052 us]
flume/threads           [13.157 us 15.460 us 18.298 us]
```
