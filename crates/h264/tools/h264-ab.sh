#!/usr/bin/env bash
# A/B: FilmCraft's H.264 decoder vs ffmpeg, same bitstream, same resolution, same frame count.
#
#   crates/h264/tools/h264-ab.sh <clip> [--start 60] [--dur 20] [--iters 3] [--threads "1 4 12"]
#
# Extracts one Annex-B segment with `-c copy` (no re-encode) and times both decoders on those exact
# bytes. ffmpeg is timed with `-benchmark` (`rtime`, which excludes process start and muxing); ours
# is timed internally and reports its fastest run. Both counts of frames come from the same stream,
# so "ms/frame" is directly comparable - that is the number a playback budget (40 ms at 25 fps) is
# spent against.
#
# Memory: ours would retain 33 MB per 4K 4:2:2 10-bit picture if it collected output, so the
# benchmark discards each batch and `H264_INFLIGHT` bounds the pictures in flight. Peak RSS is
# measured for both sides. Never point this at a clip without --dur on a machine you care about.

set -u

CLIP="${1:?usage: h264-ab.sh <clip> [--start S] [--dur S] [--iters N] [--threads \"1 4 12\"]}"
shift
START=0; DUR=20; ITERS=3; THREADS="1 4 12"; FPS=""; CHUNK=65536
while [ $# -gt 0 ]; do
  case "$1" in
    --start)   START="$2"; shift 2 ;;   # accepted for compatibility; not used (see extraction note)
    --dur)     DUR="$2"; shift 2 ;;
    --iters)   ITERS="$2"; shift 2 ;;
    --threads) THREADS="$2"; shift 2 ;;
    --fps)     FPS="$2"; shift 2 ;;
    --chunk)   CHUNK="$2"; shift 2 ;;
    *) echo "unknown option: $1"; exit 2 ;;
  esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DEC="$ROOT/target/release/examples/h264dec"
[ -x "$DEC" ] || { echo "build first: cargo build --release -p filmcraft-h264 --example h264dec"; exit 1; }
command -v ffmpeg >/dev/null || { echo "ffmpeg not found"; exit 1; }

ES="$(mktemp -t h264ab-XXXXXX.264)"
trap 'rm -f "$ES"' EXIT

echo "=============================================================================="
echo "clip   : $CLIP"
echo "segment: start ${START}s, ${DUR}s, ${ITERS} iteration(s)"
ffprobe -v error -select_streams v -show_entries \
  stream=codec_name,profile,width,height,pix_fmt,r_frame_rate -of default=nw=1 "$CLIP" 2>/dev/null | sed 's/^/         /'

# No `-ss`: with `-c copy` it emitted 12 frames for a 20 s request here, and a silently truncated
# segment makes every number below meaningless. Take the segment from the start instead - decode cost
# does not depend on where in the clip it sits - and verify the frame count below.
# `-map 0:v:0 -an` matters: without it `-c copy` muxes the audio track's NAL units into the
# elementary stream, and the chunked feeder then cuts at an audio start code, slicing a video slice
# in half (the decoder reports "CABAC read past end of slice data").
ffmpeg -v error -y -t "$DUR" -i "$CLIP" -map 0:v:0 -an -c copy -bsf:v h264_mp4toannexb -f h264 "$ES" \
  || { echo "extraction failed"; exit 1; }
[ -s "$ES" ] || { echo "empty elementary stream"; exit 1; }

FRAMES=$(ffprobe -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames \
  -of csv=p=0 "$ES" 2>/dev/null | head -1)
# Take fps from the CONTAINER: on a raw Annex-B stream ffprobe reports `r_frame_rate` as the field
# rate (50/1 for a 25 fps clip), which is what made an earlier guard refuse a perfectly good segment.
# `--fps` overrides when even the container is unclear.
if [ -z "$FPS" ]; then
  FPS=$(ffprobe -v error -select_streams v:0 -show_entries stream=avg_frame_rate -of csv=p=0 "$CLIP" 2>/dev/null | head -1)
fi
SIZE=$(( $(stat -c%s "$ES") / 1048576 ))
EXPECT=$(python3 -c "
try:
    n, d = '${FPS:-25/1}'.split('/')
    fps = float(n) / float(d)
except Exception:
    fps = 25.0
print(max(1, int(${DUR} * fps * 0.8)))" 2>/dev/null || echo 0)
echo "         elementary stream: ${SIZE} MiB, ${FRAMES} frames at ${FPS:-?} (expected >= ${EXPECT})"

if [ "${FRAMES:-0}" -lt "${EXPECT:-1}" ]; then
  echo
  echo "REFUSING TO MEASURE: ${FRAMES} frames for a ${DUR}s request (expected >= ${EXPECT} at ${FPS})."
  echo "Pass --fps 25/1 (or the clip's real rate) if that is wrong; a truncated segment would"
  echo "report fixed overhead instead of decode speed."
  exit 1
fi

echo "=============================================================================="
printf "%-8s %-10s %-10s %-10s %-10s %-9s %-9s\n" \
  threads ffmpeg_s ours_s "ours/ff" "ms/frame" "ff_rssMB" "our_rssMB"
echo "-------- ---------- ---------- ---------- ---------- --------- ---------"

for T in $THREADS; do
  # ffmpeg: decode-only rtime + peak RSS, best of ITERS
  FFB=""; FFR=""
  for _ in $(seq "$ITERS"); do
    OUT=$(ffmpeg -v info -threads "$T" -benchmark -i "$ES" -f null - 2>&1 | grep -oE 'rtime=[0-9.]+|maxrss=[0-9]+' | tr '\n' ' ')
    FFB="$FFB$(printf '%s' "$OUT" | grep -oE 'rtime=[0-9.]+' | cut -d= -f2)
"
    FFR="$(printf '%s' "$OUT" | grep -oE 'maxrss=[0-9]+' | cut -d= -f2)"
  done
  FF=$(printf '%s' "$FFB" | grep -E '^[0-9.]+$' | sort -g | head -1)

  # ours: feed in access-unit-aligned chunks, the way a demuxer does. Handing the whole clip to one
  # `decode()` call retains ~1.5 frame-buffers per picture (2.4 GB for 300 frames of 720p); chunked
  # feeding is both far lighter (60 MB) and faster, so this is the honest measurement anyway.
  ERR=/tmp/h264ab-err.$$
  OUR=$(env H264_STREAM=1 H264_BENCH_ITERS="$ITERS" H264_THREADS="$T" H264_CHUNK="$CHUNK" "$DEC" "$ES" 2>"$ERR" \
        | true; grep -oE 'in [0-9.]+s' "$ERR" | grep -oE '[0-9.]+' | head -1)
  OURR=$(env H264_STREAM=1 H264_BENCH_ITERS=1 H264_THREADS="$T" H264_CHUNK="$CHUNK" /usr/bin/time -f '%M' "$DEC" "$ES" 2>&1 >/dev/null \
        | tail -1)
  if [ -z "${OUR:-}" ]; then
    echo "  (threads=$T) our decoder produced no timing. Its output was:"
    sed 's/^/      /' "$ERR" | tail -4
  fi
  rm -f "$ERR"

  RATIO=$(python3 -c "print(f'{$OUR/$FF:.2f}')" 2>/dev/null || echo "?")
  MSF=$(python3 -c "print(f'{$OUR*1000/max($FRAMES,1):.1f}')" 2>/dev/null || echo "?")
  printf "%-8s %-10s %-10s %-10s %-10s %-9s %-9s\n" \
    "$T" "${FF:-?}" "${OUR:-?}" "$RATIO" "$MSF" "${FFR:-?}" "${OURR:-?}"
done

echo
echo "ours/ff  < 1 means our decoder is faster. ffmpeg_s is decode-only (rtime), so this is"
echo "         decoder vs decoder, not process startup."
echo "ms/frame is ours; at 25 fps the playback budget is 40 ms/frame."
