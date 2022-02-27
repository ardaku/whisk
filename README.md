# Whisk
A simple and fast two-way async channel.

## Benchmarks
Initial benchmarks for v0.1.0:

```
function/call           time:   [1.5397 ns 1.5445 ns 1.5493 ns]
extern/call             time:   [1.5522 ns 1.5566 ns 1.5612 ns]
ffi/call                time:   [2.3279 ns 2.3406 ns 2.3521 ns]
whisk/call              time:   [128.41 ns 128.79 ns 129.15 ns]
whisk/threads           time:   [7.2141 us 7.2827 us 7.3575 us]
```
