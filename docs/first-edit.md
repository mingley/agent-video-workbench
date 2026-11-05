# Make and deliver your first edit

This walkthrough targets **0.4.0** and creates two independent,
captioned five-second outputs. The same JSON requests work in the MCP `avw`
tool. Start with [installation](deployment.md); no speech model is required.
Use a checkout for the request example files below; the installed binary needs
no build. The automated ID extraction below uses the updated source CLI;
explicit `jq` alternatives are provided for the published 0.4.0 binary.
You can also download the three JSON examples and substitute their
local paths in the request commands.

## Prepare a fresh workspace

Choose a recording at least ten seconds long and a licensed TTF font containing
the characters you will caption. The example keeps source intervals `[0, 2)`
and `[7, 10)` seconds. These are example cuts, not an input-duration limit.
Longer originals are accepted; each rendered output is limited to one hour.
Replace the sample text with reviewed text from your
recording. Copy inputs into a persistent workspace outside the repository.

Run the following in one Bash session, replacing the absolute paths:

```bash
avw_binary=/absolute/path/to/avw
avw_workspace=/absolute/path/to/new-workspace
avw_ffmpeg=/absolute/path/to/ffmpeg
avw_ffprobe=/absolute/path/to/ffprobe
avw_checkout=/absolute/path/to/agent-video-workbench

mkdir -p "$avw_workspace/inputs"
cp /absolute/path/to/recording.mp4 "$avw_workspace/inputs/source.mp4"
cp /absolute/path/to/licensed-font.ttf "$avw_workspace/inputs/font.ttf"

avw() {
  "$avw_binary" --workspace "$avw_workspace" \
    --ffmpeg "$avw_ffmpeg" --ffprobe "$avw_ffprobe" "$@"
}

avw doctor
avw create project --name "First edit"
avw import project inputs/source.mp4 --id source \
  --expected-revision 0 --key first-source-import
avw import project inputs/font.ttf --id font \
  --expected-revision 1 --key first-font-import
```

Require `ok: true` for each command; `doctor` also needs `result.ready: true`.
Stop on an error before issuing the next mutation. The project is now at
revision **2**. The revision numbers below apply to this fresh project only.
For an existing project, read `resume` and rebuild requests from its current
revision and IDs. Asset and output IDs must be distinct.

Use a descriptive **8–200 ASCII character** `--key` (for example,
`first-source-import`). Operation batches require at least eight bytes; some
workflow operations also enforce the 200-byte maximum. Reuse the exact key only
when retrying the same request. A new edit needs a new key.

## Import reviewed captions and compose

The [current request examples](../examples/requests/README.md) are complete
service envelopes. Edit the transcript example's text if using your own
recording, then run:

```bash
avw request "$avw_checkout/examples/requests/transcript-import.json"
avw request "$avw_checkout/examples/requests/compose.json"
avw request "$avw_checkout/examples/requests/variant.json"
avw resume project
```

These mutations advance the project to revisions **3**, **4** and **5**.
`short` is 360×640; `square` is 360×360. Both retain the same five seconds of
source footage and source-linked captions, at 30 fps. The small canvas makes
the trial quick; `compose` defaults to 1080×1920 when dimensions are omitted.
The empty `seq_main` created by `create` is separately 360×640. `compose` creates
a new named sequence and does not inherit that canvas. `assemble` instead uses
source geometry unless explicitly overridden. These defaults do not resize the
original media. The square variant copies revision 4 and can be edited independently.

Caption cues and cuts use half-open source intervals in milliseconds. A cue
intersecting a cut is shortened to that retained interval; review its wording
at cut boundaries. Source timestamps use the container origin, including any
nonzero stream starts: source time zero is the container start, not raw PTS
zero. Internal times are rational; output timing follows the
30 fps grid. Importing captions does not modify the source audio.

## Freeze, render and verify both outputs

Queue a batch at revision 5 without launching a detached process:

```bash
avw_batch_id=$(avw --result-pointer /id batch-start project --sequences short,square \
  --expected-revision 5 --key first-delivery-batch --no-launch) || exit 1
avw worker "$avw_workspace/project"
```

The variable captures the batch ID without copying it by hand. `--result-pointer`
is available in the updated source build; for the published 0.4.0 binary, capture
the normal JSON response instead (enable Bash `pipefail` to retain command errors):

```bash
set -o pipefail
avw_batch_id=$(avw batch-start project --sequences short,square \
  --expected-revision 5 --key first-delivery-batch --no-launch | jq -er '.result.id') || exit 1
```

The worker uses an
**absolute project path** and drains the queue before exiting by default.
It uses the backend frozen into each job. Queuing/rendering leaves the editing
revision at 5. On a long-running host, supervised workers can instead use
`--idle-seconds 60`; see [restart behavior](deployment.md#persistence-maintenance-and-rollback).

Inspect the captured batch:

```bash
avw batch-status project "$avw_batch_id"
```

Require `result.complete: true`, `result.succeeded: 2`, and both child jobs in
`result.items` to have `state: "succeeded"`. Extract a child's `id` and retrieve
its verified artifact and contact sheet:

```bash
avw_child_id=$(avw --result-pointer /items/0/id batch-status project "$avw_batch_id") || exit 1
avw artifact project "$avw_child_id"
avw artifact project "$avw_child_id" --sheet
```

For the published binary, use
`avw_child_id=$(avw batch-status project "$avw_batch_id" | jq -er '.result.items[0].id') || exit 1`
instead of `--result-pointer`.

Check `result.verified`, `result.revision`, `result.sha256` and `result.path`.
Each MP4 should contain 150 decoded video frames. The manifest records technical
verification, matching SRT/VTT, a cover and an indexed sheet. Playback and
editorial review still matter: technical checks cannot approve the story,
transcription, crop or color appearance.

## Package delivery and reopen

The delivery destination must be a new directory:

```bash
avw delivery project "$avw_batch_id" first-delivery
avw resume project
avw backup project first-backup
avw backup-restore first-backup restored-project
avw resume restored-project
avw verify-project restored-project
```

Open `first-delivery/index.html` with a local browser or your host's file tools.
The bundle contains verified videos, sidecars, covers, sheets and a delivery
manifest. Failed batch items are recorded in that manifest; the HTML presents
successful videos. An agent returns the actual files using its host's delivery
tools. The workbench does not upload or publish them.

Backup retains the consistent database and all historical originals, including
the owned font. Derived analysis, renders and review bundles are excluded;
keep delivery separately. The restored project remains editable at revision 5,
but derived jobs are `unavailable`. Queue fresh render jobs with a **new key**
to regenerate outputs; this backup does not resume an interrupted batch.

## Continue through MCP

Point [the example MCP configuration](../examples/mcp.json) at the same binary,
backend and workspace. Send each JSON envelope as arguments to the `avw` tool;
the server root resolves `project`, input and delivery paths. Begin a new agent
session with `agent-guide`, `capabilities` and `resume`.

See the [interface reference](agent-api.md) for response fields, retries and
conflicts, and the [agent guide](../AGENT_GUIDE.md) for selective restoration,
profiles, protection, analysis and feedback.

## Scripting and project identity

The updated source CLI accepts `--result-pointer /id` (a JSON Pointer relative
to `result`): strings print unquoted; numbers, arrays and objects print as JSON.
Errors keep the normal envelope and nonzero exit status. A missing field after a
successful operation returns `E_RESULT_POINTER` with the complete result; the
operation has already succeeded, so do not retry it with a new key. This option
is CLI presentation only; MCP retains its structured response.

New projects identify `agent-video-workbench` and its build version in
`createdBy`. Historical snapshots retain their original metadata. The
`agentcut-project` schema identifier intentionally remains: it describes the
upstream project format that AVW extends, not the application version. Renaming
it would misrepresent compatibility without a schema migration.
