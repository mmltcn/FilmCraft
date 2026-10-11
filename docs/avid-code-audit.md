# Avid import audit and validation

This record covers the native AVB reader, catalog/sequence adapters, component media model,
MXF changes, engine transaction boundaries and desktop import/relink UI. It describes tested
coverage, not complete reverse engineering or certification for arbitrary Avid projects.
See [the workflow and limits](avid-import-plan.md) for the current product contract.

## Review focus

- Normalize full identities while preserving raw byte order and source evidence. Reject
  conflicting identities, duplicate live objects and ambiguous copies.
- Bound file reads, counts, allocations and graph traversal. Truncation, cycles, unsupported
  structures and hostile command parameters must produce errors rather than panic.
- Keep current-bin metadata, source metadata, source-component timing and timeline placement
  distinct. Validate derived catalog entries against decoded bin objects.
- Keep every logical audio slot, including missing A1 with present A2. Do not substitute the
  first available file for a complete source.
- Validate replacements using playback's identity/profile/range checks. Treat incomplete scans
  and stale project snapshots explicitly; never auto-select an ambiguous candidate.
- Apply successful import and relink operations atomically with undo/redo and save/reopen coverage.
  Failed preparation, cancellation or changed source bytes must leave the project untouched.
- Keep parsing off the UI thread and action controls accessible with expanded reports and many
  folder rows. Search evidence and applied links must be visibly different states.

## Reproduced defects fixed during development

The relevant regressions failed before these corrections and pass afterward.

| Defect | Correction |
| --- | --- |
| Catalog completeness checked only against its own count | Cross-check canonical identities and placement against decoded masters, so deleting a clip and decreasing the count cannot silently omit it. |
| Duplicate detection used raw bytes plus byte-order labels | Compare canonical material-number fields while retaining original evidence. |
| Derived catalog fields could disagree with bin objects | Validate names, metadata, rates/lengths, attributes and ordered source-track structure. |
| Media binding trusted derived references alone | Compare full IDs, source tracks, offsets and extents against the actual bin source clip before binding. |
| Zero-length source padding was discarded before validation | Validate unsupported/truncated flags, kind and rate on every child before removing permitted filler. |
| Composition/timecode completeness checks were incomplete | Validate primary timing evidence while retaining untranslated auxiliary tracks separately. |
| Ordinary Avid OP-Atom notices disabled automatic matching | Recognize only the exact validated companion-reference and opaque-packet notices; preserve all text and continue blocking on other parser warnings. |
| Repeated native references could amplify copied payloads | Charge variable payloads and normalized values to graph byte budgets in addition to node/depth limits. |
| UI search assertions could read stale labels | Require a completed search report and let the previous-frame automation registry catch up before checking labels. |

The last UI case reproduced in a parallel release run while passing in isolation. All twelve
media UI cases subsequently passed in ten consecutive parallel runs. The correction changes
the test's synchronization, not relinking behavior or its expected results.

The separate research reader also received short-ID collision, truncated-leaf and malformed
helper-envelope fixes. Its Python boundary and PMR/MDB behavior are not part of the FilmCraft
product implementation in this PR.

## Public regressions

Synthetic fixtures cover AVB truncation/mutation, object bounds, dangling/cyclic references,
Unicode/MacRoman text, endian conversion, unsupported objects and graph budgets. Engine tests
cover invalid parameters, metadata cross-checking, bin naming collisions, independent channel
signals, partial media, wrong identities, save/reopen, cancellation and atomic undo/redo.

Relink tests include multiple roots, overlapping roots in either order, renamed Unicode paths,
duplicate copies, stale results, symlink/incomplete searches and metadata-read limits. UI tests
exercise search-versus-link guidance, explicit duplicate selection, leaving one channel offline,
preview-before-apply, cancellation and 800×600 layouts with expanded reports and 32 folder rows.

Run the repository gate from the checkout with the required Rust toolchain installed:

```sh
cargo xtask ci
```

Focused tests can be run independently:

```sh
cargo test -p filmcraft-interchange avid
cargo test -p filmcraft-engine avid
cargo test -p filmcraft-engine component
cargo test -p filmcraft-ui-egui --test media_ui
```

The gate performs formatting, release Clippy with warnings denied, release workspace tests,
dependency-layer checks, asset attribution checks and WASM compilation. Private/environmental
checks marked ignored are not silently counted as covered by this gate.

## Private corpus evidence

An explicit Phases 1–12 manifest contains 25 AVBs. Native decoding was compared with an external
pyavb 1.4.0 oracle for 95,990 core objects and 14,269 supported location/descriptor/effect objects,
checking the exported supported properties. Native metadata tests transfer 1,999 placed masters,
including 1,953 in the large sound-effects bin. Third-party decoder implementation source was
not read or adapted for the native modules.

- Phases 4/5 check current-bin names and user columns independently of media metadata.
- Phases 9–12 check managed source identities, mono/stereo routing, source offsets, gaps,
  sequence lengths and offline reconstruction.
- Phase 12's native import links six components and agrees with the earlier catalog workflow's
  media/timeline data. The sequence retains 152 frames and five clips.
- Selected Phase 12 picture decodes differ from the external FFmpeg oracle by at most one
  8-bit YUV code value; decoded PCM, seeking and timeline audio checks pass.
- Native save/reopen and one-step undo/redo pass. The user confirmed playback, the File-menu
  import and multiple-folder UI in the test application.
- SHA-256 checks retain all 25 AVBs and the ten Phase 12 source files unchanged.

These tests are opt-in. Use the explicit private manifests and report paths expected by the
ignored tests; do not scan unrelated directories or redistribute source media. The relevant
inputs include `FILMCRAFT_AVB_CORPUS_MANIFEST`, `FILMCRAFT_AVB_SEMANTIC_ORACLE` and
`FILMCRAFT_AVID_PHASE12_REPORT`. Corpus files, generated reports, screenshots, launchers and
converted projects are excluded from the repository.

Real-file evidence is currently from macOS/APFS. Synthetic big-endian cases do not establish
coverage of every Windows AVB variant. Complex feature-film bins remain deferred.

## Acceptance boundaries

The standalone research catalog's Phase 6 counts are 1,953 masters, 6,934 live MDB matches,
3,043 verified MXF identities, 1,761 complete clips and 192 partial clips. They describe
identity/availability evidence, not native playback acceptance.

The native default import preserves all 1,953 masters, but currently binds only five offline
sources; the other 1,948 retain metadata and diagnostics. Timing, missing source evidence,
track slots and descriptor ranges require further work. Search currently follows binding and
cannot rescue a rejected source using MXF evidence. This limitation is intentional and visible;
it must not be reported as successful large-library playback.

The tested automatic frame-rate/spatial adapter mappings are not proof of Avid rendering fidelity.
AMA binding, general retimes/effects, complex sequences, color presentation and broad decoder
profiles remain outside this acceptance result.

## Checkpoint gate

The checkpoint at `f569f9f` passed `cargo xtask ci`: 2,881 tests passed, none failed and 62 were
ignored; layering, attribution for 69 assets and WASM checks across 44 crates passed. Two
pre-existing gate defects (zero-softness Extract and caption-picker automation targeting) are
kept in the separate maintenance commit `f10fda5`. The Extract tolerance was not relaxed.

Local evidence is retained in ignored `target/avid-checkpoint-ci-green.log`, the targeted
`target/avid-audit-*` / `target/avid-multiroot-*` logs and the explicit private manifests. Subsequent
changes were checked separately in the final submission gate below.


## Final submission gate

After dialog localization, explicit preview-omission counts and documentation consolidation,
`cargo xtask ci` passes on macOS 27.0.1 arm64 with Rust 1.95.0: **2,881 passed, 0 failed,
62 ignored**, plus formatting, release Clippy, layers, 69 attributed assets and 44 WASM crates.
Workspace release and WASM artifacts were rebuilt after checking the isolated maintenance branch.
The log is retained locally as `target/avid-submission-ci-clean.log`.

All twelve media UI tests also pass with offscreen rendering enabled. The inspected renders
confirm visible action buttons at 800×600 with expanded reports, long paths and 32 folder rows,
and the orange quoted Link-button guidance. These renders remain in ignored
`target/avid-submission-ui/`; the test log is `target/avid-submission-ui.log`.

The separately submitted maintenance tree passes its own complete gate: 2,807 passed, 0 failed,
41 ignored. Git commit attribution was corrected to the contributor's GitHub no-reply identity;
the tested source trees were unchanged by that metadata correction.
