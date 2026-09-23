"""Convert SADIE II (Neumann KU100, "D1") 48 kHz HRIR WAVs into the compact
binary embedded by src-tauri/src/spatial/hrtf.rs.

Source: https://www.york.ac.uk/sadie-project/database.html (Apache-2.0)
Usage:  python3 scripts/convert_hrir.py <D1/D1_HRIR_WAV/48K_24bit> <out.bin>

Layout (little-endian): b"HRIR", u32 sample_rate, u32 taps, u32 count, then per
direction: f32 azimuth_deg, f32 elevation_deg, f32[taps] left, f32[taps] right.
Azimuth follows SOFA: 0 = front, +90 = listener's left.
"""
import os
import struct
import sys
import wave

AZ_STEP = 5
ELEVATIONS = [-45, -30, -15, 0, 15, 30, 45, 60, 75, 90]


def read_wav24(path):
    with wave.open(path, "rb") as w:
        assert w.getnchannels() == 2 and w.getsampwidth() == 3
        rate, n = w.getframerate(), w.getnframes()
        raw = w.readframes(n)
    left, right = [], []
    for i in range(n):
        for ch, dst in ((0, left), (1, right)):
            b = raw[(i * 2 + ch) * 3:(i * 2 + ch) * 3 + 3]
            v = int.from_bytes(b, "little", signed=True)
            dst.append(v / 8388608.0)
    return rate, left, right


def main(src, out):
    entries = []
    for el in ELEVATIONS:
        azs = [0] if el == 90 else range(0, 360, AZ_STEP)
        for az in azs:
            name = f"azi_{az},0_ele_{el},0.wav"
            path = os.path.join(src, name)
            if not os.path.exists(path):
                sys.exit(f"missing {name}")
            rate, l, r = read_wav24(path)
            entries.append((az, el, l, r))
    taps = len(entries[0][2])
    with open(out, "wb") as f:
        f.write(b"HRIR")
        f.write(struct.pack("<III", rate, taps, len(entries)))
        for az, el, l, r in entries:
            f.write(struct.pack("<ff", az, el))
            f.write(struct.pack(f"<{taps}f", *l))
            f.write(struct.pack(f"<{taps}f", *r))
    print(f"{len(entries)} directions, {taps} taps @ {rate} Hz -> {out}")

    # Sanity: a source at +90 (left) must reach the left ear first and louder.
    for az, el, l, r in entries:
        if el == 0 and az in (90, 270):
            onset = lambda x: next(i for i, v in enumerate(x) if abs(v) > 0.1 * max(map(abs, x)))
            energy = lambda x: sum(v * v for v in x)
            print(f"az {az}: onset L={onset(l)} R={onset(r)}  energy L={energy(l):.3f} R={energy(r):.3f}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
