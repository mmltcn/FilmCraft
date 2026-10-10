//! Reserved API for a future clean-room AV1 software encoder.
//!
//! The former prototype did not encode input samples or produce conformant AV1 tiles.
//! Until a conformance-tested encoder exists, every encoding request returns Unsupported.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid AV1 encoder configuration: {0}")]
    Invalid(&'static str),
    #[error("software AV1 encoding is not implemented; use a registered hardware encoder")]
    Unsupported,
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub qindex: u8,
    pub key_int: u32,
}
impl EncoderConfig {
    pub fn new(width: u32, height: u32, fps_num: u32, fps_den: u32) -> Self {
        Self { width, height, fps_num, fps_den, qindex: 100, key_int: 30 }
    }
}
pub struct YuvFrame<'a> {
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub y_stride: usize,
    pub uv_stride: usize,
}
/// No public constructor succeeds until software encoding passes conformance tests.
#[derive(Debug)]
pub struct Encoder {
    _private: (),
}
impl Encoder {
    pub fn new(config: EncoderConfig) -> Result<Self> {
        if config.width == 0
            || config.height == 0
            || config.width > 8192
            || config.height > 8192
            || u64::from(config.width) * u64::from(config.height) > 8192 * 4320
            || !config.width.is_multiple_of(2)
            || !config.height.is_multiple_of(2)
        {
            return Err(Error::Invalid("dimensions must be even and within the 8K limit"));
        }
        if config.fps_num == 0 || config.fps_den == 0 || config.key_int == 0 {
            return Err(Error::Invalid("frame rate and keyframe interval must be positive"));
        }
        Err(Error::Unsupported)
    }
    pub fn encode(&mut self, _frame: &YuvFrame<'_>, _pts: i64) -> Result<Vec<u8>> {
        Err(Error::Unsupported)
    }
    pub fn flush(&mut self) -> Result<Vec<u8>> {
        Err(Error::Unsupported)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unfinished_encoder_cannot_emit_corrupt_video() {
        assert!(matches!(Encoder::new(EncoderConfig::new(64, 64, 25, 1)), Err(Error::Unsupported)));
        // Internal construction also cannot bypass the unsupported result.
        let mut e = Encoder { _private: () };
        let frame = YuvFrame { y: &[], u: &[], v: &[], y_stride: 0, uv_stride: 0 };
        assert_eq!(e.encode(&frame, 0), Err(Error::Unsupported));
        assert_eq!(e.flush(), Err(Error::Unsupported));
    }
    #[test]
    fn hostile_configuration_returns_errors_without_panicking() {
        for (w, h, n, d) in [
            (0, 2, 25, 1),
            (2, 0, 25, 1),
            (3, 2, 25, 1),
            (2, 3, 25, 1),
            (8192, 8192, 25, 1),
            (131072, 2, 25, 1),
            (u32::MAX, u32::MAX, 25, 1),
            (64, 64, 0, 1),
            (64, 64, 25, 0),
        ] {
            let result = std::panic::catch_unwind(|| Encoder::new(EncoderConfig::new(w, h, n, d)));
            assert!(matches!(result, Ok(Err(Error::Invalid(_)))));
        }
        let mut c = EncoderConfig::new(64, 64, 25, 1);
        c.key_int = 0;
        assert!(matches!(Encoder::new(c), Err(Error::Invalid(_))));
    }
    #[test]
    fn valid_boundary_configuration_still_reports_unsupported() {
        for (w, h) in [(2, 2), (8192, 4320), (4320, 8192)] {
            assert!(matches!(Encoder::new(EncoderConfig::new(w, h, u32::MAX, u32::MAX)), Err(Error::Unsupported)));
        }
    }
}
