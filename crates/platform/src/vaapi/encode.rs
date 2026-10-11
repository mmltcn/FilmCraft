//! VA-API as an Export encoder factory.
//!
//! Linux hardware H.264 encode on AMD and Intel is VA-API `EncSlice` (VCN / QSV), not NVENC.
//! [`factory`] is registered in front of NVENC so those GPUs are probed first. A full slice encoder
//! is not in this crate yet: when the driver exposes `EncSlice` we still return `None` so NVENC
//! (NVIDIA) and then the software encoder run. The probe exists so logs name the real encode path
//! instead of only "NVENC: no libnvidia-encode.so.1", and so a later encoder can take the same
//! factory without changing `register()`.
//!
//! The factory never hangs: `vaQueryConfigEntrypoints` is a driver query, not a session.

use filmcraft_export::{ExportSettings, Format, HardwareEncoding, Result, VideoEncoder};
use filmcraft_time::FrameRate;

/// The H.264 side of the VA-API encoder factory (see the module documentation).
pub fn factory(format: Format, _w: u32, _h: u32, _rate: FrameRate, s: &ExportSettings) -> Option<Result<Box<dyn VideoEncoder>>> {
    if s.hardware_encoding != HardwareEncoding::Auto || format != Format::H264 {
        return None;
    }
    #[cfg(target_os = "linux")]
    match super::va::h264_encode_probe() {
        Ok(driver) => {
            log::info!("VA-API H.264 EncSlice present ({driver}); slice encode is not implemented, trying other factories");
            None
        }
        Err(why) => {
            log::info!("VA-API H.264 encode declined: {why}");
            None
        }
    }
    #[cfg(not(target_os = "linux"))]
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_and_non_h264_never_probe() {
        let mut s = ExportSettings { hardware_encoding: HardwareEncoding::Off, ..Default::default() };
        assert!(factory(Format::H264, 640, 360, FrameRate::FPS_24, &s).is_none());
        s.hardware_encoding = HardwareEncoding::Auto;
        assert!(factory(Format::ProRes, 640, 360, FrameRate::FPS_24, &s).is_none());
        assert!(factory(Format::Hevc, 640, 360, FrameRate::FPS_24, &s).is_none());
    }
}
