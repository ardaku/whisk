# Whisk
A simple and fast two-way async channel.

## Benchmarks
Initial benchmarks for v0.1.0:

```
function_call           time:   [905.88 ps 908.10 ps 910.39 ps]
extern_call             time:   [1.2176 ns 1.2282 ns 1.2385 ns]
ffi_call                time:   [2.1366 ns 2.5387 ns 3.0501 ns]
whisk/call              time:   [121.16 ns 121.58 ns 122.11 ns]
whisk/threads           time:   [6.3501 us 6.6031 us 6.8963 us]
```
