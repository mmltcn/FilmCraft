# Uncompiled AV1 encoder prototype

These files preserve unfinished local work for design review. None is included by
`src/lib.rs`, exported as an API, or used by FilmCraft's export pipeline. Their
bitstream correctness, input bounds and arithmetic have not been verified, and
they must not be enabled as an encoder in this state.

The former tile-writing path ignored input samples and did not produce conformant
AV1 video. Header, entropy coding, intra prediction and transform fragments here
are not evidence of a working codec.

Before integrating any part, implement from the public AV1 specification, validate
hostile inputs without panics, and prove pixel reconstruction and bitstream
conformance using external test oracles. The compiled API remains Unsupported.
