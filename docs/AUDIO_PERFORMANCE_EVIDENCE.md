# Audio mixer optimization: correctness and performance evidence

Measured 2026-10-10 against the historical development tree based on `98ba9acbcedcbb630ba2a07b782176b66835edd2`. These results validate that tree and the final audio candidate, not a subsequently rebased submission revision. The final measured audio patch SHA-256 is `ce6b8dd1ef7ee90bf8eec723e0478cfa761ea2a5c8ddcfbcce9af3cd66b43aec`.

## Changes covered

Static mixer inserts refresh settings once per request; keyframed inserts retain their original absolute 64-sample evaluation grid and live edits take effect at the next request. Settled EQ avoids redundant smoothing and safely bypasses exact identity bands while preserving tails, signed zero and non-finite behavior. Delay/reverb uses bounded circular indexing, caches unchanged loop gains on the original update grid, and prepares interpolation positions for settled delays once per block. Moving delays retain sample-by-sample behavior.

## Correctness

The final release suites passed with the existing performance gate enabled and unchanged. Four new DSP tests compare optimized processing with pre-optimization loops or reference indexing:

- `flat_eq_fast_path_preserves_samples_and_filter_tails`
- `settled_and_automated_eq_matches_original_loop`
- `circular_reads_match_modulo_reference`
- `reverb_automation_and_tail_match_original_loop`

The render regression `constant_insert_refresh_matches_grid_refresh_and_live_edits` also passed. Coverage includes bit-exact output, automation, irregular/zero-sized blocks, reset, tails, signed zero, subnormal and non-finite samples, and live parameter edits.

| Platform / final suite | DSP | Oracle integration | Render |
|---|---:|---:|---:|
| Linux, Intel Core i7-10700K | 123 passed, 2 ignored | 1 passed | 179 passed, 2 ignored |
| macOS 27.2, Apple M4 | 123 passed, 2 ignored | 1 passed | 179 passed, 2 ignored |
| Later Linux/NVIDIA host | 123 passed, 2 ignored | 1 passed | 179 passed, 2 ignored |

Apple's actual command used direct Rust 1.99.0 binaries with the following arguments and environment:

```sh
RAYON_NUM_THREADS=8 cargo test --release --locked -j6 \
  -p filmcraft-audio-dsp -p filmcraft-render
```

The final clean Intel combined suite used one test thread with no filters and no concurrent validation builds. It additionally passed export 72 plus one integration, GPU 37 plus one integration, and 33 goldens. A separate render rerun with two test threads also passed all 179 tests, with two ignored. Release all-target audio-DSP/render Clippy with warnings denied and formatting checks passed. The later NVIDIA suite included the backend fix and therefore 38 GPU tests; its combined total was 448 passed / four ignored.

The retained Intel runner used:

```sh
RAYON_NUM_THREADS=8 mise exec rust@stable -- cargo test --release -j6 \
  --no-fail-fast -p filmcraft-audio-dsp -p filmcraft-render \
  -p filmcraft-export -p filmcraft-gpu -p filmcraft-golden \
  -- --test-threads=1 --nocapture
```

## Paired unchanged audio workload

The existing `mixer_tests::perf_24_tracks_3_effects_realtime_factor` renders ten seconds of 48 kHz stereo through 24 tracks with EQ/compression/reverb inserts and a submix. It explicitly creates one Rayon worker. On these platforms it measures that worker's thread CPU time. The release gate remains strictly greater than 4× realtime, equivalent to less than 2.5 CPU seconds. No threshold was lowered and no performance test was filtered from the final audio suites.

The saved before/after executables were invoked separately with:

```sh
RAYON_NUM_THREADS=8 ./render-test-binary --exact \
  mixer_tests::perf_24_tracks_3_effects_realtime_factor --nocapture
```

| CPU | Before CPU seconds, runs 1 / 2 | After CPU seconds, runs 1 / 2 | Before realtime | After realtime | Combined CPU-time speedup |
|---|---:|---:|---:|---:|---:|
| Intel Core i7-10700K | 3.923 / 4.031 | 1.312 / 1.306 | 2.5× / 2.5× | 7.6× / 7.7× | 3.04× |
| Apple M4 | 1.722 / 1.720 | 0.609 / 0.650 | 5.8× / 5.8× | 16.4× / 15.4× | 2.73× |

Combined speedup is summed before CPU seconds divided by summed after CPU seconds. Intel ran before → after → after → before; Apple ran before → after → before → after. Both Intel baseline invocations failed the unchanged gate, both final candidate invocations passed. All four Apple invocations passed. No validation build/test job ran concurrently with either paired sequence. External workload was not fully controlled: Apple's one-minute load was 25.13–25.84. Thread CPU time reduces scheduling interference but is not a guarantee of identical thermal/cache conditions.

The clean Intel full suite measured 1.326 CPU seconds / 7.5× realtime; the two-test-thread render rerun measured 1.675 seconds / 6.0×. The later NVIDIA-host full suite measured 0.940 CPU seconds / 10.6×. The latter is a passing final measurement, not a paired before/after speedup or evidence that the GPU accelerated audio.

## Submission-tree test isolation

The independent audio submission is based on `ab7318e788528515605f02c024dc1170368964f9`. On this newer tree, the same optimized render test executable passed its unchanged timing gate alone but failed when the render correctness suite ran concurrently inside that executable. With production thin LTO, the isolated workload took 2.109 thread-CPU seconds (4.7× realtime), versus 3.843 seconds (2.6×) in the full render suite. The no-LTO executable also passed alone at 1.896 seconds (5.3×), so these observations do not establish LTO as the cause.

The timing gate now lives in `crates/render/tests/mixer_perf.rs`, its own integration-test executable. Cargo runs test executables sequentially, avoiding competition with this crate's correctness tests for CPU frequency, cache and memory bandwidth. The unit tests and timing gate share the original synthetic fixture in `tests/support/mixer.rs`; there is no second, simplified workload. The 24 tracks, insert order, automated faders, submix/sends, 48 kHz stereo, 4,800-sample warmup, 1,024-sample requests, ten-second release workload, one Rayon worker, thread-CPU clock, strict greater-than-4× release threshold, debug behavior and non-Unix behavior are unchanged. This addresses test interference; the DSP and mixer changes above provide the actual throughput improvement. Other external load can still affect a timing gate.

The final submission-tree workspace run passed **2,785 tests, zero failures, 41 existing ignores**, including doc tests. Render contributes 172 passing unit tests / two ignored and the passing standalone timing test; DSP contributes 123 passing unit tests / two ignored and one passing oracle integration test. The test harness used its default parallelism with no filters. To bound whole-workspace linking time, this broad run disabled release LTO while retaining optimization level 3; this is a verification-profile distinction, not a change to the checked-in release profile:

```sh
CARGO_PROFILE_RELEASE_LTO=false CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=8 \
  cargo clippy --workspace --all-targets --release -j4 -- -D warnings
CARGO_PROFILE_RELEASE_LTO=false CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=8 \
  cargo test --workspace --release -j4 --no-fail-fast
```

The final production thin-LTO render run also passed: 172 unit tests / two existing ignores and the standalone gate at **1.356 thread-CPU seconds, 7.4× realtime**. Formatting, dependency layering, attribution (69 assets) and WASM (44 crates) passed on the final tree:

```sh
CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=8 \
  cargo test --release -j4 -p filmcraft-render --lib --test mixer_perf -- --nocapture
cargo fmt --all --check
cargo xtask layers
cargo xtask assets
cargo xtask wasm
```

The production invocation did not set `CARGO_PROFILE_RELEASE_LTO`. These submission-tree results are Linux validation; the historical Apple M4 measurements above do not claim that this newer rebased tree or a sustained audio/video film export was tested on macOS.

## Limitations and source audit

This is a synthetic one-worker mixer workload, not end-to-end film export timing. There are two paired samples per platform and no statistical confidence interval. Earlier candidates and load-contaminated timing failures are retained in the private evidence but are not substituted for final clean results. The first optimization candidate improved performance but narrowly missed the gate in a broader Intel run, motivating the final settled-delay preparation. A later final-candidate run overlapped a queued benchmark rebuild; the clean final suites and paired runs above supersede it for throughput claims.

Values were checked against the final Intel before/after test logs and consolidated audio results, Apple's `tests-candidate2.log`, `before-*.log`, `after-*.log`, `paired.log` and load snapshots, and the later NVIDIA final test log. Apple's actual paired ordering is taken from its execution log; an older consolidated prose summary gave a different ordering. Raw measurements are unchanged. Reports exclude private media identifiers, paths, machine names, account details and operational history.
