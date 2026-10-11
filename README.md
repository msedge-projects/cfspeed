# cfspeed

A TUI speed test against Cloudflare's public speed-test endpoints.

![license](https://img.shields.io/badge/license-MIT-blue.svg)
![rust](https://img.shields.io/badge/rust-1.81%2B-orange.svg)

## Why

Most terminal speed tests wrap a box-drawing border around a single number.
This one is built around the graph:

- **Braille area charts.** Each cell carries 2x4 dots, so the charts have 8x the
  vertical resolution of a character sparkline. Curves read as curves, not
  staircases.
- **Live streaming.** Throughput is sampled every 200ms while the test runs, so
  you watch the line draw itself instead of waiting for a summary.
- **No boxes.** Hierarchy comes from colour weight and whitespace, not chrome.
- **Honest numbers.** Uploads are counted only once the server acknowledges
  them; a `send()` that resolves on response headers cannot over-count.

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

- Download and upload throughput, sampled live and summarised (avg, p50, p95, max)
- Idle latency, plus latency under load (the bufferbloat signal)
- Jitter, packet loss, DNS resolution time
- Colocation, ISP and transport from `cdn-cgi/trace`

Grades are waveform-style: bufferbloat `A+`..`F` from latency added under load,
stability `A`..`F` from throughput variation.

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