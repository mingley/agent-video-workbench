# Browse a source and request a constrained preview

Agent Video Workbench 0.5.0 adds cached, paginated source storyboards and precisely
mapped review previews. These are analysis jobs: originals, editing revisions and
undo history remain intact. An agent or configured provider interprets content.
The workbench does not infer what matters, transcribe automatically, or change a
host's accepted media types, policies or input limits.

## Source-wide storyboard

After importing a source, save a request such as:

```json
{"assetId":"source","sampleEveryMs":10000,"limit":12,"tileWidth":256,"maxBytes":1048576}
```

Run it with the current project revision and a descriptive retry key:

```sh
avw source-storyboard /path/to/project --request /path/to/story.json \
  --expected-revision 1 --key source-story-page-01 --no-launch
avw worker /path/to/project
avw artifact /path/to/project JOB_ID
```

Equivalent JSON/MCP service request:

```json
{"command":"source-storyboard","project":"project","expectedRevision":1,
 "key":"source-story-page-01","noLaunch":true,
 "request":{"assetId":"source","sampleEveryMs":10000,"limit":12}}
```

The verified JSON artifact links `storyboard.png`. Its `data.tiles` pair each
requested sample with the **observed decoded frame PTS and time base**, a rational
`sourceTime`, rounded `sourceTimeMs`, and a tile row/column. The container origin
is retained. Image tiles have no burned timestamp text; pair the image with this
mapping when passing it to an agent. Requested seek times alone are not proof of
which frame was decoded. Missing or inconsistent source clocks are refused.

A page covers at most 24 samples. `nextOffset` advances through the same source,
`startMs`, `endMs` and sampling recipe; null means its last page. The default
scope is the reported video span up to 24 hours. An audio-only container tail
is reported by `reportedVideoEndMsFloor`, `sourceDurationMsFloor` and
`coverage.audioOnlyTailMayRemain`; it does not create phantom video samples.
Use transcript/audio inspection for remaining audio. A longer source reports
`coverage.scopeContinues`. An explicit `endMs` must be within 24 hours.
`sampleEveryMs` accepts 1000–300000, `tileWidth` accepts 128–512 in multiples of
16, and `maxBytes` accepts 16384–2097152. The default page has 12 tiles, 256×144
letterboxed cells and a 1 MiB PNG cap. Oversized images fail with an instruction
to reduce the page or tile size. They are never silently truncated.

Imported active transcript cues appear under `data.transcript`, with identity,
provider/language, original cue bounds and truncation flags. At most 20 cues and
512 characters per cue are returned per page. A missing or ambiguous transcript
is explicitly unavailable. Updating the active transcript invalidates its cached
context; edits to unrelated sequences do not invalidate source-only recipes.

To receive the image through the existing MCP `avw-media` tool, use:

```json
{"kind":"storyboard","project":"project","jobId":"JOB_ID"}
```

It returns the verified PNG plus its timestamp/coverage metadata. The CLI/JSON
`agent-media` equivalent accepts this object under `media` (or through the named
alias's request file). CLI media extraction requires `--workspace ROOT` containing the project;
the MCP server root already scopes its paths. For example:
`avw --workspace /path/to/workspace agent-media --request /path/to/media.json`.
Raw PNG content is capped at 2 MiB. Base64 transport adds
about one third plus JSON; model image/token charges are host-specific. Paginate
instead of attaching every page at once. Host image support is still required.

## Budget-aware preview

Save a separate request using original-source coordinates:

```json
{"assetId":"source","startMs":15300,"endMs":25300,"maxDurationMs":10000,
 "maxBytes":1048576,"width":640,"height":360}
```

```sh
avw budget-preview /path/to/project --request /path/to/preview.json \
  --expected-revision 1 --key source-window-01 --no-launch
avw worker /path/to/project
avw artifact /path/to/project JOB_ID
```

The JSON/MCP service command is `budget-preview`, with the same envelope as
`source-storyboard` and this request object. Both commands can also be submitted
through `analyze-start` tasks with `kind: "storyboard"` or `"budget-preview"`.
CLI, JSON and MCP queue the same frozen jobs and perform the same validation.

`maxDurationMs` is 1–60000; the requested interval must fit it. `maxBytes` is
16384–67108864 and dimensions are even values in 64–1280. Dimensions are bounds,
not permission to upscale. Encoding tries up to three resolution/CRF settings,
retaining aspect ratio and source frame order/timing. `data.attempts` discloses
the settings and measured sizes. If all attempts exceed the byte cap, the job
fails without a successful artifact. Shorten the interval or raise the budget.
The returned MP4 is a Rec.709 SDR H.264/AAC review aid, not a replacement master.

Every successful preview measures **actual container duration and file bytes**,
fully decodes the output, and checks a one-to-one timestamp mapping against
source frames. `frameMapping` contains original and proxy PTS, with both time
bases and the source container origin. `sourceMinusProxyTime` maps proxy time
back to the original source clock; the requested range and observed first/last
source frame starts are separately reported. Frame PTS are paired in presentation
order, preserving VFR rather than inventing average-fps numbering. A two-second
source-probe guard accounts for normal B-frame reordering. Unusual gaps or
reordering that prevent verification are refused instead of guessed.

Strict duration caps can refuse an otherwise reasonable cut: frame quantization
or audio/container timing may produce a result slightly longer than the cap.
Choose a slightly shorter range or a larger cap. The tool does not silently cut
off requested footage to meet a cap, and a CRF/bitrate setting is never treated
as proof of file size. Audio is re-encoded; listen across cut boundaries.
Short WAV excerpts from these jobs also work through `avw-media audio-proxy`.
Video is retrieved as a verified attachment for the host's own file/video tools;
this does not add a native video content block to MCP.

## Coverage, reuse and edits

A storyboard is sparse navigation evidence. A brief flash, small text, gesture
or spoken word may occur between samples or disappear at thumbnail resolution.
Its coverage explicitly says continuous video was not inspected. Request denser
pages, original-resolution frames/crops, `inspect` scenes/silence, transcript
search and native-frame-rate window previews as appropriate. A scene score is a
visual change heuristic, not semantic understanding or proof of completeness.

For a 10-second host cap with context, choose an 8-second core plus up to one
second on each side; the **entire** requested window must fit the cap. Keep
context overlaps distinct from desired edits. Use the original asset ID and
source ranges in `plan-edit`/`assemble`; do not reimport previews as new masters.
Retain source protections and speech-safe handle review.

Cache keys bind source bytes, clock/color metadata, request budgets, backend and
view adapter; storyboard context also binds its active transcripts. Cache reuse
revalidates originals and output hashes. Changed artifacts are refused. Jobs are
cancellable and retain the existing worker deadline/resource limits. A page can
fail if its requested interval has no decodable frame or a reliable clock.
Generated-media checks establish tested cases, not arbitrary camera/host support.

[Implementation evidence and benchmark](implementation-status.md#source-browsing-and-constrained-review-media-050)
records generated-media checks and their limits.
