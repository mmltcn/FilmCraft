//! Test packaging encoded AV1 frames into an ISOBMFF / MP4 container.

use std::io::Cursor;

use filmcraft_av1enc::{Encoder, EncoderConfig, YuvFrame};
use filmcraft_isobmff::{Av1Config, Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};

#[test]
fn test_av1enc_mp4_mux_and_decode() {
    let (w, h) = (320u32, 240u32);
    let mut enc = Encoder::new(EncoderConfig::new(w, h, 24, 1)).unwrap();
    let y_plane = vec![128u8; (w * h) as usize];
    let uv_plane = vec![128u8; ((w / 2) * (h / 2)) as usize];
    let frame = YuvFrame { y: &y_plane, u: &uv_plane, v: &uv_plane, y_stride: w as usize, uv_stride: (w / 2) as usize };

    let mut packets = Vec::new();
    for i in 0..5 {
        let p = enc.encode(&frame, i * 1000).unwrap();
        packets.push(p);
    }

    let av1c = Av1Config {
        seq_profile: 0,
        seq_level_idx_0: 8,
        seq_tier_0: false,
        high_bitdepth: false,
        twelve_bit: false,
        monochrome: false,
        chroma_subsampling_x: true,
        chroma_subsampling_y: true,
        chroma_sample_position: 0,
        initial_presentation_delay_minus_one: None,
        config_obus: enc.sequence_header_obu(),
    };

    let sample_entry = SampleEntry::av1(av1c, w as u16, h as u16);
    let track_cfg = TrackConfig::new(sample_entry, 24);

    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Mp4Writer::new(&mut cursor, WriterOptions::new(Brand::Mp4)).unwrap();
    let track_id = writer.add_track(track_cfg).unwrap();

    for p in &packets {
        writer.write_sample(track_id, WriteSample { data: &p.data, duration: 1, is_sync: p.keyframe, composition_offset: 0 }).unwrap();
    }
    writer.finish().unwrap();

    let mp4_bytes = cursor.into_inner();
    assert!(!mp4_bytes.is_empty(), "MP4 bytes must not be empty");

    // Open MP4 and verify demux
    let mp4 = filmcraft_isobmff::open(&mp4_bytes).unwrap();
    let tracks = &mp4.tracks;
    assert_eq!(tracks.len(), 1);
    let trk = &tracks[0];
    assert_eq!(trk.samples.len(), 5);

    // Decode all frames with AV1 decoder
    let mut dec = filmcraft_av1::Decoder::new();
    let mut decoded_pics = Vec::new();
    for s in &trk.samples {
        let sample_data = &mp4_bytes[s.offset as usize..s.offset as usize + s.size as usize];
        decoded_pics.extend(dec.decode(sample_data).unwrap());
    }
    decoded_pics.extend(dec.flush().unwrap());

    assert_eq!(decoded_pics.len(), 5, "Decoded all 5 frames from MP4");
    assert_eq!(decoded_pics[0].width, w);
    assert_eq!(decoded_pics[0].height, h);
    assert_eq!(decoded_pics[0].planes[0][0], 128);
}
