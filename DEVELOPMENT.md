# Develop and run the prototype

The first Rust prototype implements a SQLite revision store and a synchronous
SDR media loop. It pins AgentCut's pure model and renderer to the audited
revision; its best-effort journal is never used as project authority.

Use latest stable Rust (`rustup update stable`), a C compiler for bundled SQLite,
Python 3 for the generated-media regression, FFmpeg/ffprobe 8+ with libx264,
AAC, drawtext/libass, and a licensed local TTF font. No GUI, Node, GPU or model
service is required. Build and verify from this checkout:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --release --locked
python3 evaluation/application_smoke.py \
  --avw "$PWD/target/release/avw" \
  --ffmpeg /path/to/ffmpeg --ffprobe /path/to/ffprobe \
  --font /path/to/font.ttf --output /path/to/new-smoke-directory
```

`--output` must not exist. The regression generates a two-minute source,
imports original bytes and a font, makes a 30-second vertical captioned draft,
restores a source interval, changes caption size/opening, renders 31 seconds,
and undoes/redoes from fresh CLI processes. It checks decoded frame counts,
unchanged input bytes, byte-identical undo output and corrupt-source rejection.
Commands/results stay in the chosen output directory; private media stays
outside Git. Automated synthetic checks do not establish phone appearance.

`avw --help` lists commands. `create DIRECTORY --name NAME` requires a new
directory. `import DIRECTORY SOURCE --id ID --expected-revision N --key KEY`
copies and hashes source bytes. `status DIRECTORY` returns an AgentCut project
snapshot. `apply DIRECTORY --request FILE [--dry-run]` accepts its pinned typed
operation-batch format; the regression is a complete request example.
`render DIRECTORY` produces a unique artifact directory containing the render
plan, verified MP4 and revision/source snapshot manifest. `history`, `diff`,
and `restore` expose immutable revisions. Restoring appends a revision; redo
means restoring the earlier edited revision with a new idempotency key.

Normal command outcomes and argument errors are JSON envelopes. Requests must
supply current revision and an idempotency key. Same request/key returns the
committed response; changed requests using that key conflict. State, history
and request outcome commit in one SQLite transaction, with WAL and FULL sync.
Use persistent local block storage; do not place the SQLite database on object
storage or assume arbitrary network filesystem locking is safe.

The render adapter disables implicit FFmpeg autorotation because the pinned
compiler emits its own display-matrix transforms. HDR HLG/PQ/BT.2020 ingest
is rejected rather than silently converted. This is a prototype: persisted
jobs/cancellation, portable media backups, protected source annotations, ASR,
phone/HDR/VFR acceptance, hosted-bot trials and release packaging are still
roadmap work. Rendering is synchronous; preserve the project directory and
all originals. Never interpret the historical candidate evaluation as an
application acceptance run.
