# Hosted agent compatibility and assumptions

October 3, 2026 execution evidence: this Codex cloud workspace runs Debian 13
x86_64 with persistent `/workspace`, a no-root native binary, FFmpeg/Whisper,
CLI editing and actual official MCP stdio integration. The expanded trial
retrieves verified MP4s, analysis and a frozen HTML/sidecar review package, and
reopens a portable project. Native Ubuntu x86_64/ARM64 CI repeats installed-binary
workflows. This qualifies that execution route; the consumer account observations
below remain separate research and do not certify those products.


The intended agent is a hosted product, possibly an OpenAI dot, Grok Bot, or
Meta Muse. None was exercised through an account during this evaluation.
Official documentation establishes useful capabilities, but it does not
establish that our binary can install or render large videos on a given account.
Keep product claims separate from installation tests.

The [distribution and worker specification](specs/distribution-and-workers.md)
expands the proposed install/update, long-job lifetime, remote execution and
compatibility-test contracts. The dated provider observations below remain
research evidence; new product specifications do not constitute host validation.

| Host | What official sources establish | What remains to verify |
| --- | --- | --- |
| OpenAI dot | Own cloud computer for files and software; state can persist between uses; can coordinate configured Codex cloud work and supported plugins | Cloud OS/architecture, arbitrary binary installation, shell permissions for this task, quotas, long process lifetime, GPU, original-video upload/download limits |
| Grok Bot | Persistent cloud computer with command line and shared `/workspace`; files can survive normal updates; desktop video attachments up to 200 MB | Account's execution policy, CPU/RAM/disk and job limits, original attachment access, download egress, GPU, restore/recovery durability for current writes |
| Meta Muse | Dedicated Linux VM; Debian runtime container executes tools/binaries; can compile code and use API/CLI connectors; network egress mediated by Sentinel | Download/install permissions, architecture, workspace retention across runtime replacement, disk/RAM/job quotas, upload limits, GPU, custom MCP support |

Sources inspected October 2, 2026:
[dot computers](https://learn.chatgpt.com/docs/dots/computers-and-apps),
[dot plugin access](https://learn.chatgpt.com/docs/dots),
[Grok computer/files](https://docs.x.ai/grok-bot/computer-and-apps),
[Grok video attachments](https://docs.x.ai/grok-bot/files-and-results),
[Grok computer recreation](https://docs.x.ai/grok-bot/computers), and
[Muse runtime design](https://research.meta.ai/blog/security-and-safety-for-ai-agents-our-approach-with-muse).

Grok documents that manually installed apps/packages can disappear during
computer recreation while the durable disk survives. Therefore keep project
data in a supported persistent workspace, separate from replaceable tools,
and make installation repeatable. A persistent bot identity or conversation
does not itself guarantee persistent files, jobs, or unlimited storage.

OpenAI's developer-hosted sandbox APIs describe their own setup and lifetimes.
Those API details are **not** dot upload or compute limits and are not used as
such here. The plan does not require building an OpenAI API application or
embedding any vendor's agent SDK.

**Prefer installing directly in the bot's computer.** Publish downloadable
Rust CLI archives for Linux x86_64 and aarch64, then macOS arm64/x86_64 and
Windows as supported build targets. The installer needs no sudo, Python,
Node, Cargo, Docker, or GUI. It verifies checksums, uses a user-writable tools
directory, and emits a machine-readable installation result. Ship an explicit
FFmpeg dependency bundle per platform or accept a compatible system build;
never guess that an installed FFmpeg has caption/HDR filters. Include pinned
tool versions and redistribution notices. A container is an optional deployment
format for a user's worker, not a requirement for a hosted bot.

The bot first runs `doctor` and a tiny generated media test. Report actual
OS/architecture, writable persistent project path, binary/filter/encoder
capabilities, available disk, and a successful encode. Try hardware codecs only
through an actual encode probe. Do not require GPU access or advertise hardware
from an encoder name alone. CPU is the correctness baseline.

**Fallback when local installation/execution is restricted:** the same Rust
engine can run on a separate persistent media worker, exposing authenticated
MCP tools over a supported remote transport. This is a media service, not a
conversation/agent harness. Start long work and return a job ID; the bot polls
and downloads small previews. This mode adds hosting/compute/storage costs
and requires that the bot actually supports connecting to such tools. No
confirmed generic remote-MCP route was established for every named consumer
bot. A bot with neither usable shell nor supported custom tools cannot operate
the editor yet. Do not promise universal compatibility through packaging.

**Transfer strategy:** accept original files already present on disk and
authorized HTTPS downloads; return an asset ID after streaming, hashing, and
probing. The hosted agent may use an existing storage connector to materialize
the original locally, then pass its path. A cloud-sharing page is not always a
direct download URL. Handle expired links, authentication, redirects, quotas,
and interrupted downloads explicitly. Keep transient signed URLs out of
durable project exports/logs; store the source hash and stable locator instead.

Direct chat attachments are convenient for small recordings only when the
bot makes their original bytes accessible to software. Large originals should
be sent through an accessible cloud folder or object-storage link. Use bounded
memory and resumable file transfer when the server supports ranges. Return
small preview MP4s/contact sheets in chat; deliver full-resolution final files
via attachments when within limits, otherwise authorized download links.
Check that the phone-sharing method preserved the intended original. Apple
documents preservation of format/resolution/frame rate through iCloud Photos
and possible conversions on other sharing paths:
[Apple HEVC sharing](https://support.apple.com/en-gb/116944).

**Unconfirmed environment targets:** a CPU-only Linux host, 4 vCPU/8 GiB RAM as
an initial benchmark profile, local disk for SQLite and active media, and a
persistent directory large enough for the source, one working derivative,
proxies, and output. These are proposed test conditions, not provider specs
or measured minimums. Derive disk estimates from the actual file/bitrate;
perform space checks before downloads/conversions/renders. For example, a
30-minute 50 Mb/s source is about 11.25 GB decimal before additional artifacts.

The external model must reason over timestamped text and preferably inspect
images/contact sheets. Native video or audio model input is optional. Without
vision, crop quality requires creator review or a framing-safe letterbox;
without audio listening, speech/caption accuracy also requires review. Local
ASR supplies timestamps; an optional remote provider must be configured with
explicit data-transfer/cost policy. Tool JSON should remain small and paginated
even when the source is hours long.

Support targets cover portrait/landscape, SDR/HDR, H.264/HEVC, and path/link
transfer. They do not imply every iPhone codec/profile or every hosted account
was verified. Before implementation depends on a product, its installation
test must persist a project, leave/reopen the conversation, and retrieve a
preview. Repeat after a normal host update where available. Store a dated
compatibility result per product/version instead of assuming it forever.
