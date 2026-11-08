#!/usr/bin/env python3
"""Optional serial comparison of uncached ASR workers with an offline provider.

This is a diagnostic, not a correctness gate or a speech-model benchmark.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
for name in ("avw", "baseline-avw", "ffmpeg", "ffprobe", "source", "output"):
    parser.add_argument("--" + name, required=True)
args = parser.parse_args()
root = Path(args.output).resolve()
summary = {"available": False, "reason": "measurement not completed"}

def command(argv, payload=None, timeout=180):
    result = subprocess.run(
        list(map(str, argv)), input=payload, text=True, capture_output=True, timeout=timeout,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed ({result.returncode}): {list(map(str, argv))}; "
            f"stderr={result.stderr[-1200:]}; stdout={result.stdout[-1200:]}"
        )
    return result.stdout

def sha256(path):
    with Path(path).open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()

def read_optional(path):
    try:
        return Path(path).read_text().strip()
    except OSError:
        return None

def run_measurement():
    root.mkdir(parents=True, exist_ok=False)
    source = root / "source.mkv"
    shutil.copyfile(args.source, source)
    source_hash = sha256(source)
    if source_hash != sha256(args.source):
        raise RuntimeError("source copy hash mismatch")
    model = root / "model.bin"
    model.write_bytes(b"offline ASR benchmark model")
    model_hash = sha256(model)
    provider = root / "fake-whisper"
    provider.write_text(
        "#!/usr/bin/env python3\n"
        "import json,sys\n"
        "from pathlib import Path\n"
        "p=Path(sys.argv[sys.argv.index('-of')+1])\n"
        "Path(str(p)+'.json').write_text(json.dumps({'transcription':["
        "{'offsets':{'from':100,'to':200},'text':'offline benchmark'}]}))\n",
        encoding="utf-8",
    )
    provider.chmod(0o755)
    def prefix(binary):
        return [binary, "--workspace", root, "--ffmpeg", args.ffmpeg, "--ffprobe", args.ffprobe,
                "--whisper", provider, "--whisper-model", model, "--whisper-model-sha256", model_hash]
    def call(binary, project, kind, **fields):
        request = {"command": kind, "project": str(project), **fields}
        response = json.loads(command([*prefix(binary), "request", "-"], json.dumps(request)))
        if not response["ok"]:
            raise RuntimeError(f"request {kind} failed: {response}")
        return response["result"]
    def prepare(binary, name):
        project = root / ("project-" + name)
        call(binary, project, "create", name="ASR overhead comparison")
        call(binary, project, "import", source=str(source), id="audio",
             expectedRevision=0, key="benchmark-source-" + name)
        job = call(binary, project, "transcribe-start", assetId="audio", expectedRevision=1,
                   key="benchmark-transcription-" + name, noLaunch=True)
        return binary, project, job["id"]
    def execute(entry):
        binary, project, job_id = entry
        start = time.perf_counter()
        command([*prefix(binary), "worker", project])
        elapsed = time.perf_counter() - start
        status = call(binary, project, "job-status", id=job_id)
        if status["state"] != "succeeded" or status["result"]["cacheHit"]:
            raise RuntimeError(f"expected successful uncached job: {status}")
        transcript = json.loads(Path(status["result"]["path"]).read_text())
        cues = [(cue["startMs"], cue["endMs"], cue["text"]) for cue in transcript["cues"]]
        return elapsed, cues
    binaries = {"baseline": args.baseline_avw, "candidate": args.avw}
    # Fresh projects keep each ASR cache cold; imports and queuing happen before timing.
    warm = {role:prepare(binary, "warm-" + role) for role,binary in binaries.items()}
    order = ["baseline","candidate","candidate","baseline",
             "candidate","baseline","baseline","candidate"]
    entries = [prepare(binaries[role], f"sample-{i}-{role}") for i,role in enumerate(order)]
    references = [execute(warm[role])[1] for role in ("baseline","candidate")]
    if references[0] != references[1]:
        raise RuntimeError(f"warmup cues differ: {references}")
    samples = {"baseline": [], "candidate": []}
    for role,entry in zip(order,entries):
        elapsed,cues = execute(entry)
        if cues != references[0]:
            raise RuntimeError(f"{role} produced incomparable cues: {cues}")
        samples[role].append(elapsed)
    base_median = statistics.median(samples["baseline"])
    candidate_median = statistics.median(samples["candidate"])
    probe = json.loads(command([
        args.ffprobe, "-v", "error", "-select_streams", "a:0", "-show_format", "-show_streams",
        "-show_entries", "format=duration,size,start_time:stream=codec_name,sample_rate,channels,time_base,start_time",
        "-of", "json", source,
    ]))
    cpu_model = None
    cpuinfo = read_optional("/proc/cpuinfo") or ""
    for line in cpuinfo.splitlines():
        if line.lower().startswith(("model name", "hardware")) and ":" in line:
            cpu_model = line.split(":",1)[1].strip()
            break
    try:
        affinity = len(os.sched_getaffinity(0))
    except (AttributeError, OSError):
        affinity = None
    return {
        "available": True,
        "order": order,
        "workerSeconds": samples,
        "baselineMedianSeconds": base_median,
        "candidateMedianSeconds": candidate_median,
        "candidateChangePercent": (candidate_median / base_median - 1) * 100 if base_median else None,
        "source": {"sha256":source_hash,"bytes":source.stat().st_size,"probe":probe},
        "binaries": {
            role: {"sha256":sha256(binary),"version":command([binary,"--version"]).strip()}
            for role,binary in binaries.items()
        },
        "ffmpegVersion": command([args.ffmpeg,"-version"]).splitlines()[0],
        "machine": {"platform":platform.platform(),"cpuModel":cpu_model,
                    "logicalCpus":os.cpu_count(),"affinityCpus":affinity,
                    "cgroupCpuMax":read_optional("/sys/fs/cgroup/cpu.max"),
                    "cgroupMemoryMax":read_optional("/sys/fs/cgroup/memory.max")},
        "validations": {"freshProjects":True,"cacheMisses":True,"identicalCues":True,
                        "sameSourceAndFakeProvider":True},
        "limitations": [
            "Warm filesystem, generated 60-second audio, constant-output offline provider, and four serial samples per binary.",
            "Measures worker and audio-audit overhead, not real speech-model throughput or model accuracy.",
            "Shared CI runner contention and small samples can affect the comparison; no speed threshold is applied.",
        ],
    }

try:
    summary = run_measurement()
except Exception as error:
    summary = {"available":False,"reason":f"{type(error).__name__}: {str(error)[:2000]}",
               "limitations":["Optional diagnostic only; correctness checks run separately."]}
try:
    root.mkdir(parents=True, exist_ok=True)
    (root / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
except OSError as error:
    summary = {"available":False,"reason":f"cannot write summary: {error}"}
print("ASR_OVERHEAD_JSON=" + json.dumps(summary, separators=(",", ":"), sort_keys=True))
