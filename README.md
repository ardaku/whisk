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

## Benchmarks
Initial benchmarks for v0.1.0:

```
function/call           time:   [1.5397 ns 1.5445 ns 1.5493 ns]
extern/call             time:   [1.5522 ns 1.5566 ns 1.5612 ns]
ffi/call                time:   [2.3279 ns 2.3406 ns 2.3521 ns]
whisk/call              time:   [128.41 ns 128.79 ns 129.15 ns]
flume/call              time:   [461.08 ns 462.58 ns 464.00 ns]
whisk/threads           time:   [7.2141 us 7.2827 us 7.3575 us]
flume/threads           time:   [7.4545 us 7.6034 us 7.7496 us]
```
