#!/usr/bin/env python3
"""Generated source-wide storyboard / precise constrained preview / mapping regression."""
import argparse, base64, hashlib, json, statistics, subprocess, time
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
for name in ('avw','ffmpeg','ffprobe','output'): p.add_argument('--'+name,required=True)
a=p.parse_args();root=Path(a.output).resolve();root.mkdir(parents=True,exist_ok=False);project=root/'project';ledger=[]

def run(argv, data=None):
 r=subprocess.run(list(map(str,argv)),input=data,capture_output=True,timeout=300)
 assert r.returncode==0,(argv,r.stdout[-2000:],r.stderr[-2000:]);return r.stdout

def call(command,ok=True,**fields):
 req={'command':command,'project':str(project),**fields}
 if command=='agent-media':req.pop('project')
 t=time.perf_counter()
 r=subprocess.run([a.avw,'--workspace',str(root),'--ffmpeg',a.ffmpeg,'--ffprobe',a.ffprobe,'request','-'],input=json.dumps(req).encode(),capture_output=True,timeout=300)
 out=json.loads(r.stdout);ledger.append({'request':req,'response':out,'seconds':time.perf_counter()-t});(root/'commands.json').write_text(json.dumps(ledger,indent=2))
 assert out['ok']==ok,(req,out);assert (r.returncode==0)==ok,out
 return out['result'] if ok else out['error']

def view(command,request,key,state='succeeded'):
 revision=call('status')['revision'];t=time.perf_counter()
 job=call(command,request=request,key=key,expectedRevision=revision,noLaunch=True)
 run([a.avw,'worker',project]);elapsed=time.perf_counter()-t
 status=call('job-status',id=job['id']);assert status['state']==state,status
 if state!='succeeded':return status,elapsed,None
 artifact=call('artifact',id=job['id']);report=json.loads(Path(artifact['path']).read_text())
 assert call('status')['revision']==revision
 return report,elapsed,artifact

source=root/'source.mp4'
run([a.ffmpeg,'-v','error','-f','lavfi','-i','testsrc2=s=640x360:r=30:d=30','-f','lavfi','-i','sine=frequency=440:duration=30','-c:v','libx264','-preset','ultrafast','-threads','2','-g','240','-bf','2','-keyint_min','240','-sc_threshold','0','-pix_fmt','yuv420p','-x264-params','colorprim=bt709:transfer=bt709:colormatrix=bt709','-color_range','tv','-c:a','aac','-shortest',source])
original_sha=hashlib.sha256(source.read_bytes()).hexdigest();call('create',name='Source views regression');call('import',source=str(source),id='source',expectedRevision=0,key='source-import')
q={'assetId':'source','sampleEveryMs':10000,'limit':2}
page,t,artifact=view('source-storyboard',q,'story-page-first');d=page['data'];assert d['totalSamples']==3 and d['nextOffset']==2
assert [t['sourceTimeMs'] for t in d['tiles']]==[0,10000],d
assert not d['coverage']['continuousVideoInspected']
media=call('agent-media',media={'kind':'storyboard','project':str(project),'jobId':artifact['jobId']})
assert base64.b64decode(media['content']['data']).startswith(b'\x89PNG\r\n\x1a\n')
assert media['metadata']['storyboard']['tiles']==d['tiles']
page2,_,_=view('source-storyboard',{**q,'offset':2},'story-page-last');assert page2['data']['nextOffset'] is None and page2['data']['tiles'][0]['sourceTimeMs']==20000
cached,hit,cache_artifact=view('source-storyboard',q,'story-page-cached');assert cache_artifact['path']==artifact['path']
assert call('job-status',id=cache_artifact['jobId'])['result']['cacheHit']
# Normal clip cap and exact frame-to-original mapping.
preview={'assetId':'source','startMs':300,'endMs':10300,'maxDurationMs':10000,'maxBytes':2000000,'width':640,'height':360}
r,preview_time,pa=view('budget-preview',preview,'bounded-preview');pd=r['data'];assert pd['bytes']<=preview['maxBytes'] and len(pd['frameMapping'])==300,pd
assert pd['verification']['durationWithinBudget'] and pd['verification']['bytesWithinBudget']
assert abs(pd['sourceMinusProxyTime']['value']/pd['sourceMinusProxyTime']['rate']['numerator']*pd['sourceMinusProxyTime']['rate']['denominator']-.3)<.001
# The adaptive encoder may reduce resolution/CRF, while keeping all source frames.
adaptive_budget=pd['bytes']*3//4
adapted,_,_=view('budget-preview',{**preview,'maxBytes':adaptive_budget},'adapt-size-preview')
assert adapted['data']['bytes']<=adaptive_budget and len(adapted['data']['attempts'])>1,adapted['data']
# Actual duration must be measured: 299.97 frames cannot silently exceed a cap.
duration_failed,_,_=view('budget-preview',{**preview,'endMs':10299,'maxDurationMs':9999},'measured-duration',state='failed')
assert 'duration' in duration_failed['error'],duration_failed
# Impossible byte cap must not publish a successful or partial artifact.
failed,_,_=view('budget-preview',{**preview,'maxBytes':16384},'impossible-size',state='failed');assert 'maxBytes' in failed['error'],failed
assert failed['result'] is None
call('budget-preview',ok=False,request={**preview,'maxDurationMs':9999},expectedRevision=1,key='invalid-duration',noLaunch=True)
# Sparse VFR, nonzero container origin and deliberately delayed audio.
vfr=root/'vfr-offset.mp4'
run([a.ffmpeg,'-v','error','-f','lavfi','-i','testsrc2=s=320x180:r=30:d=8','-f','lavfi','-i',"aevalsrc=0.5*sin(2*PI*1000*t)*between(t\\,1\\,1.1):s=48000:d=8",'-vf',"drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='between(t,1,1.1)',select='not(mod(n,3))+eq(mod(n,30),1)',setpts=PTS+5/TB",'-af','asetpts=PTS+5.25/TB','-fps_mode','vfr','-copyts','-c:v','libx264','-preset','ultrafast','-threads','2','-pix_fmt','yuv420p','-x264-params','colorprim=bt709:transfer=bt709:colormatrix=bt709','-color_range','tv','-c:a','aac',vfr])
call('import',source=str(vfr),id='vfr',expectedRevision=1,key='vfr-import')
vr,_,_=view('source-storyboard',{'assetId':'vfr','startMs':550,'endMs':4500,'sampleEveryMs':1000,'limit':4},'vfr-storyboard')
assert vr['data']['tiles'][0]['sourceTimeMs']==600,vr['data']
vp,_,vpa=view('budget-preview',{'assetId':'vfr','startMs':500,'endMs':4500,'maxDurationMs':4500,'maxBytes':1000000,'width':320,'height':180},'vfr-preview')
assert vp['data']['frameMapping'] and abs(vp['data']['sourceClock']['containerOrigin']['value']*vp['data']['sourceClock']['containerOrigin']['rate']['denominator']/vp['data']['sourceClock']['containerOrigin']['rate']['numerator']-5)<.001,vp['data']
# Validate delay against authored bursts, not just packet metadata.
clip=next(Path(f['path']) for f in vpa['attachments'] if Path(f['path']).name=='proxy.mp4')
from array import array
pcm=array('f',run([a.ffmpeg,'-v','error','-i',clip,'-vn','-ac','1','-ar','48000','-f','f32le','-']))
audio_on=next(i/48000 for i in range(0,len(pcm)-480,480) if sum(v*v for v in pcm[i:i+480])/480>.02)
assert abs(audio_on-.75)<.06,audio_on
video_probe=json.loads(run([a.ffprobe,'-v','error','-select_streams','v:0','-show_frames','-show_entries','frame=best_effort_timestamp_time','-of','json',clip]))
gray=run([a.ffmpeg,'-v','error','-threads','1','-i',clip,'-an','-pix_fmt','gray','-fps_mode','passthrough','-f','rawvideo','-'])
w,h=vp['data']['width'],vp['data']['height'];frame_bytes=w*h
assert len(gray)==len(video_probe['frames'])*frame_bytes
video_on=next(float(row['best_effort_timestamp_time']) for i,row in enumerate(video_probe['frames']) if sum(gray[i*frame_bytes:(i+1)*frame_bytes])/frame_bytes>220)
assert abs(video_on-.5)<.04 and abs((audio_on-video_on)-.25)<.06,(video_on,audio_on)

# Active transcript changes invalidate cached text context, without editing footage.
call('transcript-import',transcript={'assetId':'source','language':'en','provider':'generated fixture','cues':[{'id':'words','startMs':500,'endMs':2000,'text':'Known source words'}]},expectedRevision=2,key='source-transcript')
text_page,_,ta=view('source-storyboard',q,'story-with-transcript')
assert text_page['data']['transcript']['status']=='available'
assert text_page['data']['transcript']['cues'][0]['text']=='Known source words'
assert ta['path']!=artifact['path']
# The named CLI alias uses the same service operation as JSON/MCP.
request_file=root/'story-request.json';request_file.write_text(json.dumps(q))
alias=json.loads(run([a.avw,'--workspace',root,'--ffmpeg',a.ffmpeg,'--ffprobe',a.ffprobe,'source-storyboard',project,'--request',request_file,'--expected-revision','3','--key','story-cli-alias','--no-launch']))
assert alias['ok'];run([a.avw,'worker',project]);assert call('job-status',id=alias['result']['id'])['state']=='succeeded'
# HDR and rotation reuse the source adapter, with observed clocks retained.
hdr=root/'hlg.mp4'
run([a.ffmpeg,'-v','error','-threads','1','-i',source,'-t','3','-vf','zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt2020:t=arib-std-b67:m=bt2020nc:r=limited,format=yuv420p10le','-c:v','libx265','-preset','ultrafast','-x265-params','pools=2:frame-threads=1:log-level=error:colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc','-c:a','copy',hdr])
call('import',source=str(hdr),id='hlg',expectedRevision=3,key='hlg-view-import')
hp,_,_=view('source-storyboard',{'assetId':'hlg','sampleEveryMs':1000,'limit':3},'hlg-storyboard')
assert len(hp['data']['tiles'])==3 and hp['data']['nextOffset'] is None,hp['data']
hr,_,ha=view('budget-preview',{'assetId':'hlg','startMs':300,'endMs':2300,'maxDurationMs':2000,'maxBytes':1000000,'width':320,'height':180},'hlg-budget-preview')
hclip=next(Path(f['path']) for f in ha['attachments'] if Path(f['path']).name=='proxy.mp4')
hprobe=json.loads(run([a.ffprobe,'-v','error','-select_streams','v:0','-show_streams','-of','json',hclip]))['streams'][0]
assert hprobe['color_transfer']=='bt709' and hprobe['pix_fmt']=='yuv420p',hprobe
rotated=root/'rotated.mp4';run([a.ffmpeg,'-v','error','-display_rotation','90','-i',source,'-t','3','-c','copy',rotated])
call('import',source=str(rotated),id='rotated',expectedRevision=4,key='rotate-view-import')
rot,_,_=view('budget-preview',{'assetId':'rotated','startMs':300,'endMs':2300,'maxDurationMs':2000,'maxBytes':2000000,'width':360,'height':640},'rotated-budget-preview')
assert rot['data']['width']==360 and rot['data']['height']==640,rot['data']
# Integrity rejection for an existing cached image.
sheet=next(Path(f['path']) for f in artifact['attachments'] if Path(f['path']).name=='storyboard.png');saved=sheet.read_bytes();sheet.write_bytes(saved+b'tamper')
call('agent-media',ok=False,media={'kind':'storyboard','project':str(project),'jobId':artifact['jobId']});sheet.write_bytes(saved)
video_saved=clip.read_bytes();clip.write_bytes(video_saved+b'tamper')
call('artifact',ok=False,id=vpa['jobId']);clip.write_bytes(video_saved)
managed=project/'originals'/original_sha;managed_saved=managed.read_bytes();managed.write_bytes(managed_saved+b'tamper')
source_failed,_,_=view('source-storyboard',q,'changed-source-refused',state='failed')
assert 'bytes changed' in source_failed['error'];managed.write_bytes(managed_saved)
assert hashlib.sha256(source.read_bytes()).hexdigest()==original_sha
assert not any(project.joinpath('cache').glob('analysis-*'))
summary={'passed':True,'storyboardTotalSamples':3,'pagination':True,'observedFrameTimes':True,'cacheHit':True,'storyboardColdSeconds':t,'storyboardWarmSeconds':hit,'previewSeconds':preview_time,'previewBytes':pd['bytes'],'previewFrames':pd['verification']['videoFrames'],'hardCaps':True,'adaptiveEncoding':True,'actualDurationFailureRejected':True,'transcriptContextInvalidation':True,'cliAlias':True,'impossibleBudgetRejected':True,'originalUnchanged':True,'nonzeroOriginVfrMapping':True,'hdrSdrPreview':True,'rotationAppliedOnce':True,'delayedAudioOnsetSeconds':audio_on,'lightBurstOnsetSeconds':video_on,'measuredAvOffsetSeconds':audio_on-video_on,'tamperRejected':True,'sourceTamperRejectedOnCacheHit':True,'videoTamperRejected':True,'revision':call('status')['revision'],'storyboardJobId':artifact['jobId'],'previewJobId':pa['jobId'],'limitations':['Generated local SDR; no semantic model or host-specific media acceptance','Storyboard is sparse; no claim to capture every important event']}
(root/'summary.json').write_text(json.dumps(summary,indent=2));print(json.dumps(summary,indent=2))
