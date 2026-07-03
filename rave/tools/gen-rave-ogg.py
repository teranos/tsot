#!/usr/bin/env python3
"""Placeholder music track for rave.

Generates a mono OGG-source WAV to stdout via wave module — pattern is a
sine sequence C-C-E-C-E at ~120 BPM with rhythm 1-2-2-1-2 (beats), 40 ms
glide between notes, 0.35 Hz amplitude tremolo, gentle attack + release
envelope. Loops for 60 s total. Piped to ffmpeg + libvorbis to encode as
OGG in the Makefile.

Placeholder while there's no real music track. Real music takes over the
moment `rave/assets/music/rave.ogg` is replaced by hand or by whatever
future audio-sharing pipeline lands.
"""

import math
import struct
import sys
import wave

SAMPLE_RATE = 44100
NOTES = [
    (261.63, 0.5),  # C, 1 beat
    (261.63, 1.0),  # C, 2 beats
    (329.63, 1.0),  # E, 2 beats
    (261.63, 0.5),  # C, 1 beat
    (329.63, 1.0),  # E, 2 beats
]
LOOPS = 15  # ~60 s of audio


def synthesize() -> list[int]:
    samples: list[int] = []
    phase = 0.0
    prev_freq = NOTES[-1][0]
    for _ in range(LOOPS):
        for freq, dur in NOTES:
            n = int(SAMPLE_RATE * dur)
            glide = int(SAMPLE_RATE * 0.04)
            for i in range(n):
                t = i / SAMPLE_RATE
                # 40 ms glide from previous note's pitch to this one
                if i < glide:
                    f = prev_freq + (freq - prev_freq) * (i / glide)
                else:
                    f = freq
                phase += 2 * math.pi * f / SAMPLE_RATE
                # Envelope: 20 ms attack, 60 ms release
                env = 1.0
                if t < 0.02:
                    env = t / 0.02
                elif t > dur - 0.06:
                    env = max(0.0, (dur - t) / 0.06)
                # Slow amplitude tremolo at 0.35 Hz
                lfo = 0.7 + 0.3 * math.sin(2 * math.pi * 0.35 * t)
                amp = 0.35 * env * lfo
                samples.append(int(amp * math.sin(phase) * 32767))
            prev_freq = freq
    return samples


def main() -> None:
    if len(sys.argv) != 2:
        print("usage: gen-rave-ogg.py <output.wav>", file=sys.stderr)
        sys.exit(2)
    out = sys.argv[1]
    samples = synthesize()
    with wave.open(out, "w") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SAMPLE_RATE)
        w.writeframes(struct.pack(f"<{len(samples)}h", *samples))
    print(f"{out}: {len(samples)} samples, {len(samples) / SAMPLE_RATE:.1f}s")


if __name__ == "__main__":
    main()
