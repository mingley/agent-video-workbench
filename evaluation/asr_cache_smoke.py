#!/usr/bin/env python3
"""Offline ASR cache, legacy repair, asset identity and no-overwrite regression."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("avw", "ffmpeg", "ffprobe", "output"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--baseline-avw")
    args = parser.parse_args()
    root = Path(args.output).resolve()
    root.mkdir(parents=True, exist_ok=False)
    project = root / "project"
    source = root / "speech.wav"
    model = root / "fake-model.bin"
    provider = root / "fake-whisper"
    phrase = root / "phrase.txt"
    calls = root / "provider-calls.txt"
    ledger = []

    def process(argv, payload=None):
        result = subprocess.run(
            list(map(str, argv)),
            input=payload,
            text=True,
            capture_output=True,
            timeout=180,
        )
        assert result.returncode == 0, (argv, result.stdout[-2000:], result.stderr[-2000:])
        return result.stdout

    process([
        args.ffmpeg, "-v", "error", "-f", "lavfi", "-i",
        "sine=frequency=440:sample_rate=16000:duration=2",
        "-c:a", "pcm_s16le", source,
    ])
    model.write_bytes(b"generated fake ASR model")
    phrase.write_text("first asset words", encoding="utf-8")
    provider.write_text(
        "#!/usr/bin/env python3\n"
        "import json, sys\n"
        "from pathlib import Path\n"
        f"root = Path({str(root)!r})\n"
        "prefix = Path(sys.argv[sys.argv.index('-of') + 1])\n"
        "with (root / 'provider-calls.txt').open('a') as f: f.write('called\\n')\n"
        "start = 100\n"
        "if (root / 'measure-onset').exists():\n"
        "    import array, wave\n"
        "    with wave.open(sys.argv[sys.argv.index('-f') + 1], 'rb') as audio:\n"
        "        rate = audio.getframerate()\n"
        "        samples = array.array('h', audio.readframes(audio.getnframes()))\n"
        "    if sys.byteorder != 'little': samples.byteswap()\n"
        "    first = next(i for i, sample in enumerate(samples) if abs(sample) > 200)\n"
        "    start = round(first * 1000 / rate)\n"
        "result = {'transcription': [{'offsets': {'from': start, 'to': start + 500}, "
        "'text': (root / 'phrase.txt').read_text()}]}\n"
        "Path(str(prefix) + '.json').write_text(json.dumps(result))\n",
        encoding="utf-8",
    )
    provider.chmod(0o755)
    model_sha = hashlib.sha256(model.read_bytes()).hexdigest()

    def prefix(binary):
        return [
            binary, "--workspace", root, "--ffmpeg", args.ffmpeg,
            "--ffprobe", args.ffprobe, "--whisper", provider,
            "--whisper-model", model, "--whisper-model-sha256", model_sha,
        ]

    def call(binary, command, **fields):
        request = {"command": command, "project": str(project), **fields}
        raw = process([*prefix(binary), "request", "-"], json.dumps(request))
        response = json.loads(raw)
        ledger.append({"binary": str(binary), "request": request, "ok": response["ok"]})
        (root / "commands.json").write_text(json.dumps(ledger, indent=2) + "\n")
        assert response["ok"], response
        return response["result"]

    def transcribe(binary, asset_id, revision, key):
        job = call(
            binary, "transcribe-start", assetId=asset_id,
            expectedRevision=revision, key=key, noLaunch=True,
        )
        process([*prefix(binary), "worker", project])
        return call(binary, "job-status", id=job["id"])

    def artifact(binary, job):
        return call(binary, "artifact", id=job["id"])

    def call_count():
        return len(calls.read_text().splitlines())

    baseline = args.baseline_avw or args.avw
    if args.baseline_avw:
        assert process([baseline, "--version"]).strip() == "avw 0.5.1"

    call(baseline, "create", name="Generated ASR cache regression")
    call(
        baseline, "import", source=str(source), id="voice_a",
        expectedRevision=0, key="source-voice-a",
    )
    first = transcribe(baseline, "voice_a", 1, "first-transcription")
    assert first["state"] == "succeeded" and not first["result"]["cacheHit"], first
    legacy = Path(first["result"]["path"])
    legacy_manifest = legacy.with_suffix(".manifest.json")
    original = legacy.read_bytes()
    original_hash = hashlib.sha256(original).hexdigest()
    assert json.loads(original)["assetId"] == "voice_a"
    assert legacy_manifest.is_file() and call_count() == 1
    assert first["result"]["sha256"] == original_hash

    # A true 0.5.1 job has no manifest field. Without that optional baseline,
    # remove only that additive field to exercise legacy retention behavior.
    if args.baseline_avw:
        assert "manifest" not in first["result"], first
    else:
        db = sqlite3.connect(project / "project.sqlite")
        result = dict(first["result"])
        result.pop("manifest", None)
        with db:
            db.execute("UPDATE jobs SET result=? WHERE id=?", (json.dumps(result), first["id"]))
        db.close()

    old = time.time() - 120
    for path in (legacy, legacy_manifest):
        os.utime(path, (old, old))
    orphan = project / "analysis" / "expired-unreferenced.manifest.json"
    orphan.write_text("{}", encoding="utf-8")
    os.utime(orphan, (old, old))
    gc = call(args.avw, "cache-gc", graceSeconds=60, dryRun=False)
    assert not gc["dryRun"] and not orphan.exists(), gc
    assert legacy.is_file() and legacy_manifest.is_file(), gc
    assert legacy.read_bytes() == original

    reused = transcribe(args.avw, "voice_a", 1, "matching-asset-reuse")
    assert reused["state"] == "succeeded" and reused["result"]["cacheHit"], reused
    assert Path(reused["result"]["path"]) == legacy
    assert Path(reused["result"]["manifest"]) == legacy_manifest
    assert call_count() == 1 and artifact(args.avw, first)["sha256"] == original_hash

    saved_legacy_manifest = legacy_manifest.read_bytes()
    legacy_manifest.unlink()
    recovery = transcribe(args.avw, "voice_a", 1, "missing-legacy-sidecar")
    if args.baseline_avw:
        assert recovery["state"] == "succeeded" and recovery["result"]["cacheHit"], recovery
        assert legacy_manifest.is_file() and legacy.read_bytes() == original
        assert json.loads(legacy_manifest.read_text())["adapterVersion"] == 2
    else:
        assert recovery["state"] == "failed" and "incomplete" in recovery["error"], recovery
        assert not legacy_manifest.exists() and legacy.read_bytes() == original
        legacy_manifest.write_bytes(saved_legacy_manifest)
    assert call_count() == 1 and artifact(args.avw, first)["sha256"] == original_hash

    call(
        args.avw, "import", source=str(source), id="voice_b",
        expectedRevision=1, key="same-bytes-second-asset",
    )
    phrase.write_text("second asset words", encoding="utf-8")
    second = transcribe(args.avw, "voice_b", 2, "second-asset-transcription")
    assert second["state"] == "succeeded" and not second["result"]["cacheHit"], second
    second_path = Path(second["result"]["path"])
    second_manifest = Path(second["result"]["manifest"])
    second_original = second_path.read_bytes()
    second_manifest_bytes = second_manifest.read_bytes()
    second_transcript = json.loads(second_original)
    assert second_path != legacy and second_transcript["assetId"] == "voice_b"
    assert second_transcript["cues"][0]["text"] == "second asset words"
    assert legacy.read_bytes() == original and call_count() == 2
    again = transcribe(args.avw, "voice_b", 2, "second-asset-cache-hit")
    assert again["state"] == "succeeded" and again["result"]["cacheHit"], again
    assert Path(again["result"]["path"]) == second_path and call_count() == 2

    # A partial v3 pair must fail before invoking a provider or changing bytes.
    second_manifest.unlink()
    missing_manifest = transcribe(args.avw, "voice_b", 2, "missing-v3-manifest")
    assert missing_manifest["state"] == "failed" and "incomplete" in missing_manifest["error"], missing_manifest
    assert missing_manifest["result"] is None and second_path.read_bytes() == second_original
    assert not second_manifest.exists() and call_count() == 2
    second_manifest.write_bytes(second_manifest_bytes)
    retried = call(args.avw, "job-retry", id=missing_manifest["id"], noLaunch=True)
    assert retried["id"] == missing_manifest["id"] and retried["state"] == "queued", retried
    process([*prefix(args.avw), "worker", project])
    retried = call(args.avw, "job-status", id=missing_manifest["id"])
    assert retried["state"] == "succeeded" and retried["attempt"] == 2, retried
    assert retried["result"]["cacheHit"] and second_path.read_bytes() == second_original
    assert call_count() == 2

    held = root / "held-transcript.json"
    second_path.rename(held)
    missing_data = transcribe(args.avw, "voice_b", 2, "missing-v3-transcript")
    assert missing_data["state"] == "failed" and "incomplete" in missing_data["error"], missing_data
    assert not second_path.exists() and second_manifest.read_bytes() == second_manifest_bytes
    held.rename(second_path)
    assert call_count() == 2

    # Every component of a complete manifest is checked, not only its hash.
    good = json.loads(second_manifest_bytes)
    for index, (field, bad) in enumerate([
        ("sourceSha256", "0" * 64), ("provider", {}), ("adapterVersion", 99),
        ("assetId", "wrong-asset"),
    ]):
        altered = dict(good)
        altered[field] = bad
        second_manifest.write_text(json.dumps(altered))
        invalid = transcribe(args.avw, "voice_b", 2, f"bad-manifest-{index}")
        assert invalid["state"] == "failed" and "manifest" in invalid["error"], invalid
        assert second_path.read_bytes() == second_original and call_count() == 2
    second_manifest.write_bytes(second_manifest_bytes)

    # A leaf symlink is not accepted as a managed cache file.
    moved = root / "held-manifest.json"
    second_manifest.rename(moved)
    second_manifest.symlink_to(moved)
    linked = transcribe(args.avw, "voice_b", 2, "symlinked-manifest")
    assert linked["state"] == "failed" and "regular files" in linked["error"], linked
    assert second_manifest.is_symlink() and second_path.read_bytes() == second_original
    second_manifest.unlink()
    moved.rename(second_manifest)

    if args.baseline_avw:
        # Recreating a sidecar requires a matching historically successful job.
        legacy_manifest.unlink()
        db = sqlite3.connect(project / "project.sqlite")
        rows = [
            (job_id,) for job_id, raw in
            db.execute("SELECT id,result FROM jobs WHERE state='succeeded'").fetchall()
            if raw and json.loads(raw).get("path") == str(legacy)
        ]
        assert rows
        with db:
            db.executemany("UPDATE jobs SET state='failed' WHERE id=?", rows)
        db.close()
        no_evidence = transcribe(args.avw, "voice_a", 2, "missing-legacy-evidence")
        assert no_evidence["state"] == "failed" and "incomplete" in no_evidence["error"], no_evidence
        assert not legacy_manifest.exists() and legacy.read_bytes() == original
        db = sqlite3.connect(project / "project.sqlite")
        with db:
            db.executemany("UPDATE jobs SET state='succeeded' WHERE id=?", rows)
        db.close()
        retried = call(args.avw, "job-retry", id=no_evidence["id"], noLaunch=True)
        assert retried["state"] == "queued", retried
        process([*prefix(args.avw), "worker", project])
        retried = call(args.avw, "job-status", id=no_evidence["id"])
        assert retried["state"] == "succeeded" and retried["attempt"] == 2, retried
        assert retried["result"]["cacheHit"] and legacy_manifest.is_file()
        assert legacy.read_bytes() == original and call_count() == 2
        legacy_manifest.unlink()

        # A job with a different queued config or asset cannot authorize repair.
        db = sqlite3.connect(project / "project.sqlite")
        saved_input = db.execute("SELECT input FROM jobs WHERE id=?", (first["id"],)).fetchone()[0]
        db.close()
        for index, (field, value) in enumerate([
            ("assetId", "voice_b"),
            ("modelSha256", "0" * 64),
        ]):
            altered_input = json.loads(saved_input)
            if field == "assetId":
                altered_input["asr"]["assetId"] = value
            else:
                altered_input["asr"]["config"][field] = value
            db = sqlite3.connect(project / "project.sqlite")
            with db:
                db.execute("UPDATE jobs SET input=? WHERE id=?", (json.dumps(altered_input), first["id"]))
            db.close()
            mismatch = transcribe(args.avw, "voice_a", 2, f"legacy-job-mismatch-{index}")
            assert mismatch["state"] == "failed" and "incomplete" in mismatch["error"], mismatch
            assert not legacy_manifest.exists() and legacy.read_bytes() == original
            db = sqlite3.connect(project / "project.sqlite")
            with db:
                db.execute("UPDATE jobs SET input=? WHERE id=?", (saved_input, first["id"]))
            db.close()

        # Historical source identity must match the current original bytes.
        db = sqlite3.connect(project / "project.sqlite")
        saved_snapshot = db.execute("SELECT snapshot FROM revisions WHERE id=1").fetchone()[0]
        altered_project = json.loads(saved_snapshot)
        prior_asset = next(a for a in altered_project["assets"] if a["id"] == "voice_a")
        prior_asset["fingerprint"]["sha256"] = "0" * 64
        with db:
            db.execute("UPDATE revisions SET snapshot=? WHERE id=1", (json.dumps(altered_project),))
        db.close()
        wrong_source = transcribe(args.avw, "voice_a", 2, "legacy-source-mismatch")
        assert wrong_source["state"] == "failed" and "incomplete" in wrong_source["error"], wrong_source
        assert not legacy_manifest.exists() and legacy.read_bytes() == original
        db = sqlite3.connect(project / "project.sqlite")
        with db:
            db.execute("UPDATE revisions SET snapshot=? WHERE id=1", (saved_snapshot,))
        db.close()

        # A modified legacy transcript must not be blessed or overwritten.
        tampered = original + b" "
        legacy.write_bytes(tampered)
        bad_hash = transcribe(args.avw, "voice_a", 2, "legacy-hash-mismatch")
        assert bad_hash["state"] == "failed" and "incomplete" in bad_hash["error"], bad_hash
        assert legacy.read_bytes() == tampered and not legacy_manifest.exists()
        legacy.write_bytes(original)
        legacy_manifest.write_bytes(saved_legacy_manifest)

    assert call_count() == 2
    assert artifact(args.avw, first)["sha256"] == original_hash
    assert Path(artifact(args.avw, second)["path"]).read_bytes() == second_original

    # Diagnostic only: compare a known delayed PCM burst's original decoded PTS
    # with the cue produced from the adapter's extracted WAV. Do not gate this
    # cache test on a timing hypothesis.
    timing = {"status": "unavailable"}
    try:
        delayed = root / "delayed-audio.mkv"
        process([
            args.ffmpeg, "-v", "error",
            "-f", "lavfi", "-i", "testsrc2=s=160x90:r=10:d=4",
            "-itsoffset", "1", "-f", "lavfi", "-i",
            "sine=frequency=440:sample_rate=16000:duration=2",
            "-map", "0:v:0", "-map", "1:a:0", "-c:v", "libx264",
            "-preset", "ultrafast", "-threads", "1", "-c:a", "pcm_s16le",
            "-t", "4", delayed,
        ])
        probe = json.loads(process([
            args.ffprobe, "-v", "error", "-select_streams", "a:0",
            "-show_frames", "-show_format", "-show_entries",
            "frame=best_effort_timestamp_time:format=start_time", "-of", "json", delayed,
        ]))
        original_ms = round(1000 * (
            float(probe["frames"][0]["best_effort_timestamp_time"])
            - float(probe["format"]["start_time"])
        ))
        call(
            args.avw, "import", source=str(delayed), id="delayed_audio",
            expectedRevision=2, key="delayed-audio-import",
        )
        (root / "measure-onset").touch()
        diagnostic = transcribe(args.avw, "delayed_audio", 3, "delayed-audio-observation")
        if diagnostic["state"] == "succeeded":
            cue = json.loads(Path(diagnostic["result"]["path"]).read_text())["cues"][0]
            cue_ms = cue["startMs"]
            timing = {
                "status": "observation-only",
                "declaredDelayMs": 1000,
                "firstDecodedAudioSourcePtsMs": original_ms,
                "providerCueStartMs": cue_ms,
                "cueMinusSourcePtsMs": cue_ms - original_ms,
            }
        else:
            timing = {"status": "unavailable", "reason": diagnostic.get("error")}
    except (AssertionError, OSError, KeyError, IndexError, ValueError) as error:
        timing = {"status": "unavailable", "reason": str(error)[:240]}
    summary = {
        "passed": True,
        "generatedOfflineProvider": True,
        "legacyV2Compatibility": "passed" if args.baseline_avw else "not-exercised",
        "legacySucceededManifestSurvivesGc": True,
        "cacheReuseWithoutProvider": True,
        "duplicateAssetIdentitySeparated": True,
        "partialCacheRefusedWithoutOverwrite": True,
        "failedJobsRetryWithoutReprocessing": True,
        "manifestIdentityChecked": True,
        "symlinkedCacheRefused": True,
        "legacyRecoveryRequiresMatchingEvidence": bool(args.baseline_avw),
        "priorArtifactStillVerifies": True,
        "cacheProviderRuns": 2,
        "delayedAudioObservation": timing,
        "limitations": [
            "Generated audio and fake provider; no speech model accuracy or real-camera audio tested.",
            "Legacy 0.5.1 compatibility is exercised only when --baseline-avw is supplied.",
        ],
    }
    (root / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
