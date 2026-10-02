# Repository instructions

This repository contains an initial Rust SDR editing prototype, research,
product specifications and evaluation harnesses. Read DEVELOPMENT.md for the
verified prototype scope, then README.md, docs/implementation-status.md,
docs/decision.md and docs/product-plan.md before implementation. Real phone/host
acceptance and most broader capabilities remain open. A later user request can
change scope.

- Use Rust for application logic, project state, interfaces, and job execution.
- Use FFmpeg/ffprobe for media work. Keep the reasoning agent external.
- Preserve original media and source timestamps. Use stable IDs and exact time.
- Keep one authoritative project store; JSON exports are derived snapshots.
- Commit revisions, request outcomes, and history together. Never silently lose undo.
- Give CLI and MCP the same validated operations and structured errors.
- Distinguish documented, source-reviewed, and runtime-tested behavior.
- Keep implementation status and feature acceptance evidence current. Draft
  schemas/examples are not proof of implemented or semantically valid operations.
- Never commit private footage, credentials, expiring private URLs, or model weights.
- Evaluation media must be generated here or carry an explicit redistribution license.
- Store project data separately from downloaded tools and disposable caches.
- Repository commits use name mingley and email michael.ingley@gmail.com.

For documentation changes, check links and git diff --check. For evaluation
changes, run the affected evaluation and inspect its generated output. Future
Rust changes need appropriate tests for their behavior, plus formatting and
lint checks; do not claim an unrun check passed.
