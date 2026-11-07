#!/usr/bin/env python3
"""Generate the city/weather ambience used by the viewer.

The files are synthesised rather than sampled so the repository stays small and
the loops are license-free.  Run from the repository root:

    python3 scripts/generate_audio.py
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import wave

import numpy as np

SR = 44_100
OUT = os.path.join(os.path.dirname(__file__), "..", "viewer", "assets", "sounds")
RNG = np.random.default_rng(0x4D414E48)


def fft_shape(n: int, low: float, high: float, slope: float = 1.0) -> np.ndarray:
    freqs = np.fft.rfftfreq(n, 1.0 / SR)
    shape = 1.0 / (1.0 + (freqs / max(high, 1.0)) ** (2.0 * slope))
    shape *= (freqs / max(low, 1.0)) ** 2 / (1.0 + (freqs / max(low, 1.0)) ** 2)
    shape[0] = 0.0
    return shape


def coloured_noise(n: int, low: float, high: float, slope: float = 1.0) -> np.ndarray:
    spec = np.fft.rfft(RNG.normal(size=n))
    out = np.fft.irfft(spec * fft_shape(n, low, high, slope), n=n)
    std = out.std()
    return out / std if std > 1e-9 else out


def stereo(n: int, left: np.ndarray, right: np.ndarray | None = None) -> np.ndarray:
    right = left if right is None else right
    return np.stack([left, right], axis=0)


def normalise(audio: np.ndarray, peak: float = 0.82) -> np.ndarray:
    peak_now = float(np.max(np.abs(audio)))
    return audio * (peak / peak_now) if peak_now > 1e-9 else audio


def loopify(audio: np.ndarray, seconds: float = 0.8) -> np.ndarray:
    n = audio.shape[1]
    fade = min(int(seconds * SR), n // 3)
    if fade <= 1:
        return audio
    out = audio[:, : n - fade].copy()
    ramp = np.linspace(0.0, 1.0, fade, dtype=np.float32) ** 2
    out[:, :fade] = (
        audio[:, n - fade :] * (1.0 - ramp)
        + audio[:, :fade] * ramp
    )
    return out


def write_ogg(name: str, audio: np.ndarray, *, loop: bool = False) -> None:
    if loop:
        audio = loopify(audio)
    audio = normalise(audio)
    pcm = np.clip(audio.T, -1.0, 1.0)
    pcm = (pcm * 32767.0).astype("<i2")
    os.makedirs(OUT, exist_ok=True)
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp:
        wav_path = tmp.name
    with wave.open(wav_path, "wb") as handle:
        handle.setnchannels(2)
        handle.setsampwidth(2)
        handle.setframerate(SR)
        handle.writeframes(pcm.tobytes())
    ogg_path = os.path.join(OUT, f"{name}.ogg")
    subprocess.run(
        [
            "ffmpeg",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            wav_path,
            "-c:a",
            "libvorbis",
            "-q:a",
            "4",
            ogg_path,
        ],
        check=True,
    )
    os.unlink(wav_path)
    print(f"{ogg_path}: {os.path.getsize(ogg_path) / 1024:.0f} KiB")


def ambient_city() -> np.ndarray:
    n = int(14 * SR)
    t = np.arange(n) / SR
    rumble = coloured_noise(n, 18, 180, 1.2)
    air = coloured_noise(n, 120, 950, 0.8)
    hum = 0.10 * np.sin(2 * np.pi * 55.0 * t) + 0.05 * np.sin(2 * np.pi * 110.0 * t)
    swell = 0.55 + 0.25 * np.sin(2 * np.pi * 0.031 * t + 0.4)
    slowly = 0.7 + 0.3 * np.sin(2 * np.pi * 0.017 * t + 1.1)
    left = rumble * 0.55 * slowly + air * 0.16 + hum * 0.22
    right = coloured_noise(n, 20, 210, 1.15) * 0.55 * slowly + air * 0.17 + hum * 0.20
    left += coloured_noise(n, 300, 1800, 0.5) * 0.025 * swell
    right += coloured_noise(n, 320, 1900, 0.5) * 0.024 * swell
    return stereo(n, left, right)


def wind() -> np.ndarray:
    n = int(9 * SR)
    t = np.arange(n) / SR
    gust = 0.45 + 0.28 * np.sin(2 * np.pi * 0.13 * t) + 0.14 * np.sin(2 * np.pi * 0.037 * t + 1.3)
    left = coloured_noise(n, 90, 2400, 0.55) * gust
    right = coloured_noise(n, 100, 2600, 0.55) * gust
    # A little low rush under the hiss.
    left += coloured_noise(n, 30, 140, 1.2) * 0.25
    right += coloured_noise(n, 30, 140, 1.2) * 0.23
    return stereo(n, left, right)


def traffic() -> np.ndarray:
    n = int(8 * SR)
    t = np.arange(n) / SR
    rumble = coloured_noise(n, 24, 220, 1.2)
    tyre = coloured_noise(n, 350, 1800, 0.7)
    pass_by = 0.55 + 0.30 * np.sin(2 * np.pi * 0.19 * t + 0.2) + 0.15 * np.sin(2 * np.pi * 0.07 * t)
    left = rumble * 0.75 + tyre * 0.18 * pass_by
    right = coloured_noise(n, 26, 240, 1.2) * 0.73 + tyre * 0.17 * pass_by
    return stereo(n, left, right)


def rain() -> np.ndarray:
    n = int(9 * SR)
    t = np.arange(n) / SR
    hiss = coloured_noise(n, 1100, 9000, 0.35)
    body = coloured_noise(n, 250, 2500, 0.5)
    left = hiss * 0.42 + body * 0.20
    right = coloured_noise(n, 1200, 9800, 0.35) * 0.41 + body * 0.21
    # Individual drops: short decaying high-frequency ticks.
    rng = np.random.default_rng(0x5241494E)
    drops = np.zeros(n, dtype=np.float32)
    for _ in range(260):
        at = int(rng.uniform(0, n - 200))
        length = int(rng.uniform(40, 180))
        env = np.exp(-np.linspace(0.0, 7.0, length)) * rng.uniform(0.04, 0.18)
        freq = rng.uniform(1800, 5200)
        drops[at : at + length] += (
            np.sin(2 * np.pi * freq * np.arange(length) / SR) * env
        )
    left += drops
    right += np.roll(drops, 91) * 0.82
    return stereo(n, left, right)


def thunder() -> np.ndarray:
    n = int(7 * SR)
    t = np.arange(n) / SR
    base = coloured_noise(n, 18, 170, 1.4)
    envelope = np.exp(-t / 2.8) * (1.0 - np.exp(-t / 0.12))
    left = base * envelope * 1.1
    right = coloured_noise(n, 18, 190, 1.35) * envelope * 1.0
    # A few sharper cracks inside the rumble.
    rng = np.random.default_rng(0x5448554E)
    for at_s, amp in [(0.20, 0.8), (1.05, 0.45), (2.25, 0.30), (3.7, 0.20)]:
        at = int(at_s * SR)
        length = int(0.5 * SR)
        env = np.exp(-np.linspace(0.0, 8.0, length)) * amp
        click = coloured_noise(length, 80, 2200, 0.8) * env
        left[at : at + length] += click
        right[at : at + length] += np.roll(click, 37) * 0.9
    return stereo(n, left, right)


def horn(f1: float, f2: float, duration: float) -> np.ndarray:
    n = int(duration * SR)
    t = np.arange(n) / SR
    env = np.minimum(1.0, t / 0.025) * np.exp(-np.maximum(t - 0.05, 0.0) / (duration * 0.55))
    tone = (
        np.sin(2 * np.pi * f1 * t)
        + 0.7 * np.sin(2 * np.pi * f2 * t)
        + 0.18 * np.sin(2 * np.pi * f1 * 2.02 * t)
    )
    left = tone * env * 0.7
    right = tone * env * 0.65
    return stereo(n, left, right)


def main() -> None:
    write_ogg("ambient_city", ambient_city(), loop=True)
    write_ogg("wind", wind(), loop=True)
    write_ogg("traffic", traffic(), loop=True)
    write_ogg("rain", rain(), loop=True)
    write_ogg("thunder", thunder())
    write_ogg("horn1", horn(420.0, 520.0, 0.9))
    write_ogg("horn2", horn(350.0, 455.0, 0.7))
    write_ogg("horn3", horn(500.0, 620.0, 1.1))


if __name__ == "__main__":
    main()
