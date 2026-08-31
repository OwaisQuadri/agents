#!/bin/sh
# Generates a synthetic fixture video (three solid-color 2s segments: red,
# blue, green) plus a matching transcript with deliberate pauses that fall
# inside the color boundaries. Ground truth is known analytically (segment N
# is exactly [2N, 2N+2) seconds of color N) rather than requiring a manually
# annotated real video. Writes into $1, which the caller is responsible for
# making a process-locally unique path (AGNT-INV-001).
set -eu
OUT_DIR="$1"
mkdir -p "$OUT_DIR"

ffmpeg -y -f lavfi -i "color=c=red:s=64x64:d=2:r=5" -pix_fmt yuv420p "$OUT_DIR/seg0.mp4" >/dev/null 2>&1
ffmpeg -y -f lavfi -i "color=c=blue:s=64x64:d=2:r=5" -pix_fmt yuv420p "$OUT_DIR/seg1.mp4" >/dev/null 2>&1
ffmpeg -y -f lavfi -i "color=c=green:s=64x64:d=2:r=5" -pix_fmt yuv420p "$OUT_DIR/seg2.mp4" >/dev/null 2>&1

printf "file 'seg0.mp4'\nfile 'seg1.mp4'\nfile 'seg2.mp4'\n" > "$OUT_DIR/concat.txt"
ffmpeg -y -f concat -safe 0 -i "$OUT_DIR/concat.txt" -c copy "$OUT_DIR/video.mp4" >/dev/null 2>&1

cat > "$OUT_DIR/video.srt" <<'EOF'
1
00:00:00,000 --> 00:00:01,000
red segment marker

2
00:00:03,000 --> 00:00:03,500
blue segment marker

3
00:00:05,500 --> 00:00:06,000
green segment marker
EOF
