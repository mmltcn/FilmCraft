//! Headless full-resolution project benchmark. No project writes, audio, mux or output files.
//! cargo run --release -p filmcraft-gpu --example export_bench -- PROJECT START_FRAME FRAMES [gpu|cpu|verify] [none|prores] [WARMUP]
//! Timings are wall-clock stage totals; plan includes source I/O/decode and CPU fallback work.
use filmcraft_frame::{AudioBuffer, VideoFrame};
use filmcraft_media::{FrameRequest, MediaInfo, MediaKind, MediaSource, SharedSource};
use filmcraft_project::ItemId;
use filmcraft_render::{RenderOptions, SourceProvider};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

#[derive(Default)]
struct SourceTimes {
    open: AtomicU64,
    still: AtomicU64,
    decode: AtomicU64,
    errors: AtomicU64,
}
fn add(a: &AtomicU64, t: Instant) {
    a.fetch_add(u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX), Ordering::Relaxed);
}
struct TimedSource {
    inner: SharedSource,
    times: Arc<SourceTimes>,
}
impl MediaSource for TimedSource {
    fn info(&self) -> &MediaInfo {
        self.inner.info()
    }
    fn video_frame(&self, req: FrameRequest) -> filmcraft_media::Result<Arc<VideoFrame>> {
        let t = Instant::now();
        let r = self.inner.video_frame(req);
        add(if self.info().kind == MediaKind::Still { &self.times.still } else { &self.times.decode }, t);
        if r.is_err() {
            self.times.errors.fetch_add(1, Ordering::Relaxed);
        }
        r
    }
    fn audio(&self, start: i64, frames: usize, rate: u32) -> filmcraft_media::Result<AudioBuffer> {
        self.inner.audio(start, frames, rate)
    }
}
struct Sources {
    inner: filmcraft_engine::media_pool::PoolProvider,
    times: Arc<SourceTimes>,
}
impl SourceProvider for Sources {
    fn source(&self, id: ItemId) -> Option<SharedSource> {
        let t = Instant::now();
        let inner = self.inner.source(id);
        add(&self.times.open, t);
        if inner.is_none() || self.inner.pool.offline_status(id).is_some() {
            self.times.errors.fetch_add(1, Ordering::Relaxed);
        }
        inner.map(|inner| Arc::new(TimedSource { inner, times: self.times.clone() }) as SharedSource)
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().ok_or("usage: export_bench PROJECT START_FRAME FRAMES [gpu|cpu|verify] [none|prores] [WARMUP]")?;
    let start: i64 = args.get(1).ok_or("missing start frame")?.parse()?;
    let count: u32 = args.get(2).ok_or("missing frame count")?.parse()?;
    let mode = args.get(3).map(String::as_str).unwrap_or("gpu");
    let encode = args.get(4).map(String::as_str).unwrap_or("none");
    let warmup: u32 = args.get(5).map(|s| s.parse()).transpose()?.unwrap_or(2);
    if start < 0 || !(1..=100_000).contains(&count) || warmup > 1000 || !["gpu", "cpu", "verify"].contains(&mode) || !["none", "prores"].contains(&encode) {
        return Err("invalid range/mode (count 1..100000, warmup 0..1000)".into());
    }
    filmcraft_platform::register();
    filmcraft_render::timing::enable(true);
    let loaded = filmcraft_format::decode(&std::fs::read(path)?)?;
    let seq = loaded
        .view
        .and_then(|v| v.active_sequence)
        .or_else(|| loaded.project.items.iter().find_map(|(id, _)| loaded.project.sequence(*id).map(|_| *id)))
        .ok_or("no sequence")?;
    let project = Arc::new(loaded.project);
    let q = project.sequence(seq).ok_or("missing active sequence")?;
    let rate = q.settings.frame_rate;
    let end = start.checked_add(i64::from(count)).ok_or("range overflow")?;
    if rate.num <= 0 || rate.den <= 0 || end > rate.frame_at(q.duration()) {
        return Err("invalid frame rate or range outside sequence".into());
    }
    filmcraft_project::validate_frame_size(q.settings.width, q.settings.height)?;
    let (w, h) = (q.settings.width, q.settings.height);
    let pool = Arc::new(filmcraft_engine::media_pool::MediaPool::default());
    let times = Arc::new(SourceTimes::default());
    let sources = Sources { inner: pool.full_res_provider(project.clone(), Arc::new(filmcraft_engine::FsServices)), times: times.clone() };
    let adapter = if mode != "cpu" {
        let instance = wgpu::Instance::default();
        let a = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
        format!("{:?}", a.get_info())
    } else {
        "CPU".into()
    };
    eprintln!("adapter: {adapter}");
    let mut gpu = if mode != "cpu" { Some(filmcraft_gpu::ExportRenderer::new().ok_or("no GPU adapter")?) } else { None };
    let mut encoder = (encode == "prores").then(|| filmcraft_prores::Encoder::new(filmcraft_prores::Profile::Lt, w, h));
    let opts = RenderOptions::default();
    let mut render_s = 0.;
    let mut colour_s = 0.;
    let mut encode_s = 0.;
    let mut yuv_s = 0.;
    let mut parity_max_mean = 0.0f64;
    let mut parity_max_p99 = 0u8;
    let mut checksum = 0u64;
    let mut bytes = 0usize;
    for i in 0..warmup {
        let t = rate.tick_of(start + i64::from(i % count));
        let img = if let Some(g) = &mut gpu {
            g.render(&project, seq, t, opts, &sources).ok_or("GPU declined warmup")?
        } else {
            filmcraft_render::render_sequence(&project, seq, t, opts, &sources)
        };
        filmcraft_frame::pool::recycle_f32(img.px);
    }
    for a in [&times.open, &times.still, &times.decode, &times.errors] {
        a.store(0, Ordering::Relaxed);
    }
    filmcraft_gpu::export_renderer::timing::reset();
    filmcraft_render::timing::reset();
    let load_before = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let wall = Instant::now();
    for f in start..end {
        if (f - start) % 10 == 0 {
            eprintln!("frame {}/{}", f - start, count);
        }
        let t = Instant::now();
        let img = if let Some(g) = &mut gpu {
            g.render(&project, seq, rate.tick_of(f), opts, &sources)
                .unwrap_or_else(|| filmcraft_render::render_sequence(&project, seq, rate.tick_of(f), opts, &sources))
        } else {
            filmcraft_render::render_sequence(&project, seq, rate.tick_of(f), opts, &sources)
        };
        render_s += t.elapsed().as_secs_f64();
        let t = Instant::now();
        let rgba = img.over_black_rgba8();
        colour_s += t.elapsed().as_secs_f64();
        filmcraft_frame::pool::recycle_f32(img.px);
        if mode == "verify" {
            let reference = filmcraft_render::render_sequence(&project, seq, rate.tick_of(f), opts, &sources);
            let expected = reference.over_black_rgba8();
            filmcraft_frame::pool::recycle_f32(reference.px);
            if rgba.len() != expected.len() {
                return Err("GPU/CPU dimensions differ".into());
            }
            let mut histogram = [0usize; 256];
            let mut total = 0u64;
            let mut pixels = 0usize;
            for (a, b) in rgba.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0) {
                let d = a[0].abs_diff(b[0]).max(a[1].abs_diff(b[1])).max(a[2].abs_diff(b[2]));
                if let Some(n) = histogram.get_mut(usize::from(d)) {
                    *n += 1;
                }
                total += u64::from(d);
                pixels += 1;
            }
            let mut cumulative = 0usize;
            let p99 = histogram
                .iter()
                .position(|n| {
                    cumulative += n;
                    cumulative > pixels.saturating_mul(99) / 100
                })
                .unwrap_or(255) as u8;
            let mean = total as f64 / pixels.max(1) as f64;
            parity_max_p99 = parity_max_p99.max(p99);
            parity_max_mean = parity_max_mean.max(mean);
            if p99 > 6 || mean >= 1.5 {
                return Err(format!("frame {f}: GPU/CPU parity failed: p99={p99}, mean={mean}").into());
            }
        }
        if let Some(enc) = &mut encoder {
            let t = Instant::now();
            let mut frame = filmcraft_prores::Frame::new(w, h, filmcraft_prores::ChromaFormat::Yuv422, 10, false);
            filmcraft_export::rgba_to_yuv422_10(&rgba, w as usize, h as usize, &mut frame.y, &mut frame.cb, &mut frame.cr);
            yuv_s += t.elapsed().as_secs_f64();
            let t = Instant::now();
            bytes = bytes.saturating_add(enc.encode(&frame)?.len());
            encode_s += t.elapsed().as_secs_f64();
        }
        // Sampled diagnostic checksum, outside the measured render/colour stages.
        for b in rgba.iter().step_by(4093) {
            checksum = checksum.wrapping_mul(1099511628211).wrapping_add(u64::from(*b));
        }
    }
    let elapsed = wall.elapsed().as_secs_f64();
    let g = filmcraft_gpu::export_renderer::timing::get();
    let ms = |a: &AtomicU64| a.load(Ordering::Relaxed) as f64 / 1e6 / f64::from(count);
    let frames = f64::from(count);
    let frame_ms = |d: std::time::Duration| d.as_secs_f64() * 1000. / frames;
    let report = json!({
        "adapter": adapter,
        "mode": mode,
        "encoder": encode,
        "start_frame": start,
        "frames": count,
        "warmup": warmup,
        "size": [w, h],
        "wall_s": elapsed,
        "fps": frames / elapsed,
        "render_fps": frames / render_s,
        "ms_per_frame": {
            "render": render_s * 1000. / frames,
            "source_open_in_render": ms(&times.open),
            "still_frame_in_render": ms(&times.still),
            "decode_in_render": ms(&times.decode),
            "upload_host_in_submit": frame_ms(g.upload),
            "gpu_submit_in_render": frame_ms(g.submit),
            "gpu_wait_readback_in_render": frame_ms(g.map_wait),
            "readback_half_to_float_in_render": frame_ms(g.convert),
            "linear_to_srgb": colour_s * 1000. / frames,
            "rgb_to_yuv": yuv_s * 1000. / frames,
            "encode": encode_s * 1000. / frames
        },
        "cpu_stages_ms": filmcraft_render::timing::get().into_iter()
            .map(|(k, d)| (k, frame_ms(d))).collect::<std::collections::BTreeMap<_, _>>(),
        "cpu_composite_fallbacks": g.cpu_composites,
        "cpu_plan_frames": g.cpu_frames,
        "plan_ms": frame_ms(g.plan),
        "prepare_ms": frame_ms(g.prepare),
        "uploaded_bytes": g.uploaded_bytes,
        "gpu_frames": g.frames,
        "source_errors": times.errors.load(Ordering::Relaxed),
        "checksum": format!("{checksum:016x}"),
        "encoded_bytes": bytes,
        "parity_max_p99": parity_max_p99,
        "parity_max_mean": parity_max_mean,
        "load_before": load_before.trim(),
        "load_after": std::fs::read_to_string("/proc/loadavg").unwrap_or_default().trim()
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    if times.errors.load(Ordering::Relaxed) > 0 {
        return Err("source errors: benchmark invalid".into());
    }
    Ok(())
}
fn main() -> std::process::ExitCode {
    match std::panic::catch_unwind(run) {
        Ok(Ok(())) => std::process::ExitCode::SUCCESS,
        Ok(Err(e)) => {
            eprintln!("export_bench: {e}");
            std::process::ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!("export_bench: rendering panicked");
            std::process::ExitCode::FAILURE
        }
    }
}
