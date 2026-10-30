# Interpret bounded scene and silence inspection

Use the current source build to inspect an interval without changing the project:

```json
{
  "command": "inspect",
  "project": "project",
  "assetId": "source",
  "kind": "silence",
  "startMs": 500,
  "endMs": 2000
}
```

The same envelope works through CLI JSON and MCP. `kind` can be `silence` or
`scenes`; intervals are source times relative to the container origin and are
at most five minutes. `inspect` reports observations, not instructions to cut.
Always check `analysisStatus` and `unavailableReason`. An empty `evidence` array
does not prove that a recording contains no silence or scene change.

The selected stream, its reported start and time base, and the container origin
appear under `timing`. A usable source clock is required: if the import lacks
the relevant timestamps or stream identity, inspection returns `unavailable`
without guessed findings. The detectors receive only the requested half-open
window. Event `sourceTimeMs` values are bounded to it; a `silence_end` may equal
the end because it describes the end of a half-open interval.

`coverage.leadingCoverage` and `trailingCoverage` are normally `not-measured`.
Missing samples before, between or after decoded audio are not automatically
classified as silence. `reportedStreamStartMs` is metadata, not measured
coverage. A silence beginning at the window edge may have started earlier. A
`silence_end` can indicate either sound resumption or the end of decoded samples;
`boundaryMayBeTruncated` marks that uncertainty. Listen beyond the boundary.

Before reporting silence, the workbench probes decoded audio frame timestamps
and sample counts in the interval. It rejects gaps or overlaps larger than two
stream time-base ticks plus two audio samples, and reports the observed probe
span under `coverage.audioContinuity`. A stream time base coarser than 10 ms,
missing timestamps/sample counts, incomplete leading or trailing probe coverage,
or a discontinuity produces an explicit unavailable result without silence
findings. A bounded probe can return at most 8 MiB; if that limit is exceeded,
inspect a shorter interval. The frame probe and detector seek independently, so
the observed span and metadata do not certify every decoder behavior or an
unobserved gap at a stream edge. A codec's delay and timestamp rounding can
also limit accuracy; the returned tolerance records the check used.

For scenes, `firstDecodedVideoTimeMs` reports the first in-window frame seen by
the detector. A seek that begins substantially later than expected is marked
unavailable, but this conservative check can also reject sparse or variable-frame-rate
footage. The first decoded frame has no preceding in-window frame for comparison;
a scene score compares successive decoded frames and can bridge a video PTS
gap. Thus a score indicates an observed frame change, not the exact instant a
change happened in missing media. Truncated detector diagnostics make the
analysis unavailable rather than implying complete coverage. Attached album
art is not treated as a video scene stream.

Unknown or unavailable results are not cached, allowing a retry after a transient
probe failure. Completed evidence uses a versioned, source/metadata/tool-bound
cache; older unbounded detector results are not reused. Video and audio originals
remain unchanged. Use source-context playback and editorial review before
deciding whether to cut any material.
