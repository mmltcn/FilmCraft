# Avid bin import: workflow, contract and remaining work

FilmCraft reads an AVB once and transfers supported contents into native project data. It does
not embed the AVB, synchronize with it or write back to it. Media remains in its original
locations; save the imported project as `.fcproj`.

This is a supported-subset importer. It does not implement persistent media databasing,
modular project storage or a complete Avid timeline renderer. Those boundaries are described
in [the issue/PR breakdown](avid-issue-breakdown.md).

## Desktop workflow

1. Choose **File > Import Avid Bin…** and select one `.avb`.
2. Optionally choose managed MXF folders. **+ Add folder** adds another location, including
   another drive. Selecting a parent includes its numbered subfolders. Leave the list empty
   to import supported sources offline.
3. Choose **Read bin**, then review the clip/sequence counts and conversion problems.
4. Choose **Import** to apply the reviewed snapshot in one undoable edit.

There is one import action for all contents. Recoverable clips and sequences are retained even
when other items fail. A report with conversion problems remains open after import. Technical
reader notices are shown separately from errors; a notice alone does not imply missing media.
Reports and folder lists scroll while the action buttons remain accessible.

Each AVB creates a bin named from its filename. Name collisions use the first free sibling
suffix: `Sound Effects`, `Sound Effects_2`, `Sound Effects_3`. Existing bins are never merged or
overwritten implicitly. Multiple-AVB selection is not yet available in this dialog.

Imported sources use FilmCraft's **Link Media…** dialog when components need reconnecting.
Search only finds candidates. Choose **Link N files** to apply the selected replacement and,
when enabled, other unique matches. Duplicate copies require an explicit choice. Leaving A1
offline preserves A2 in its original channel slot.

## Supported data and playback

- Placed master names, current-bin user metadata, full identities, source descriptors and raw
  provenance are retained separately from media availability.
- Native decoding covers the observed membership, attributes, source references, descriptors,
  locators and structural timeline wrappers. Unknown payloads remain inspectable; unknown
  semantics are not silently treated as supported effects.
- A logical source may contain independently identified picture and mono PCM MXF components.
  Missing channels retain their positions and render silence. Missing picture displays an
  offline slate while usable audio can still play.
- Mono and paired-stereo channel grouping are preserved in the tested managed-media profiles.
  Source-component track numbers, logical audio slots and timeline track identities remain
  distinct.
- Save/reopen does not require access to the original AVB. Raw evidence and conversion diagnostics
  remain in project metadata; undo/redo restores the complete import.

Filesystem presence does not establish Avid's own online status. FilmCraft distinguishes an
identity match, a supported decoder/profile and a usable component binding.

### Sequence conversion

The validated subset includes cuts, gaps/filler, source offsets, mono/stereo audio and the
observed channel-combiner wrappers. The importer attempts every composition:

- Supported source intervals retain their timeline positions and durations.
- An unsupported single-input effect can be bypassed only when its duration and rate agree
  with its input; the omission is reported.
- Known but unsupported intervals become labeled offline placeholders. A composition with a
  trustworthy clock and extent can be retained as a placeholder sequence.
- Unknown or inconsistent timing is reported per composition without blocking other contents.

The tested automatic mixed-rate adapter maps to FilmCraft Frame Sampling at 100% playback
speed. The tested spatial adapter maps to Scale to Frame Size. Avid's cadence, interpolation,
crop/reformat treatment and effect rendering may differ. This is not a fidelity claim.

Original color attributes and auxiliary tracks are retained as evidence, but visual color
mapping, markers, mixer automation, creative retimes, transitions, nesting and complex effects
are outside the validated subset. Sequence audio output currently uses FilmCraft's 48 kHz
stereo defaults.

## Identity and timing rules

Managed MXF matching uses complete 32-byte file-package identities, normalized according to
encoded byte order. Filenames, display URNs, shortened IDs and search result order do not select
replacements. Candidates are reopened and verified against the expected component before apply.

Stored decimal AVB rates remain preserved as decoded evidence. Playback uses a verified rational
clock where supported. For the observed Phase 12 adapter, output rate `24/1` and speed ratio
`1001/1250` independently establish source rate `30000/1001`, corroborating its MXF. That permits
the same source clock while offline without guessing from the decimal `29.97` alone.

Avid's [published rate convention](https://developer.avid.com/ctms/api/aa/resources/keyframes.html)
maps 29.97, 23.976 and 59.94 to their standard rational clocks. That documentation does not
specify every AVB field's encoding. Original counts, component rates and source/timeline extents
are kept separate; unexplained rates do not receive a blanket NTSC substitution.

## Bounds, cancellation and atomicity

AVBs are capped at 256 MiB and one million indexed objects. Native graph expansion has separate
node, depth, attribute and byte budgets. Malformed references, cycles and truncated payloads
produce errors. Parsing runs in a guarded background job and makes no project edits.

Up to 32 media roots share a limit of 32 directory levels, 1,024 directories and 20,000 entries.
Canonical directory keys deduplicate repeated, aliased and overlapping selections. Distinct file
copies retain their ambiguity. MXF metadata reads and diagnostic output are bounded.

Incomplete searches, unreadable paths, skipped symlinks or exhausted limits prevent automatic
linking. An explicit, independently verified replacement can still be chosen through Link Media.
Cancellation leaves the project untouched. Preparation rereads and compares AVB bytes after
conversion, including same-size changes, and rejects changes to the destination project before
applying the snapshot.

## Commands

The desktop action uses these engine commands; automation uses the same transaction path:

| Command | Contract |
| --- | --- |
| `file.prepareAvidBin` | `{path,parent?,mediaFolders?:[string],wait?}` starts preparation. |
| `file.avidBinPreparation` | `{job}` returns completion state and the prepared report. |
| `file.importAvidBin` | `{job}` commits the reviewed snapshot once. |
| `file.inspectAvidBin` | `{path}` inspects container and membership without importing. |
| `media.searchComponents` | Searches managed components by identity. |
| `media.componentSearchResult` | `{job}` retrieves search evidence without applying it. |
| `media.applyComponentSearch` | Applies an explicit choice and optional unique companions. |
| `media.relinkComponents` | Validates and applies explicit component paths atomically. |

Folder paths must be existing absolute directories. The legacy `mediaFolder` string remains
accepted, but cannot be combined with `mediaFolders`. Editing a source or folder field invalidates
the preview. The native default attempts all clips and sequences; restricted modes remain for
research tests rather than separate UI tabs.

`file.previewAvidCatalog` and `file.importAvidCatalog` are development interfaces for the standalone
reader's schema-2 JSON. Their explicit mode is `metadata`, `media` or `sequences`; their stricter
binding/conversion behavior is separate from native best-effort import. Python decoders are
external comparison tools and are not invoked or distributed by the FilmCraft product.

## Project compatibility

The project schema advances from 15 to 18: audio components, combined picture/audio references,
and pending picture-format verification. Migrations read the intermediate audio-only form.
Older FilmCraft builds reject these newer project schemas rather than silently discard bindings.
Proxies, generic Make Offline, collection/consolidation and reference-based interchange export
have explicit limitations for component sources; unsupported operations must not fall back to
an arbitrary component as though it were the whole source.

## Validation and limits

See [the audit record](avid-code-audit.md) and the
[interchange README](../crates/interchange/README.md#native-avb-semantic-transfer) for tests,
corpus coverage and decoder provenance. Private media, projects, screenshots and reports are
not included in the repository. Public regressions generate their own fixtures.

The native large-bin check retains all 1,953 masters. Only five currently bind as offline sources;
1,948 retain metadata and binding diagnostics. This is **not native Phase 6 playback acceptance**.
The earlier standalone reader's 6,934 live MDB matches, 3,043 verified MXF identities and
1,761 complete / 192 partial clip counts describe catalog evidence, not this playback path.

Current import preparation binds descriptors before searching selected MXFs. A failed binding
therefore cannot yet be rescued by that search. The next media-coverage increment should index
selected media first and use verified identities and timing evidence during binding. Observed
quarter-frame audio rates and component ranges need their own regression coverage.

Other follow-ups are AMA media binding/remapping, broader codecs/descriptors, real Windows AVB
samples, multiple-bin selection, color presentation and more timeline semantics. Complex
feature-film bins are deferred until after the initial PR. No additional user captures or
routine database rebuilds are required for the current subset.
