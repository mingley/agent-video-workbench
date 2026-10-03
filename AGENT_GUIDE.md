# Agent workflow

Call `agent-guide` and `capabilities` first. The JSON request API and the MCP
`avw` tool share one schema returned by `schema`. Paths in MCP are restricted
to the configured workspace. Put source media and projects inside that root.
Never infer success from a process ID; inspect the JSON envelope and job state.

1. `create`: choose a new project directory and name.
2. `import`: import a source and a licensed TTF font with stable caller IDs.
   Each mutation supplies the current `expectedRevision` and a unique `key`.
3. Optionally `transcribe-start`: queue local ASR if configured, poll the job,
   and read its transcript artifact. Review machine text; analysis does not
   change history. `transcript-import` attaches reviewed cues with IDs, startMs,
   endMs and literal text, plus assetId/language/provider. `transcript-search`
   finds source cues. `inspect` offers metadata, source frames, silence and scene
   evidence. Suggestions do not cut automatically.
4. `compose`: provide outputId/name/fontAssetId and an ordered cuts array
   (id, assetId, startMs, endMs). Defaults are 1080x1920, 30 fps and font size 48.
   Imported transcript cues intersecting retained ranges become captions.
   A cue split by a cut may need text review. Each output has independent IDs.
   Captions use a dark stroke and wrap within 80% canvas width, at most three
   lines. If overflow is rejected, split the source cue or reduce fontSize.
5. `render-start`: name the sequence, expectedRevision and key; save the job ID.
   It launches a local worker where process lifetime allows. Poll `job-status`
   until succeeded, failed, cancelled or interrupted. Verification is a separate
   phase; encoding completion alone does not establish success.
6. `artifact`: use the succeeded job ID to get the verified MP4 or sheet path,
   MIME type, content hash and revision. Review the actual output and deliver it
   using the agent host's file tools. No media bytes are embedded in JSON.
7. On resume, `resume` shows outputs, sources, protection, recent edits and jobs.
   Read `history`, `diff`, `transcript-search` or `request-outcome` before editing.
   `apply` accepts the pinned typed batch format and supported operation names.
   `restore` appends an old snapshot as a new revision; redo restores the prior
   edited revision. Protected source coverage remains enforced on restores. `protect` adds an
   exact source-coverage requirement; `unprotect` removes it explicitly in an
   audited revision.
8. `backup` creates a portable project with all historical original bytes.
   Open and render that copied directory to restore editing. Derived artifacts
   are omitted and copied job records are labeled unavailable.

Same key and same request replays its committed outcome. Changed intent needs
another key. On a revision conflict, inspect current state/diff and reconsider
before submitting another edit. `dryRun` commits no state or success outcome.
Lost render-start replies replay the logical job. Job retry keeps the job ID and
increments attempts. Cancellation preserves earlier outputs.

A killed worker leaves a running attempt until the next worker acquires its
owner lock and marks it interrupted. `avw worker PROJECT` performs that
reconciliation; retry then starts a new attempt. A foreground worker can be
supervised by the host. Detached workers are appropriate only when the host
permits background process lifetime. Imports record copying/verifying/ready/terminal stages; `imports` shows recent
records. A new import reconciles abandoned staging under its owner lock. Query
request-outcome and retry the original key after an uncertain import reply.

Local CLI/MCP is single-user filesystem
access, not an unauthenticated Internet service.

Supported output is SDR H.264/AAC. HDR is rejected. Sources with complicated
stream starts, VFR, unusual color, fonts/scripts or real iPhone camera profiles
require review against the documented input matrix. Never claim semantic,
caption accuracy or phone appearance from automated technical QC alone.
