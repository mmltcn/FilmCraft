# GPU export rendering: hardware evidence

Measured 2026-10-10. This report describes the previously validated development tree based on `98ba9acbcedcbb630ba2a07b782176b66835edd2`, including its GPU export changes. It is historical evidence, not a claim that a subsequently rebased PR revision has already passed these checks. The later NVIDIA candidate also includes the separately reviewed audio optimization and standalone CLI backend fix.

## Workload and protocol

The private 3840×2160 project combines a full-length Lumetri Faded Print adjustment with still images, Motion, Gaussian Blur, H.264 footage, and Neutral Mono clip effects. Three frames starting at 60, 390, 1590, 3370 and 3500 were compared against the CPU reference after two warmups. Error is the maximum RGB-channel absolute difference per pixel in 8-bit output levels. The existing criteria are p99 ≤6 and mean <1.5; no tolerance was loosened.

Benchmark invocation, with an intentionally generic project filename:

```sh
cargo build --release -j6 -p filmcraft-gpu --example export_bench
RAYON_NUM_THREADS=8 target/release/examples/export_bench fixture.fcproj 60 3 verify none 2
RAYON_NUM_THREADS=8 target/release/examples/export_bench fixture.fcproj 60 20 gpu none 2
```

The harness renders sequential frames, reads back GPU output and converts linear pixels to RGBA8. Timed throughput excludes audio, encoding and muxing. Each measured run has 20 frames and two warmups; eight Rayon workers are used throughout. GPU-only stage counters are checked separately: CPU stage time in `verify` includes the deliberate CPU reference render.

Hardware adapters were confirmed, rather than inferred from successful API initialization:

| Platform | Adapter/backend | Driver/toolchain |
|---|---|---|
| Linux | Radeon Pro 5500 XT; `AMD Radeon Graphics (RADV NAVI14)` / Vulkan | Mesa 26.2.2-arch1.1 |
| macOS 27.2, arm64 | Apple M4, 10 CPU / 10 GPU cores, 16 GiB / Metal | Rust 1.99.0 |
| Linux, amd64 | GeForce RTX 3090, 24 GiB / Vulkan | NVIDIA 610.57.04; Rust 1.97.1 |

The AMD and Apple checks used equivalent project content. The NVIDIA checks used a later edited snapshot: baseline and candidate on that machine used the same snapshot, but cross-machine results are not an identical-project comparison. Local media was used on AMD; Apple and NVIDIA used read-only network media with writable copies of the project for path remapping.

## Full-pixel parity

Each cell is worst mean error / worst p99 across the three measured frames.

| Start frame / coverage | AMD Vulkan | Apple Metal | NVIDIA Vulkan |
|---|---:|---:|---:|
| 60: still, Motion, adjustment | 0.148006 / 1 | 0.185899 / 1 | 0.151009 / 1 |
| 390: portrait, blurred background, adjustment | 0.242118 / 2 | 0.264034 / 2 | 0.286851 / 2 |
| 1590: H.264 video, Motion, adjustment | 0.449857 / 2 | 0.449824 / 2 | 0.428338 / 2 |
| 3370: Neutral Mono video, adjustment | 0.037435 / 1 | 0.037556 / 1 | 0.036988 / 1 |
| 3500: Neutral Mono still/blur, adjustment | 0.112528 / 1 | 0.122151 / 1 | 0.119548 / 1 |

All 15 verification invocations passed. Every invocation reports three GPU frames, zero whole-frame CPU-plan frames, zero CPU-compositor fallbacks and zero source errors. Every candidate throughput run reports 20 GPU frames and zero CPU-plan/compositor fallbacks, source errors, and CPU Lumetri/blur/Motion/other-effect/source-conversion stage time.

## Linux baseline versus candidate

Both versions were invoked in GPU mode, in baseline → candidate → candidate → baseline order. Aggregate fps is 40 divided by the combined measured wall seconds for each version, not an arithmetic average of fps. No benchmark build/test job or competing film export ran during the final AMD set; no build or competing inference workload ran during the final NVIDIA set.

The AMD baseline is the preserved pre-change benchmark. The NVIDIA baseline was rebuilt natively from the stated base commit with the same harness, adapted only to omit counters unavailable in the old code; separate baseline/candidate target directories were used for final verification. These are comparisons of the combined changes, not isolated gains from individual optimizations.

| Adapter | Segment | Baseline fps, two runs | Candidate fps, two runs | Aggregate fps before → after | Ratio |
|---|---|---:|---:|---:|---:|
| AMD | Still | 3.313 / 3.406 | 24.713 / 24.145 | 3.359 → 24.426 | 7.27× |
| AMD | Blurred portrait | 1.916 / 2.069 | 14.439 / 14.702 | 1.989 → 14.569 | 7.32× |
| AMD | H.264 video | 3.459 / 3.956 | 12.899 / 12.897 | 3.691 → 12.898 | 3.49× |
| NVIDIA | Still | 3.327 / 3.610 | 36.133 / 39.885 | 3.463 → 37.916 | 10.95× |
| NVIDIA | Blurred portrait | 2.321 / 2.213 | 27.769 / 28.311 | 2.265 → 28.038 | 12.38× |
| NVIDIA | H.264 video | 3.455 / 3.405 | 24.012 / 27.107 | 3.430 → 25.466 | 7.42× |

Device-wide utilization includes startup and warmup. AMD busy-percent readings were sampled every 250 ms; NVIDIA readings approximately every 200 ms. Cells below show mean / maximum percent in run order.

| Adapter / segment | Baseline 1 | Candidate 1 | Candidate 2 | Baseline 2 |
|---|---:|---:|---:|---:|
| AMD / still | 8.5 / 84 | 34.6 / 74 | 20.2 / 58 | 6.2 / 14 |
| AMD / blur | 7.2 / 45 | 60.6 / 99 | 67.1 / 99 | 4.1 / 12 |
| AMD / H.264 | 5.9 / 80 | 10.3 / 19 | 23.8 / 70 | 2.8 / 41 |
| NVIDIA / still | 0.7 / 25 | 3.6 / 27 | 11.2 / 27 | 2.0 / 32 |
| NVIDIA / blur | 0.2 / 5 | 19.9 / 59 | 21.1 / 57 | 2.6 / 56 |
| NVIDIA / H.264 | 0.5 / 3 | 9.7 / 26 | 6.4 / 20 | 0.5 / 3 |

Utilization is a rolling device-wide sample, may retain prior activity and does not isolate shader time from transfer waits. Pixel comparisons and renderer counters establish effect execution; utilization alone does not.

## Apple: current CPU mode versus current GPU mode

This is a different comparison: the same candidate executable, CPU-reference mode versus Metal mode, run as two sequential CPU/GPU pairs. It is not a historical baseline/candidate measurement. The one-minute host load was 9.40–11.47; no validation build or film export competed during timing.

| Segment | CPU fps, two runs | GPU fps, two runs | Aggregate CPU → GPU fps |
|---|---:|---:|---:|
| Still | 8.209 / 8.272 | 48.612 / 50.235 | 8.240 → 49.410 |
| Blurred portrait | 4.078 / 4.060 | 17.953 / 15.046 | 4.069 → 16.372 |
| H.264 video | 6.466 / 7.413 | 41.776 / 36.634 | 6.907 → 39.036 |

## Regression tests and standalone CLI

The initial AMD GPU revision passed 37 GPU unit tests, one GPU export integration and 33 goldens; its full render suite had an existing audio throughput failure. The final GPU-only AMD run explicitly filtered that one audio timing test: 177 render passed, two ignored; export 72 plus one integration; GPU 37 plus one integration; golden 33. This filtered run is not evidence that the audio gate passed. The separate audio optimization subsequently addressed it.

The original Apple GPU run used:

```sh
RAYON_NUM_THREADS=8 WGPU_BACKEND=metal cargo test --release --locked -j6 \
  -p filmcraft-render -p filmcraft-gpu -p filmcraft-golden
cargo test --release --locked -j6 -p filmcraft-export settings_tests
```

Results: render 178 passed / two ignored, GPU 37 plus one integration, golden 33; all 21 selected export settings tests passed. The audio gate passed on Apple. No goldens were re-blessed. A nonfatal toolchain debug-stripping warning did not prevent running the produced binaries.

The later NVIDIA candidate passed 448 tests with four ignored: DSP 123 plus one oracle, render 179, export 72 plus one integration, GPU 38 plus one integration, golden 33. Its extra GPU test covers an empty compiled-backend set. The initial standalone CLI export exposed missing backend features hidden by joint desktop builds; after enabling backend defaults in the GPU crate and guarding initialization, standalone CLI export passed. AMD also subsequently passed 38 GPU tests plus integration and the standalone CLI export regression.

```sh
RAYON_NUM_THREADS=8 cargo test --release -j6 \
  -p filmcraft-gpu -p filmcraft-render -p filmcraft-golden \
  -p filmcraft-export -p filmcraft-audio-dsp --no-fail-fast \
  -- --test-threads=2 --nocapture
```

The NVIDIA encoded smoke produced 34 frames of 3840×2160 ProRes at 30000/1001 with 48 kHz stereo PCM: 57,633,357 bytes, 1.134467-second output, approximately 7.6 seconds elapsed export time. External probing verified both streams and external decoding completed without errors. The tested range includes titles, a cut, blurred portrait, adjustment and audio; it contains no explicit transition object.

## Rebased submission checks

The submission was reconciled onto upstream `ab7318e788528515605f02c024dc1170368964f9`.
It preserves upstream CPU-frame pool admission, passing the actual render options into the
eligibility check; adjustment regressions assert that this check agrees with the frame planner.
Workspace-wide release Clippy with warnings denied and formatting passed. The rebased RADV
`export_pipeline::gpu_export_matches_cpu_and_counts` integration passed on the GPU: all eight
frames at 320×180, 17×13, 1×1 and 257×129 had p99=0 and mean=0.

A fresh private-project recheck could not run because its read-only source volume was no longer
mounted. The earlier three-hardware private-project results above therefore remain historical.
The full workspace gate is run with `CARGO_PROFILE_RELEASE_LTO=false` (release optimization
retained; no test threshold/filter changes). Production builds and earlier benchmark binaries
retain their original settings. The initial full native run passed 2,143 tests with 34 ignored,
then stopped on the unchanged mixer throughput gate (5.932 CPU seconds for ten seconds of
audio, 1.69× realtime against the existing >4× requirement). This branch deliberately excludes
the separate audio optimization. The contributor approved independent PRs with this baseline
failure disclosed; it is not a fully green `cargo xtask ci` result. Remaining native/doc tests
were checked separately. Across all 199 native test targets and 46 documentation-test targets,
the deduplicated result is **2,794 passed, 1 known baseline mixer failure, 41 existing ignored**.
The remaining-package invocation used its selected packages' feature unification; the complete
workspace documentation invocation used workspace default features. No all-features or physical
Windows/macOS validation is claimed for this rebased revision. Layering, asset attribution
(69 assets), and WASM (44 crates) passed.

### Sustained-export deadlock found during submission review

A live Apple Metal export stalled with audio disabled. A read-only stack sample showed a
renderer-owning frame waiting for nested Rayon conversion work, then reentering another frame
whose renderer-pool condition-variable wait depended on the suspended outer frame returning
its renderer. Open media descriptors pointed to the local source volume; the sampled stacks
were synchronization waits rather than media reads. Sequential benchmark parity did not cover
this scheduling failure.

Frame admission now uses the number of successfully created renderers, not the Rayon worker
count. Both stepped/overlapped exports and GIF/still-sequence exports cap their actual batches;
an explicit oversized batch request cannot bypass the bound. Each pipeline has only one active
render batch, so every nested frame can obtain a renderer without waiting for its ancestor.
Nested conversion remains parallel, the existing renderer/memory bound remains, and contention
does not silently send eligible frames to the CPU. A GPU-enabled batch containing CPU-only
frames is also capped; optimizing mixed batches is left to later work.

The synthetic `renderer_reentrancy` regression covers PNG, GIF, oversized stepped batches,
and overlapped audio/video using a one-renderer factory that deliberately yields to Rayon.
Its private test subprocesses self-expire after 30 seconds so a regression fails instead of
hanging the runner. This is a test watchdog, not a production stall timeout. Production stall
error handling and full-length Mac export acceptance are separate follow-up work.

All 83 export unit tests and the four regression cases passed with the fix. A negative control
that disabled the admission cap deterministically deadlocked the PNG case: its test child
exited 124 at the 30-second deadline and the harness failed. Restoring the cap made all four
cases pass again, with every frame using the registered renderer and zero CPU fallbacks.
An upstream test hook that registered an unconditional fake renderer was also made thread-local
like its neighboring cases, preventing GPU Auto defaults from contaminating parallel tests.

## Limits and provenance

These short warmed sequential samples are not complete-film export forecasts or GPU-model rankings. Hosts, CPUs, source access and project revisions differ; no confidence interval or per-optimization attribution is claimed. GPU readback and CPU output conversion remain. Masks, partial adjustment coverage, non-Normal blends, unsupported sections/effects, HDR paths and adjustment/transition combinations retain correctness fallbacks. Auto defaults choose supported GPU paths and preserve explicit Off settings; they do not imply every effect has a GPU implementation.

Source audit: AMD values were checked against the final SSD paired `summary.json` and per-run JSON plus the original render notes; Apple values against `verify-*.log`, `bench-*.log`, per-run load snapshots and test logs; NVIDIA values against final per-run JSON/utilization CSV, test log and output probe. Only final uncontended-with-validation paired batches are summarized. Earlier sandbox/software-adapter runs, intermediate failed CLI smoke, and mixed-target build results are excluded. This public report intentionally omits private project/media identifiers, file paths, machine names and operational details.
