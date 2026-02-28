# dr-measure

A fast, cross-platform CLI tool written in Rust that calculates the **Dynamic Range (DR)** score
of every FLAC file in a folder and writes a plain-text report.

The algorithm follows the **DR Loudness Standard** (Pleasurize Music Foundation):
<https://www.dynamicrange.de>

---

## Features

- Pure Rust — no external binaries or system libraries required
- Cross-platform: Linux, macOS, Windows
- Recursion-free by design; scans one folder at a time
- Produces a clean, human-readable `dr_report.txt`
- Shows per-track DR, Peak dB, RMS dB, duration, and codec info
- Provides an Album DR summary and a plain-English quality rating

---

## Download

Download the [latest binary release for your platform](https://github.com/alexpilotti/dr-measure/releases) and unzip it.

---

## Usage

```
dr-measure [OPTIONS] [FOLDER]

Arguments:
  [FOLDER]  Folder containing FLAC files [default: .]

Options:
  -o, --output <OUTPUT>  Output report file path [default: <folder>/dr_report.txt]
  -q, --quiet            Suppress console output
  -h, --help             Print help
  -V, --version          Print version
```

### Examples

```bash
# Analyse current directory, write dr_report.txt here
dr-measure

# Analyse a specific album folder
dr-measure "/music/Pink Floyd - The Wall"

# Custom report path
dr-measure ~/music/album -o ~/desktop/wall_dr.txt

# Silent batch use (CI / scripts)
dr-measure ~/music/album --quiet
```

---

## Report Format

```
═══════════════════════════════════════════════════════════════════════════
  Dynamic Range Report
  Generated : 2025-06-01 14:32:11
  Folder    : /music/Pink Floyd - The Wall
═══════════════════════════════════════════════════════════════════════════

  DR    Peak dB   RMS dB   Duration  Info      File
  ─────────────────────────────────────────────────────────────────────────
  DR13   -0.20    -14.31   05:42     44/16/2   01 - In the Flesh.flac
  DR12   -0.18    -13.89   03:35     44/16/2   02 - The Thin Ice.flac
  ...
  ─────────────────────────────────────────────────────────────────────────

  Summary
  ───────────────────────────────
  Tracks analysed : 26
  Album DR        : DR13
  DR range        : DR11 – DR15

  DR Rating : Good
```

---

## DR Rating Scale

| Album DR | Rating                        |
|----------|-------------------------------|
| DR ≥ 14  | Excellent – wide dynamic range|
| DR 10–13 | Good                          |
| DR 8–9   | Acceptable                    |
| DR 6–7   | Compressed                    |
| DR < 6   | Heavily brick-walled / clipped|

---

## Build

You need [Rust](https://rustup.rs) ≥ 1.70.

```bash
# Clone / copy the project, then:
cd dr-measure
cargo build --release
```

The binary is at `target/release/dr-measure` (or `dr-measure.exe` on Windows).

Optionally install it system-wide:

```bash
cargo install --path .
```

---

## License

AGPLv3
