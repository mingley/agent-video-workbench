# Source-view latency research and implementation (0.5.1)

This work targets repeated agent browsing and small review previews. The baseline
is the checksum-verified public Linux x86_64 0.5.0 binary. Measurements use the
same generated 120-second 720p/30fps SDR original with B frames and audio, on the
same host. Five repetitions alternate baseline/candidate order. A cold recipe
means an analysis cache miss on a warm filesystem, not a cold disk.

## Findings and decisions

The subprocess runner checked `try_wait` and slept for 100ms between checks.
Short FFmpeg probes and thumbnail jobs could finish during that sleep. The new
runner waits for child completion on an owned thread and signals a bounded
channel. Cancellation, job lease and deadline checks keep their existing 100ms
polling cadence; an exit wakes the caller immediately. Timeout/cancellation still kill
the owned process group, reap the child and drain bounded output. This uses
standard Rust facilities without a new dependency. See Rust's
[child wait](https://doc.rust-lang.org/std/process/struct.Child.html#method.wait)
and [timed channel receive](https://doc.rust-lang.org/std/sync/mpsc/struct.Receiver.html#method.recv_timeout).

A twelve-frame storyboard previously required twelve FFmpeg extractions and
one PNG assembly process. It now uses three bounded row graphs and one assembly
process. Each input keeps its accurate seek; `trim` retains its first decoded
frame and a named `showinfo` records its original PTS/time base. Timestamps reset
only after that observation, so frames from different source positions align in
the image grid. Cells become RGB before stacking, preserving the earlier PNG
path's pixels. Missing, multiple or ambiguous observed clocks fail explicitly.

The design follows FFmpeg 8's documented
[accurate input seeking](https://github.com/FFmpeg/FFmpeg/blob/n8.0/doc/ffmpeg.texi),
[trim, showinfo and xstack filters](https://github.com/FFmpeg/FFmpeg/blob/n8.0/doc/filters.texi).
It retains accurate decoding from the preceding seek point, rather than assuming
a requested seek timestamp identifies the displayed frame. Sparse source pages
still do not establish continuous or semantic coverage.

Parallel decoding trades memory for latency. Each row opens at most four source
inputs. Sources whose coded area exceeds 1920×1080 pixels use one source decoder
at a time and retain the same page layout. The existing per-child 4GiB address
space limit, deadlines and cancellation remain enforced. This fallback avoids
multiplying large-frame decoder buffers; it is not a claim that arbitrary camera
codecs or resolutions fit that limit.

Preview validation previously decoded video through ffprobe for PTS and then
decoded audio/video through FFmpeg again. It now probes output headers, then
fully decodes both streams once to FFmpeg's
[framecrc format](https://github.com/FFmpeg/FFmpeg/blob/n8.0/doc/muxers.texi).
Raw-video PTS retain the demuxer time base and are paired with original frames.
The source frame probe remains. Merely omitting `-show_frames` is insufficient:
requesting `frame` entries also enables ffprobe decoding, so the output metadata
query omits both. Runtime observers check the actual tool invocations.
Frame CRCs provide the decoded timestamp index; SHA-256 still verifies source
and attachment integrity. Encoding profiles, native frame timing, measured
duration/byte caps and full decode failures retain their existing semantics.

Impossible byte budgets now report the smallest measured attempt, allowing an
agent to revise its interval or byte cap with observed evidence.

The native workflow also avoids duplicate work: internal PRs use the native run
from their pushed head, while fork PRs retain parent-repository qualification.
Changes only to `.github/release-request.json` run the existing publisher without
rebuilding its already-qualified source. Review must still verify the exact head
and successful native jobs. These use GitHub's documented
[job conditions and path filters](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax).

## Repeated A/B results

| Operation | 0.5.0 median | 0.5.1 median | Reduction |
| --- | ---: | ---: | ---: |
| Twelve-frame storyboard, new recipe | 2.312s | 0.902s | 61.0% |
| Same storyboard, cached recipe | 0.190s | 0.099s | 47.6% |
| Ten-second, 300-frame preview, new recipe | 2.315s | 1.940s | 16.2% |
| Same preview, cached recipe | 0.192s | 0.098s | 49.0% |

Timings include enqueue, worker, source/attachment integrity checks and artifact
lookup. They exclude initial source generation/import and independent equality
checks. No other build or render ran concurrently. Decoded pixels, audio and
timestamps match between the binaries in every measured recipe; the original
hash is unchanged. These are faster executions of the same requested media.

The Linux x86_64 host has an AMD EPYC 9V74 CPU, a four-CPU cgroup quota and a
16GiB memory limit. Peak single-process RSS for storyboard work rose from about
33MiB to 105MiB on this fixture; preview peaks were about 45MiB for both binaries.
The `wait4` measurement includes reaped children but is the largest single
process, not simultaneous process-tree RSS. These figures do not establish 4K,
real-camera, GPU, ASR/model or hosted-agent performance.

See [the complete measurements and local acceptance record](../evaluation/service-results/source-view-performance.json).
Reproduce the interleaved comparison with the generated source from
`evaluation/source_views_benchmark.py`:

```sh
python3 evaluation/source_views_performance.py \
  --baseline /path/to/installed-0.5.0/avw \
  --candidate /path/to/new/avw \
  --source /path/to/generated-120s-run/source.mp4 \
  --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe \
  --repetitions 5 --output /path/to/new-comparison-run
```

## Correctness and remaining costs

Local Linux x86_64 checks pass 183 Rust tests, formatting, strict Clippy, schema,
release build and six publication-gate tests. Two existing opt-in tests remain
ignored by the ordinary Rust suite. The source-view media evaluation checks all
24 cells, partial/nonzero-offset pages, the last single tile, VFR/nonzero origins,
HDR, rotation against FFmpeg autorotation, 4K sequential extraction, video-only
previews, actual duration and byte refusals, a measured 250ms audio/light offset,
transcript cache invalidation and tamper rejection. The official MCP SDK,
application exports/undo and worker kill/recover/cancel/retry checks also pass.
Local checks do not establish native ARM64 or publication qualification.

Source SHA-256 rechecks, accurate keyframe-to-target decoding and review encoding
remain real costs. Larger files can take longer even on cache hits because
source integrity is revalidated. The next performance choices should follow
profiles on representative licensed camera footage and target hardware. The
current measurements do not justify changing integrity, dropping frames or
lowering the requested preview quality to obtain smaller timings.
