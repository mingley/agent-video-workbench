#!/usr/bin/env python3
"""Check ASR source-clock mapping against generated audio and the real decoder."""
import argparse
from array import array
from fractions import Fraction
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__)
for option in ("avw", "ffmpeg", "ffprobe", "output"):
    parser.add_argument("--" + option, required=True)
parser.add_argument("--baseline-avw", help="optional real 0.5.1 binary")
parser.add_argument("--v3-avw", help="optional binary that writes ASR adapter v3")
args = parser.parse_args()
root = Path(args.output).resolve()
root.mkdir(parents=True, exist_ok=False)
model = root / "model.bin"
model.write_bytes(b"generated offline ASR model")
model_sha = hashlib.sha256(model.read_bytes()).hexdigest()
calls = root / "provider-calls.jsonl"
provider = root / "fake-whisper"

# The provider derives timestamps from the actual extracted WAV. It neither
# knows the source container origin nor is told the expected output timestamps.
provider.write_text(
    "#!/usr/bin/env python3\n"
    "from array import array\n"
    "import json, sys, wave\n"
    "from pathlib import Path\n"
    f"calls = Path({str(calls)!r})\n"
    "args = sys.argv\n"
    "path = Path(args[args.index('-f') + 1])\n"
    "prefix = Path(args[args.index('-of') + 1])\n"
    "with wave.open(str(path), 'rb') as wav:\n"
    "    assert (wav.getnchannels(), wav.getframerate(), wav.getsampwidth()) == (1, 16000, 2)\n"
    "    pcm = array('h', wav.readframes(wav.getnframes()))\n"
    "if sys.byteorder != 'little': pcm.byteswap()\n"
    "groups = []\n"
    "for i in range(0, len(pcm) - 15, 16):\n"
    "    if sum(int(x)*int(x) for x in pcm[i:i+16]) / 16 > 500**2:\n"
    "        if not groups or i - groups[-1][-1] > 16: groups.append([i])\n"
    "        else: groups[-1].append(i)\n"
    "starts = [round(g[0] / 16) for g in groups if len(g) >= 40]\n"
    "with calls.open('a') as f: f.write(json.dumps({'startsMs':starts}) + '\\n')\n"
    "segments = [{'offsets':{'from':n,'to':n+40},'text':'burst '+str(i)} for i,n in enumerate(starts)]\n"
    "Path(str(prefix)+'.json').write_text(json.dumps({'transcription':segments}))\n",
    encoding="utf-8",
)
provider.chmod(0o755)

def run(argv, payload=None):
    result = subprocess.run(list(map(str, argv)), input=payload, capture_output=True, timeout=180)
    if result.returncode:
        raise AssertionError((list(map(str, argv)), result.stdout[-3000:], result.stderr[-3000:]))
    return result.stdout

def prefix(binary, probe=None):
    return [binary, "--workspace", root, "--ffmpeg", args.ffmpeg, "--ffprobe", probe or args.ffprobe,
            "--whisper", provider, "--whisper-model", model, "--whisper-model-sha256", model_sha]

def call(binary, project, command, probe=None, **fields):
    request = {"command": command, "project": str(project), **fields}
    raw = run([*prefix(binary, probe), "request", "-"], json.dumps(request).encode())
    response = json.loads(raw)
    assert response["ok"], (request, response)
    return response["result"]

def transcribe(binary, project, key, probe=None):
    job = call(binary, project, "transcribe-start", probe=probe, assetId="audio",
               expectedRevision=1, key=key, noLaunch=True)
    run([*prefix(binary, probe), "worker", project])
    return call(binary, project, "job-status", probe=probe, id=job["id"])

def call_count():
    return len(calls.read_text().splitlines()) if calls.exists() else 0

def create_project(name, source, probe=None):
    project = root / ("project-" + name)
    call(args.avw, project, "create", probe=probe, name="ASR clock " + name)
    call(args.avw, project, "import", probe=probe, source=str(source), id="audio",
         expectedRevision=0, key="import-source-" + name)
    return project

def make_fixture(name, *, rate=16000, origin=0, delay=0, gap=False, near_end=False):
    source = root / (name + ".mkv")
    if near_end:
        audio = r"aevalsrc=0.5*sin(2*PI*1000*t)*(between(t\,0.1\,0.18)+between(t\,0.3\,0.38))"
        duration = 0.5
    else:
        audio = r"aevalsrc=0.5*sin(2*PI*1000*t)*(between(t\,0.25\,0.4)+between(t\,1.5\,1.65))"
        duration = 3
    audio += f":s={rate}:d={duration}"
    shift = f"PTS+{origin+delay}/TB" + (r"+gte(T\,1)/TB" if gap else "")
    graph = f"[0:v]setpts=PTS+{origin}/TB[v];[1:a]asetpts={shift}[a]"
    run([args.ffmpeg, "-nostdin", "-v", "error", "-copyts",
         "-f", "lavfi", "-i", "testsrc2=s=64x64:r=10:d=4",
         "-f", "lavfi", "-i", audio, "-filter_complex", graph, "-map", "[v]", "-map", "[a]",
         "-fps_mode", "passthrough", "-c:v", "libx264", "-preset", "ultrafast",
         "-threads:v", "1", "-c:a", "pcm_s16le", "-avoid_negative_ts", "disabled", "-y", source])
    return source

def source_clock(source):
    observed = json.loads(run([args.ffprobe, "-v", "error", "-select_streams", "a:0",
        "-show_frames", "-show_streams", "-show_format", "-show_entries",
        "format=start_time:stream=time_base,sample_rate:frame=best_effort_timestamp,nb_samples",
        "-of", "json", source]))
    stream = observed["streams"][0]
    frames = observed["frames"]
    base = Fraction(stream["time_base"])
    rate = int(stream["sample_rate"])
    times = [int(frame["best_effort_timestamp"]) * base for frame in frames]
    origin_value = observed.get("format", {}).get("start_time")
    origin = Fraction(origin_value) if origin_value not in (None, "N/A") else None
    gaps = [b-a-Fraction(int(frame["nb_samples"]),rate)
            for a,b,frame in zip(times,times[1:],frames)]
    return origin, times[0], gaps

def starts_from_raw(source):
    data = run([args.ffmpeg, "-nostdin", "-v", "error", "-i", source,
                "-map", "0:a:0", "-vn", "-ar", "16000", "-ac", "1",
                "-c:a", "pcm_s16le", "-f", "s16le", "-"])
    pcm = array("h")
    pcm.frombytes(data)
    if sys.byteorder != "little":
        pcm.byteswap()
    groups = []
    for i in range(0, len(pcm)-15, 16):
        if sum(int(x)*int(x) for x in pcm[i:i+16])/16 > 500**2:
            if not groups or i-groups[-1][-1] > 16:
                groups.append([i])
            else:
                groups[-1].append(i)
    return [round(group[0]/16) for group in groups if len(group) >= 40]

def validate_case(name, source, authored, *, tolerance_ms=2):
    origin, first_pts, gaps = source_clock(source)
    assert not gaps or max(abs(float(g)) for g in gaps) < .01, (name, gaps[:5])
    if origin is None:
        # This is asserted separately only for the generated audio-only WAV.
        assert name == "wav" and first_pts == 0
        origin = Fraction(0)
    reference = starts_from_raw(source)
    assert len(reference) == 2, (name, reference)
    expected = [float((first_pts-origin)*1000)+value for value in reference]
    assert all(abs(a-b) <= 12 for a,b in zip(expected,authored)), (name, expected, authored)
    project = create_project(name, source)
    before = call_count()
    job = transcribe(args.avw, project, "clock-" + name)
    assert job["state"] == "succeeded", (name,job)
    assert call_count() == before+1
    artifact = call(args.avw, project, "artifact", id=job["id"])
    transcript = json.loads(Path(artifact["path"]).read_text())
    cues = transcript["cues"]
    actual = [cue["startMs"] for cue in cues]
    assert len(actual) == 2 and all(abs(a-b) <= tolerance_ms for a,b in zip(actual,expected)), (
        name, actual, expected, cues)
    assert all(0 <= cue["startMs"] < cue["endMs"] <= 4000 for cue in cues), (name,cues)
    return {"expectedMs":expected,"actualMs":actual,"firstPtsMs":float(first_pts*1000),
            "originMs":float(origin*1000)}

fixtures = {
    "zero": (make_fixture("zero"), [250,1500]),
    "delay": (make_fixture("delay",delay=1), [1250,2500]),
    "origin": (make_fixture("origin",origin=5,delay=1), [1250,2500]),
    "resampled": (make_fixture("resampled",rate=44100,delay=.375), [625,1875]),
    "negative": (make_fixture("negative",origin=-1,delay=.5), [250,1500]),
    "near-end": (make_fixture("near-end",delay=3.5,near_end=True), [3600,3800]),
}
wav = root / "audio-only.wav"
run([args.ffmpeg,"-nostdin","-v","error","-f","lavfi","-i",
     r"aevalsrc=0.5*sin(2*PI*1000*t)*(between(t\,0.25\,0.4)+between(t\,1.5\,1.65)):s=16000:d=2",
     "-c:a","pcm_s16le","-y",wav])
fixtures["wav"] = (wav,[250,1500])
aac = root / "aac.mkv"
run([args.ffmpeg,"-nostdin","-v","error","-copyts","-i",fixtures["resampled"][0],
     "-map","0:v:0","-map","0:a:0","-c:v","copy","-c:a","aac","-b:a","96k",
     "-avoid_negative_ts","disabled","-y",aac])
fixtures["aac"] = (aac,[625,1875])

# A second, louder stereo track is deliberately more attractive to FFmpeg's
# automatic stream selection; transcription must still use the first audio.
multi = root / "multi.mkv"
run([args.ffmpeg,"-nostdin","-v","error","-copyts","-i",fixtures["delay"][0],
     "-f","lavfi","-i","sine=frequency=2000:sample_rate=16000:duration=4",
     "-map","0:v:0","-map","0:a:0","-map","1:a:0","-c:v","copy","-c:a:0","copy",
     "-c:a:1","pcm_s16le","-ac:a:1","2","-avoid_negative_ts","disabled","-y",multi])
tracks = json.loads(run([args.ffprobe,"-v","error","-select_streams","a",
                        "-show_entries","stream=channels","-of","json",multi]))["streams"]
assert [stream["channels"] for stream in tracks] == [1,2], tracks
fixtures["multi"] = (multi,[1250,2500])

observed = {name:validate_case(name,source,authored) for name,(source,authored) in fixtures.items()}

gap = make_fixture("gap",gap=True)
_, _, gaps = source_clock(gap)
assert max(gaps) > Fraction(9,10), max(gaps)
gap_project = create_project("gap",gap)
before = call_count()
gap_job = transcribe(args.avw,gap_project,"clock-gap")
assert gap_job["state"] == "failed" and "audio clock" in gap_job["error"].lower(),gap_job
assert call_count() == before, "provider should not receive a collapsed discontinuous timeline"

# Simulate an ambiguous imported video clock: the original still has audio,
# but the stored ingest probe has no format origin. No provider call is safe.
probe_wrapper = root / "missing-origin-ffprobe"
probe_wrapper.write_text(
    "#!/usr/bin/env python3\n"
    "import json, subprocess, sys\n"
    f"p = subprocess.run([{args.ffprobe!r}, *sys.argv[1:]], capture_output=True)\n"
    "out = p.stdout\n"
    "try:\n"
    "    data = json.loads(out)\n"
    "    if isinstance(data, dict) and 'format' in data and 'streams' in data:\n"
    "        data['format'].pop('start_time', None)\n"
    "        out = json.dumps(data).encode()\n"
    "except (ValueError, TypeError): pass\n"
    "sys.stdout.buffer.write(out); sys.stderr.buffer.write(p.stderr); sys.exit(p.returncode)\n",
    encoding="utf-8",
)
probe_wrapper.chmod(0o755)
ambiguous = create_project("ambiguous",fixtures["delay"][0],probe=probe_wrapper)
before = call_count()
failed = transcribe(args.avw,ambiguous,"clock-ambiguous",probe=probe_wrapper)
assert failed["state"] == "failed" and "audio clock" in failed["error"].lower(),failed
assert call_count() == before

# A real older executable demonstrates that an old successful result remains
# readable but must not be used for a fresh, corrected transcript.
legacy = {"v2Exercised":False,"v3Exercised":False}
for name,binary in (("v2",args.baseline_avw),("v3",args.v3_avw)):
    if not binary:
        continue
    project = create_project("legacy-"+name,fixtures["delay"][0])
    old = transcribe(binary,project,"clock-old-"+name)
    assert old["state"] == "succeeded",old
    old_artifact = call(binary,project,"artifact",id=old["id"])
    old_path = Path(old_artifact["path"])
    old_bytes = old_path.read_bytes()
    old_hash = hashlib.sha256(old_bytes).hexdigest()
    fresh = transcribe(args.avw,project,"clock-new-"+name)
    assert fresh["state"] == "succeeded" and not fresh["result"]["cacheHit"],fresh
    assert Path(fresh["result"]["path"]) != old_path
    new_transcript = json.loads(Path(fresh["result"]["path"]).read_text())
    assert abs(new_transcript["cues"][0]["startMs"]-1250) <= 2,new_transcript
    assert old_path.read_bytes() == old_bytes
    assert call(args.avw,project,"artifact",id=old["id"])["sha256"] == old_hash
    legacy[name+"Exercised"] = True

summary = {
    "passed":True,
    "cases":observed,
    "internalPtsGapRefused":True,
    "ambiguousVideoOriginRefused":True,
    "firstAudioTrackSelected":True,
    "legacy":legacy,
    "limitations":["Generated deterministic audio and an offline fake provider; no speech accuracy claim.",
                   "Old-cache compatibility requires the corresponding optional old binary."],
}
(root/"summary.json").write_text(json.dumps(summary,indent=2)+"\n")
print(json.dumps(summary,indent=2))
