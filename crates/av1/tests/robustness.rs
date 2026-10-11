//! Bounded mutation tests: damaged media must return an error or picture, never unwind.
mod common;
use common::*;
use filmcraft_av1::Decoder;

#[test]
fn truncation_and_entropy_mutations_do_not_panic() {
    let Some(ff) = ffmpeg() else { return };
    let spec = Spec::new("robustness_128", "testsrc2=s=128x128:r=25", 1, "yuv420p", &["-preset", "8", "-crf", "35", "-g", "1"]);
    let path = make(&ff, &spec);
    let frames = ivf_frames(&path);
    let packet = &frames[0];
    let check = |data: &[u8], threads| {
        let result = std::panic::catch_unwind(|| {
            let mut dec = Decoder::with_threads(threads);
            let _ = dec.decode(data);
            let _ = dec.flush();
        });
        assert!(result.is_ok(), "{} byte mutation panicked", data.len());
    };
    for n in 0..packet.len() {
        check(&packet[..n], 1);
    }
    // Preserve the sequence/frame headers so mutations cannot request huge valid images.
    for i in packet.len().saturating_sub(128)..packet.len() {
        for bit in 0..8 {
            let mut mutated = packet.clone();
            mutated[i] ^= 1 << bit;
            check(&mutated, 1);
            if i >= packet.len().saturating_sub(8) {
                check(&mutated, 4);
            }
        }
    }
}
