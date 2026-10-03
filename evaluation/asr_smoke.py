#!/usr/bin/env python3
"""Real local whisper.cpp test on generated speech; no private audio/model in Git."""
import argparse
import json
from pathlib import Path
import subprocess
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('avw','ffmpeg','ffprobe','whisper','model','model-sha256','output'):
        p.add_argument('--'+name, required=True)
    a=p.parse_args()
    root=Path(a.output).resolve()
    root.mkdir(parents=True,exist_ok=False)
    speech=root/'speech.wav'
    subprocess.run(['/usr/bin/ffmpeg','-v','error','-f','lavfi','-i',
        "flite=text='Hello. This video shows our product demonstration. Please keep the product visible.':voice=slt",
        '-ar','16000',str(speech)],check=True,timeout=30)
    prefix=[a.avw,'--ffmpeg',a.ffmpeg,'--ffprobe',a.ffprobe,'--whisper',a.whisper,
            '--whisper-model',a.model,'--whisper-model-sha256',a.model_sha256]
    ledger=[]
    def run(*args, expected=0):
        result=subprocess.run([*prefix,*map(str,args)],capture_output=True,text=True,timeout=30)
        ledger.append({'args':list(map(str,args)),'exit':result.returncode,'stdout':result.stdout,'stderr':result.stderr})
        (root/'commands.json').write_text(json.dumps(ledger,indent=2))
        assert result.returncode==expected,ledger[-1]
        response=json.loads(result.stdout)
        assert response['ok']==(expected==0),response
        return response.get('result',response)
    project=root/'project'
    run('create',project,'--name','Local ASR')
    run('import',project,speech,'--id','speech','--expected-revision',0,'--key','import-speech')
    job=run('transcribe-start',project,'--asset-id','speech','--expected-revision',1,'--key','asr')
    for _ in range(360):
        status=run('job-status',project,job['id'])
        if status['state'] in ('succeeded','failed','cancelled','interrupted'):
            break
        time.sleep(0.25)
    assert status['state']=='succeeded',status
    artifact=run('artifact',project,job['id'])
    transcript=json.loads(Path(artifact['path']).read_text())
    text=' '.join(c['text'] for c in transcript['cues']).lower()
    assert 'product' in text and 'video' in text,transcript
    assert run('status',project)['revision']==1,'analysis unexpectedly changed project history'
    run('transcript-import',project,'--request',artifact['path'],'--expected-revision',1,'--key','attach')
    assert run('transcript-search',project,'product')['items']
    # Another job reuses verified model/source analysis, never retranscribing for style changes.
    cached=run('transcribe-start',project,'--asset-id','speech','--expected-revision',2,'--key','asr-cache')
    for _ in range(100):
        cached=run('job-status',project,cached['id'])
        if cached['state']=='succeeded':break
        time.sleep(0.1)
    assert cached['state']=='succeeded' and cached['result']['cacheHit'],cached
    # Wrong model identity fails the queued job without attaching new text.
    prefix[-1]='0'*64
    bad=run('transcribe-start',project,'--asset-id','speech','--expected-revision',2,'--key','wrong-model')
    for _ in range(100):
        bad=run('job-status',project,bad['id'])
        if bad['state'] in ('failed','succeeded'):break
        time.sleep(0.1)
    assert bad['state']=='failed' and 'checksum' in bad['error'],bad
    assert run('status',project)['revision']==2
    prefix[-1]=a.model_sha256
    with Path(artifact['path']).open('a') as changed: changed.write('\n ')
    assert run('artifact',project,job['id'],expected=1)['error']['code']=='E_INVALID_REQUEST'
    changed=run('transcribe-start',project,'--asset-id','speech','--expected-revision',2,'--key','changed-cache')
    for _ in range(100):
        changed=run('job-status',project,changed['id'])
        if changed['state'] in ('failed','succeeded'):break
        time.sleep(0.1)
    assert changed['state']=='failed' and 'cached transcript bytes changed' in changed['error'],changed
    (root/'summary.json').write_text(json.dumps({'passed':True,'realWhisperModel':True,
        'modelSha256':a.model_sha256,'cueCount':len(transcript['cues']),'text':text,
        'analysisDidNotMutateHistory':True,'cacheReused':True,'modelMismatchRejected':True,'changedArtifactRejected':True,'changedCacheRejected':True},indent=2))
    print('ASR smoke passed:',root/'summary.json')


if __name__=='__main__':main()
