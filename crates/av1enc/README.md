# filmcraft-av1enc

Reserved API for a future clean-room software AV1 encoder. **Not implemented.**
The previous prototype discarded the input pixels and emitted nonconformant tile data.
It is no longer compiled or registered for export. `Encoder::new` validates dimensions,
frame rate and keyframe interval and returns `Error::Unsupported` for valid configurations.
There is no public API exposing the unchecked prototype entropy coder or transforms.

AV1 export currently requires a registered hardware encoder. A software encoder may only
be enabled after external decoding, image-quality, GOP/seek, bitrate and hostile-input
regressions pass, using the AV1 Bitstream & Decoding Process Specification v1.0.0 with
Errata 1. The source files in `prototype/` are retained as unfinished work for review. They are outside the compiled `src/` tree and are not a codec implementation.
