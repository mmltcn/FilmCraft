# Persistent workspace layout validation

## Scope

Panel tabs and grips support center docking into tab groups and four edge splits, with a
non-mutating drop preview. Escape, loss of focus, or release outside a destination cancels.
Only primary-button release commits. Timeline's grip moves its panel while sequence tabs retain
their existing reorder behavior. Compact Tools and Audio Meters panels have top-edge grips.

The current working layout is saved separately from named workspace checkpoints. This preserves
Reset to Saved Layout and Save Changes semantics while restoring unsaved arrangement changes at
the next launch. Completed gestures are persisted using a synced temporary file and atomic rename;
write failures leave the previous saved state intact and retry after a delay. Invalid layouts
fall back to the saved/default layout. An intentionally empty root remains valid and reopenable;
opening a project/sequence still reopens Timeline, as before.

This does not add floating windows or multi-monitor docking. No readiness percentage is raised.
The upstream fix and regression for reopening panels when all anchor groups are closed (#284)
are retained unchanged.

## Reproduction

Run on Linux with a Vulkan-capable GPU (optional screenshot rendering):

```sh
cargo fmt --check
FILMCRAFT_UI_SNAPSHOT_DIR=/tmp/filmcraft-layout-shots \
  cargo test --release -j6 -p filmcraft-ui-egui --lib \
  --test workspaces_ui --test timeline_tabs_ui --test sequence_tabs_ui
cargo clippy --release -j6 -p filmcraft-ui-egui --all-targets -- -D warnings
```

The headless harness drives the ordinary in-process control channel and real egui pointer events;
it does not launch a desktop application or open a network listener. Media is the built-in demo.
Screenshots are local review artifacts, not included here because the Media Browser can expose
local directory names.

Coverage includes all drop placements and source collapse, invalid/no-op targets, invalid nested
empty groups, invalid indices/sizes and duplicate panels; ordinary tab and Timeline-grip drags;
compact grip preview and Escape cancellation; automatic restart restore; deliberate empty layout
restore and reopening; persistence failure/retry; saved workspace reset, rename and delete; and
existing sequence-tab drag/reorder, overflow and project reopening behavior.

## Results

Environment: Linux x86-64, AMD Radeon Pro 5500 XT through RADV/Vulkan, Rust stable;
isolated branch based on main `ab7318e7`. No private projects or media were used.

Targeted validation on 2026-10-11: **138 tests passed** (124 UI unit, 3 sequence-tab,
5 Timeline-tab, 6 workspace tests), no failures or skips. Full-workspace formatting and
release all-target UI Clippy with `-D warnings` passed. Offscreen Vulkan screenshots of the
drop preview and rearranged layout were reviewed locally.

The initial empty-layout fixture opened the demo before launch and therefore correctly reopened
Timeline through the existing OpenSequence event. The final fixture launches without a project
to test empty-layout persistence independently; project-opening behavior was not changed.

Local logs: `/tmp/fc-pr-layout-tests-final.log`, `/tmp/fc-pr-layout-clippy.log`.
The initial full `cargo xtask ci` run passed workspace formatting and workspace/all-target
release Clippy (`-D warnings`). Its full-LTO test compilation was stopped before tests ran
because each integration executable required a lengthy link. The full gate was rerun as
`CARGO_BUILD_JOBS=6 CARGO_PROFILE_RELEASE_LTO=false cargo xtask ci`, using the same isolated
target directory and RADV adapter. This retains release optimization and every test/threshold,
but disables link-time optimization for the gate; production binaries are unchanged. The
`wasm32-unknown-unknown` target is now installed with operator approval.
The rerun passed formatting and workspace/all-target Clippy, but stopped in the unchanged render
mixer performance test: `mixer_tests::perf_24_tracks_3_effects_realtime_factor` measured 1.28x
realtime (10 seconds of audio / 7.825 seconds of thread CPU). Before stopping, 2,128 tests passed,
1 failed and 34 were ignored across 122 completed test executables. The render executable itself
reported 171 passed, 1 failed, 2 ignored. Later executables, including the full UI suite, were not
reached; this is **not a passing full CI run**. No thresholds or filters were changed.

A separate exact-test reproduction also failed at 1.15x (8.663 seconds of thread CPU). Other
validation could still compete for resources during these measurements; they are not presented
as idle-machine performance results. No audio optimization is included in this layout patch.

Independent remaining gates passed: `xtask layers`, `xtask assets` (69 attributed assets), and
`xtask wasm` (44 crates). The native performance blocker must be resolved or independently
triaged before claiming all gates pass.
Logs: `/tmp/fc-pr-layout-ci.log` (initial), `/tmp/fc-pr-layout-ci-nolto.log` (full rerun).
Remaining-gate and reproduction logs: `/tmp/fc-pr-layout-remaining-gates.log`,
`/tmp/fc-pr-layout-mixer-repro.log`.

To complete coverage after the fail-fast stop, the entire workspace was rerun with
`CARGO_BUILD_JOBS=4 CARGO_PROFILE_RELEASE_LTO=false cargo test --workspace --release -j4 --no-fail-fast`.
All integration and documentation tests ran, including the complete UI suite. Results:
**2,785 passed, 1 failed, 41 ignored** across 244 result groups. The only failure remained
the unchanged mixer performance test (1.39x realtime, 7.186 seconds of thread CPU). The ignored
tests are the repository's existing ignored tests; no additional filters or ignores were added.
Log: `/tmp/fc-pr-layout-tests-no-fail-fast.log`.

The operator explicitly approved submitting this as an independent layout change, with the
known baseline audio-performance gate failure disclosed, rather than depending on the separate
audio optimization change. This is a scoped gate waiver, **not a claim that full CI is green**.
The no-LTO invocation also differs from the production release profile, so these audio timings
must not be presented as production-build benchmarks. The patch contains no audio changes.
