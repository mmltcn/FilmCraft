# filmcraft-av1enc

Clean-room pure-Rust baseline AV1 software encoder according to the AOMedia AV1 Bitstream &
Decoding Process Specification v1.0.0 with Errata 1.

## Conformance and Architecture

- **Specification**: AOMedia AV1 Specification v1.0.0 with Errata 1.
- **Pure safe Rust**: `#![deny(unsafe_code)]` with zero panics, no unwrap/expect in production code paths, returning `Result<T, Error>`.
- **Clean-room**: Pure arithmetic coding and bitstream synthesis implemented directly from mathematical specifications and official syntax tables without linking, bundling, or copying third-party code.
- **Verified decoding**: The generated Open Bitstream Units (Temporal Delimiter, Sequence Header, Frame Header, and Tile Group) are validated and decoded by `filmcraft-av1::Decoder`.
