# Whisk
A simple and fast two-way async channel.

## Benchmarks
Initial benchmarks for v0.1.0:

```
function_call           time:   [905.88 ps 908.10 ps 910.39 ps]
extern_call             time:   [1.2176 ns 1.2282 ns 1.2385 ns]
ffi_call                time:   [2.1366 ns 2.5387 ns 3.0501 ns]
whisk_call              time:   [7.1319 us 7.3810 us 7.6329 us]
```
