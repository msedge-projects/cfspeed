# cfspeed

A TUI speed test against Cloudflare's public speed-test endpoints.

![license](https://img.shields.io/badge/license-MIT-blue.svg)
![rust](https://img.shields.io/badge/rust-1.81%2B-orange.svg)

## Why

Most terminal speed tests wrap a box-drawing border around a single number.
This one is built around the graph:

- **Smooth curves.** The chart is drawn on a braille sub-pixel grid (2x4 dots
  per cell), giving the edge 8x the vertical resolution of a character cell so
  it traces as a curve instead of stepping. The body beneath is filled on the
  same grid and graded toward the baseline, with a labelled value axis and time
  axis so any reading can be checked against the scale.
- **No bezels.** No cards, borders or boxes. Structure comes from whitespace,
  thin rules and colour weight, so the interface is one continuous surface.
- **The route, always visible.** The main view states plainly who *you* are and
  what *node* you measured against — public IP, ISP, ASN and location on one
  line, Cloudflare edge code and its addresses on the next.
- **Live streaming.** Throughput is sampled every 200ms while the test runs, so
  you watch the chart draw itself instead of waiting for a summary.
- **Honest numbers.** Uploads are counted only once the socket has taken the
  bytes, and download requests shrink automatically when Cloudflare starts
  rate limiting, so the figures mean something.

## Install

Download a release binary, or build from source:

```bash
cargo install --git https://github.com/msedge-projects/cfspeed
```

## Usage

```bash
cfspeed
```

| Key | Action |
| --- | --- |
| `1` `2` `3` `4` | live / charts / history / help |
| `Tab` `Shift+Tab` | cycle views |
| `r` | run again |
| `↑` `↓` / `j` `k` | move history selection |
| `Shift+H` | redact network details |
| `q` `Esc` | quit |

## What it measures

Press `3` for the full spec sheet; every group below is live-updating.

**Throughput** — download and upload, sampled at 200ms and summarised as
avg / min / median / p25 / p75 / p95 / p99 / max, plus total bytes transferred
and the steady-state window those samples cover.

**Latency** — idle, loaded-during-download and loaded-during-upload, each with
avg / min / median / p25 / p75 / p95 / p99 / max, jitter and packet loss.

**Connection** — ISP and ASN, public IP, Cloudflare edge, location, TLS version
with a **real measured handshake time**, negotiated cipher suite, HTTP protocol,
WARP status and DNS resolution time.

**Quality** — bufferbloat (latency added under load) and stability (throughput
coefficient of variation), graded waveform-style from `A+` to `F`.

## Controls

| Key | Action |
| --- | --- |
| `space` | pause / resume the live charts |
| `+` `-` | zoom the chart time window |
| `1` `2` `3` `4` | live · charts · details · history |
| `Tab` `←` `→` | cycle views |
| **mouse** | click a tab to switch view |
| `r` | run again |
| `↑` `↓` | move history selection |
| `Shift+H` | redact network details |
| `q` | quit |

## Design notes

The palette is **Tokyo Night**, taken from btop's own shipped theme
(`themes/tokyo-night.theme`) rather than invented — background `#1a1b26`, cyan
`#7dcfff` for accents, and the green → orange → red ramp btop uses for its
meters. Using a palette that already ships inside a tool people trust beats
picking colours by eye.

Every text token is validated against WCAG AA (4.5:1) on both surfaces by
`tools/contrast_check.py`, which also guards the exact hex values in a unit test
so the palette cannot silently drift from the report.

## Build

```bash
cargo build --release
cargo test
```

## License

MIT. See [LICENSE](LICENSE).

The measurement code is original. It speaks Cloudflare's documented public
speed-test protocol (`/cdn-cgi/trace`, `/__down`, `/__up`) and borrows no code
from other speed-test tools.