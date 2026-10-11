# cfspeed

A TUI speed test against Cloudflare's public speed-test endpoints.

![license](https://img.shields.io/badge/license-MIT-blue.svg)
![rust](https://img.shields.io/badge/rust-1.81%2B-orange.svg)

## Why

Most terminal speed tests wrap a box-drawing border around a single number.
This one is built around the graph, and laid out like a web dashboard:

- **Braille area charts.** Each cell carries 2x4 dots, so the charts have 8x the
  vertical resolution of a character sparkline. Curves read as curves, not
  staircases, and they carry a gradient fill.
- **Cards, not boxes.** Panels are rounded surfaces with their own background
  and a coloured accent border, borrowed from the Textual/`posting` idiom that
  treats the terminal like a browser rather than a form.
- **Metric cards.** Download and upload sit side by side, each with a circular
  gauge, a headline figure, a meter and its own chart.
- **Stat tiles.** Idle latency, loaded latency, jitter, packet loss,
  bufferbloat, stability and DNS resolve into a row of compact tiles, colour
  graded from `A+` green to `F` red.
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
| `r` | run again |
| `↑` `↓` | move history selection |
| `Shift+H` | redact network details |
| `q` | quit |

## Design notes

The palette is deliberately desaturated rather than the saturated cyan/magenta
that generic dashboards default to, because these colours sit on screen for the
length of a test. Every text token is validated against WCAG AA (4.5:1) on both
surfaces by `tools/contrast_check.py`; active borders use an accent that clears
7:1, and quiet borders are decorative only.

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