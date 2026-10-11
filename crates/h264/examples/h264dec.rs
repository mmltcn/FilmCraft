//! Decode an Annex-B H.264 file to raw planar YUV (8-bit bytes or 16-bit little-endian samples):
//! `h264dec in.h264 [out.yuv]`.
//! Prints timing information to stderr. `H264_BENCH_ITERS=n` (with `H264_THREADS=t`, and
//! `H264_DRAFT=1` for draft mode) turns it into a benchmark.

use filmcraft_h264::{Decoder, Plane};
use std::io::Write;

/// Append one output plane to the raw stream (u8 samples as bytes, u16 little-endian).
fn put_plane(o: &mut std::io::BufWriter<std::fs::File>, pl: &Plane) {
    match pl {
        Plane::U8(v) => {
            o.write_all(v).unwrap();
        }
        Plane::U16(v) => {
            for &s in v {
                o.write_all(&s.to_le_bytes()).unwrap();
            }
        }
    }
}

/// End offset of the next access-unit boundary in `data` that is at least `min_len` bytes in.
///
/// A chunked feed must not split a NAL unit: the decoder sees a truncated stream and errors. Start
/// codes delimit NAL units in an Annex-B stream, so the boundary is the first start code at or past
/// `min_len`. Falls back to the whole slice when none is found (the last unit).
fn next_au_boundary(data: &[u8], min_len: usize) -> usize {
    let mut i = min_len.min(data.len());
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && (data[i + 2] == 1 || (i + 4 <= data.len() && data[i + 2] == 0 && data[i + 3] == 1)) {
            return i;
        }
        i += 1;
    }
    data.len()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: h264dec in.h264 [out.yuv]");
        std::process::exit(2);
    }
    // The streaming benchmark below opens the file itself, so nothing is read up front.
    // H264_STREAM=1: benchmark by streaming the file in AU-aligned chunks, so peak memory does not
    // depend on clip length. The whole-file path below needs the entire elementary stream resident
    // (354 MB for 20 s of 4K), which is what exhausted memory on machines this was measuring.
    if std::env::var_os("H264_STREAM").is_some() {
        use std::io::Read;
        let n = std::env::var("H264_BENCH_ITERS").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
        let threads = std::env::var("H264_THREADS").ok().and_then(|v| v.parse::<usize>().ok());
        let chunk = std::env::var("H264_CHUNK").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1 << 16).max(4096);
        let inflight = std::env::var("H264_INFLIGHT").ok().and_then(|v| v.parse::<usize>().ok()).or(Some(2));
        let mut best = f64::MAX;
        let mut frames = 0usize;
        for _ in 0..n {
            let mut dec = threads.map(Decoder::with_threads).unwrap_or_default();
            dec.set_draft(std::env::var_os("H264_DRAFT").is_some());
            if let Some(k) = inflight {
                dec.set_max_in_flight(k);
            }
            let mut f = match std::fs::File::open(&args[1]) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("{}: {e}", args[1]);
                    return;
                }
            };
            // Keep one read buffer plus an overlap tail, never the whole file.
            let mut buf = vec![0u8; chunk * 2];
            let mut fill = 0usize;
            let t0 = std::time::Instant::now();
            frames = 0;
            loop {
                let got = match f.read(&mut buf[fill..]) {
                    Ok(0) => 0,
                    Ok(g) => g,
                    Err(e) => {
                        eprintln!("read: {e}");
                        break;
                    }
                };
                fill += got;
                let mut eof = got == 0;
                // Feed every complete access unit, keeping the incomplete tail. If no boundary turns
                // up before the buffer ends, the unit is larger than the buffer: grow it rather than
                // truncating, which would hand the decoder half a slice.
                loop {
                    if fill == 0 {
                        break;
                    }
                    let end = next_au_boundary(&buf[..fill], 1);
                    if end == fill {
                        if eof {
                            frames += dec.decode(&buf[..fill], 0).expect("decode").len();
                            fill = 0;
                        } else if buf.len() < fill * 2 {
                            buf.resize(fill * 2, 0);
                        }
                        if eof || buf.len() < fill * 2 {
                            continue;
                        }
                        // read more into the enlarged buffer before deciding anything
                        match f.read(&mut buf[fill..]) {
                            Ok(0) => eof = true,
                            Ok(g) => fill += g,
                            Err(e) => {
                                eprintln!("read: {e}");
                                eof = true;
                            }
                        }
                        continue;
                    }
                    frames += dec.decode(&buf[..end], 0).expect("decode").len();
                    buf.copy_within(end..fill, 0);
                    fill -= end;
                    if eof && buf.len() > chunk * 2 {
                        buf.truncate(chunk * 2);
                    }
                }
                if eof {
                    break;
                }
                if eof {
                    break;
                }
            }
            frames += dec.flush().len();
            best = best.min(t0.elapsed().as_secs_f64());
        }
        eprintln!("streamed best of {n}: {frames} frames in {best:.3}s ({:.1} fps)", frames as f64 / best);
        return;
    }

    let data = std::fs::read(&args[1]).expect("read input");

    // H264_BENCH_ITERS=n: decode n times without output and report the best run.
    //
    // Pictures are dropped as they arrive instead of collected: a 4K 4:2:2 10-bit frame is 33 MB of
    // `u16` planes, so retaining a 20 s clip would need ~16 GB and OOM the machine. Dropping each
    // `Vec` lets the decoder's own bounded queue govern memory (~its DPB plus in-flight pictures),
    // which makes the benchmark usable on the long 4K clips it exists to measure.
    if let Some(n) = std::env::var("H264_BENCH_ITERS").ok().and_then(|v| v.parse::<usize>().ok()) {
        let mut best = f64::MAX;
        let mut frames = 0usize;
        let threads = std::env::var("H264_THREADS").ok().and_then(|v| v.parse::<usize>().ok());
        // H264_INFLIGHT=n: bound the pictures in flight (see `Decoder::max_in_flight`), which is the
        // other half of a bounded footprint when measuring long clips.
        // Each in-flight picture is a full 4K 4:2:2 10-bit frame (33 MB of u16 planes) and the
        // decoder's output queue holds up to `max_in_flight * 2` of them, so the default
        // `threads + 2` costs ~1 GB at 4K before any output exists. Benchmarking clips of any length
        // wants a bounded footprint more than it wants that extra read-ahead, so cap it here and let
        // `H264_INFLIGHT` raise it for throughput measurements.
        let inflight = std::env::var("H264_INFLIGHT").ok().and_then(|v| v.parse::<usize>().ok()).or(Some(2));
        // H264_CHUNK=n: feed the stream in n-byte pieces across separate `decode()` calls, the way a
        // demuxer does, instead of handing over the whole clip at once. Comparing the two shows
        // whether retained memory comes from the size of one call or from decoding many pictures.
        let chunk = std::env::var("H264_CHUNK").ok().and_then(|v| v.parse::<usize>().ok()).filter(|c| *c > 0);
        for _ in 0..n {
            let mut dec = threads.map(Decoder::with_threads).unwrap_or_default();
            dec.set_draft(std::env::var_os("H264_DRAFT").is_some());
            if let Some(k) = inflight {
                dec.set_max_in_flight(k);
            }
            let t0 = std::time::Instant::now();
            frames = 0;
            match chunk {
                Some(c) => {
                    // Split at access-unit boundaries, not at byte offsets: a chunk that cuts a NAL
                    // unit in half makes the decoder see a truncated stream and error out.
                    let mut off = 0usize;
                    while off < data.len() {
                        let rest = &data[off..];
                        let end = next_au_boundary(rest, c);
                        frames += dec.decode(&rest[..end], 0).expect("decode").len();
                        off += end;
                    }
                    frames += dec.flush().len();
                }
                // `.len()` on the temporary frees that batch before the next call, so retained memory
                // is the decoder's bounded queue rather than every picture the clip has produced.
                None => frames = dec.decode(&data, 0).expect("decode").len() + dec.flush().len(),
            }
            best = best.min(t0.elapsed().as_secs_f64());
        }
        eprintln!("best of {n}: {frames} frames in {best:.3}s ({:.1} fps)", frames as f64 / best);
        return;
    }
    let mut out = args.get(2).map(|p| std::io::BufWriter::new(std::fs::File::create(p).expect("create output")));
    let mut dec = Decoder::new();
    let t0 = std::time::Instant::now();
    let mut n = 0usize;
    let mut write = |pics: Vec<filmcraft_h264::Picture>| {
        for p in pics {
            n += 1;
            if let Some(o) = out.as_mut() {
                put_plane(o, &p.y);
                put_plane(o, &p.u);
                put_plane(o, &p.v);
            }
        }
    };
    match dec.decode(&data, 0) {
        Ok(p) => write(p),
        Err(e) => eprintln!("decode error: {e}"),
    }
    write(dec.flush());
    let dt = t0.elapsed().as_secs_f64();
    if let Some(e) = dec.take_error() {
        eprintln!("decode error: {e}");
    }
    if std::env::var("H264_STATS").is_ok() {
        eprintln!("{:#?}", dec.stats());
    }
    eprintln!("{n} frames in {dt:.3}s ({:.1} fps)", n as f64 / dt);
}
