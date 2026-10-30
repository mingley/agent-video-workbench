# Creator-agent opportunities

Status: proposed work, not a release commitment or a list of implemented
features. Priorities are hypotheses until creators have tried the workflow.
The goal is to help an agent turn recordings into useful, reviewable content
while the creator retains control over meaning, appearance and final choices.

The current source build already provides source-matched baselines, exact-time
paper edits, searchable transcript cues, named variants, frozen renders,
feedback, and portable backups. Opt-in [cross-cue search](transcript-search.md)
returns source-bound matches and, when supported, requests for source-context
proxy jobs; the resulting artifacts must still be rendered and verified. See [implementation
status](implementation-status.md), the [agent guide](../AGENT_GUIDE.md), and
the [API reference](agent-api.md) for actual behavior and qualification. The
existing [editor research](editor-research.md) describes relevant editing
workflows observed in official documentation; it is not a competitive benchmark.
Published 0.3.1 and current 0.4.0 source have different capabilities.

## First: learn from complete creator workflows

**Scenario:** A creator asks an agent to turn a recording into two distinct
shorts, then returns later with a precise revision. Existing tests establish
important technical contracts, but generated footage cannot establish whether
the resulting story, captioning or pacing is good for a real audience.

**Acceptance:** Conduct a small, documented trial with consenting creators and
permitted media. Record source/device/host, the creator's request, actions and
tool calls, time to first review, corrections, unsuccessful attempts, which
version the creator preferred and why, and whether the requested revision was
completed after reopening. Include a source-preserving baseline and an agent's
candidate; distinguish technical checks from the creator's opinion. Store
private footage and identifying details outside Git; publish only agreed,
non-sensitive findings. Include a physical display and real phone recording
before claiming phone appearance, and record missing samples as gaps.

**Dependency:** A suitable creator, host and footage. Do not treat the generated
media regressions or a single agent demo as a substitute for this trial. Use the
observations to reorder the items below rather than assuming every creator has
the same workflow.

## P1 — Broaden phrase discovery across scripts and uncertain speech

**Scenario:** “Find the part where I explain why we changed the packaging.”
Opt-in `acrossCues` now performs a bounded literal search over adjacent,
nonoverlapping cues from one source and supplies source/transcript identities,
whole-cue times and a scoped preview request where available. It lowercases and
normalizes whitespace, inserts a space at cue boundaries, and is limited to
eight cues. Continuations require the reported project revision. It neither
establishes word alignment nor searches the meaning of an inaccurate transcript.

**Next slice:** Make boundary joining explicit for languages where cue breaks
do not imply spaces, and support documented canonical Unicode equivalence while
preserving source cue IDs. Report when overlaps, gaps or the cue limit prevented
a candidate from being searched as a continuous phrase. Keep the existing
literal, space-separated behavior as the default and keep source playback and
editorial selection with the agent.

**Acceptance:** Licensed or generated multilingual text fixtures include a
non-space-separated phrase split across cues, composed/decomposed accents and
case variants. Explicit join modes find their expected cue spans without joining
different assets, overlapping speakers or distant passages; default-mode
results and revision-pinned pagination remain stable. CLI and MCP agree, and
an unavailable or partial transcript is not reported as proof of absence.
Check real speech and language-specific quality separately before claiming
multilingual search or timing accuracy.

**Dependencies:** Reviewed transcript, source preview jobs and a specified
normalization/join policy. A missing or inaccurate transcript remains incomplete
evidence; the agent still listens before selecting a cut.

## P1 — Explain how two candidate edits differ

**Scenario:** “Show me what changed between the natural baseline and the tighter
opening.” Named variants and a review bundle already exist, but a creator or
agent still has to infer editorial changes from separate timelines and videos.

**Slice:** Produce a read-only, source-based comparison for two explicitly
selected frozen outputs. List retained, newly included, omitted and reordered
source ranges with exact times and stable source IDs; link each finding to the
relevant output time and verified review artifact. Include the source revision
and flag content that cannot be mapped. Keep side-by-side playback optional;
the structured comparison must work through both CLI and MCP.

**Acceptance:** On generated clips, adding, removing, repeating and reordering
source ranges yields the expected source and output coordinates. A repeated
source range is reported as ambiguous where there is no unique placement;
unsupported speeds, transitions, layered footage or missing artifacts are
reported rather than silently matched. The comparison does not mutate either
output, survives later edits because it names frozen revisions, and identifies
its unsupported cases in the result and review bundle.

**Dependencies:** Existing source maps, frozen artifact identities and review
bundles. Start with normal-speed cuts; richer effects and visual judgments need
separate mapping and qualification.

## P2 — Review word alignment without replacing corrected text

**Scenario:** “Keep the complete sentence, remove the false start, and ensure
the captions still match what I said.” Current source transcripts use timed
cues; cue boundaries do not guarantee word-level precision.

**Slice:** Define an optional alignment-provider result containing source hash,
transcript version, word text/IDs, source-time spans, provider identity and
explicit uncertainty. Present alignments as proposals against a selected
transcript, with original audio context, not as an automatic destructive edit.
Allow the agent or creator to accept a reviewed alignment without discarding
existing text corrections; reject incompatible or stale input explicitly.

**Acceptance:** Fixtures cover repeated words, cue-boundary words, overlapping
or out-of-bounds spans, missing words, changed source/transcript and correction
retention. A rejected or cancelled alignment leaves history unchanged; replay
is idempotent and backup retains accepted provenance. Compare alignment against
manually timed, licensed or permitted speech before publishing an accuracy
claim; report language, sample size and errors separately from generated tests.

**Dependencies:** The existing versioned provider boundary, transcript
selection and review workflow. No particular model, language or accuracy is
assumed, and a missing provider must not block ordinary cue-based editing.

## P2 — Check the creator's delivery constraints

**Scenario:** “Make a vertical cut under 45 seconds and a square cut under
100 MB, with captions I can edit.” Existing variants and verified packages make
the files; the constraints are still scattered across agent instructions.

**Slice:** Introduce a versioned, user-supplied delivery brief for each named
output: requested dimensions/aspect, duration and optional maximum bytes,
caption sidecars and color intent. Preflight should distinguish constraints it
can check before rendering from those requiring a verified file. The package
should record pass, fail or unknown for each requested constraint and preserve
successful siblings when one fails. Do not embed unverified platform rules.

**Acceptance:** Exercise one passing and one failing output in a frozen batch,
including an actual file-size check, missing sidecar, mismatched duration and
unknown measurement. The manifest ties each result to its requested brief,
revision and verified artifact; a later project edit cannot alter the package.
CLI/MCP agree, and no result claims that a platform accepted the upload.

**Dependencies:** Existing batch, artifact verification and delivery manifests.
Platform-specific templates would require separately maintained sources and
account-specific tests; direct publication is outside this slice.

## P3 — Prepare a portable batch for another machine

**Scenario:** “Continue my three-output batch on the new computer.” Current
verified backups retain project history and originals, but exclude derived jobs
and exports; restored derived jobs are marked unavailable.

**Slices:** First add a read-only portability report that lists required source,
font, provider and tool identities, expected bytes, optional artifacts and
unavailable work. Then design an opt-in portable bundle for verified completed
artifacts plus a resumable description of unfinished jobs, without treating an
interrupted encoder as a resumable stream.

**Acceptance:** Report missing, changed and unsupported dependencies before
import; on a second clean workspace, verify all included hashes, retain prior
successful outputs, and requeue unfinished work with frozen input identities.
Interrupted transfer, tampering, unavailable providers, retry and duplicate
import must not modify the original project or overwrite a verified artifact.
Record what was actually transferred and measured; do not imply every host is
qualified. See the existing [portable batch target](product-plan.md#beyond-mvp-acceptance-scenarios).

**Dependencies:** Backup/restore, job idempotency and retained artifact policy.
Resume across other operating systems or hosted accounts needs its own tests.

## Validation and sequencing

Each behavior change needs meaningful Rust tests, formatting, strict Clippy and
the affected local media/CLI/MCP checks. Use generated or explicitly licensed
fixtures, compare decoded media where appropriate, and inspect actual results.
Run browser checks for changed review-page behavior. Document exactly which
tests ran and keep the [acceptance matrix](implementation-status.md) current.
Prefer local checks during development to conserve GitHub Actions quota; reserve
native CI and full release qualification for changes that need those results.
