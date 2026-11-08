#!/usr/bin/env python3
"""Generated source-wide storyboard / precise constrained preview / mapping regression."""
import argparse, base64, hashlib, json, re, statistics, subprocess, time
from pathlib import Path
from fractions import Fraction
p=argparse.ArgumentParser(description=__doc__)
for name in ('avw','ffmpeg','ffprobe','output'): p.add_argument('--'+name,required=True)
p.add_argument('--baseline-avw', help='optional installed 0.5.1 binary for legacy artifact/cache compatibility')
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

def assert_story_cells(report, artifact, original, adapter='', autorotate=False):
 """Compare each cell with an independent single-seek decode, including padding."""
 data=report['data'];tw,th,columns=data['tileWidth'],data['tileHeight'],data['columns']
 rows=(len(data['tiles'])+columns-1)//columns;w,h=tw*columns,th*rows
 sheet=next(f['path'] for f in artifact['attachments'] if Path(f['path']).name=='storyboard.png')
 rgb=run([a.ffmpeg,'-v','error','-threads','1','-i',sheet,'-pix_fmt','rgb24','-threads','1','-f','rawvideo','-'])
 assert len(rgb)==w*h*3,(len(rgb),w,h)
 for tile in data['tiles']:
  filters=(adapter+',' if adapter else '')+f'scale={tw}:{th}:force_original_aspect_ratio=decrease,pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2,setsar=1'
  input_args=[a.ffmpeg,'-v','error','-threads','1','-ss',str(tile['requestedSourceTimeMs']/1000)]
  if not autorotate:input_args+=['-noautorotate']
  expected=run([*input_args,'-i',original,'-vf',filters,'-frames:v','1','-fps_mode','passthrough','-pix_fmt','rgb24','-threads','1','-f','rawvideo','-'])
  actual=b''.join(rgb[((tile['row']*th+y)*w+tile['column']*tw)*3:((tile['row']*th+y)*w+(tile['column']+1)*tw)*3] for y in range(th))
  assert actual==expected,(original,tile)
 # Cells outside a partial last row must remain black.
 for cell in range(len(data['tiles']),columns*rows):
  assert not any(b for y in range(th) for b in rgb[((cell//columns*th+y)*w+cell%columns*tw)*3:((cell//columns*th+y)*w+(cell%columns+1)*tw)*3])

def assert_suggestion(tile, scope_start, scope_end):
 request=tile['suggestedPreview']
 assert set(request)=={'assetId','startMs','endMs','maxDurationMs','maxBytes','width','height'},request
 assert request['assetId'] and request['maxDurationMs']==10000 and request['maxBytes']==1048576
 assert request['width']==640 and request['height']==640
 source_time=tile['sourceTime'];rate=source_time['rate']
 observed_ms=Fraction(source_time['value']*rate['denominator']*1000,rate['numerator'])
 assert scope_start<=request['startMs']<=observed_ms<request['endMs']<=scope_end,(tile,request)
 assert request['endMs']-request['startMs']==min(9000,scope_end-scope_start),request
 return request

def assert_preview_budget(report, artifact, request):
 clip=next(Path(f['path']) for f in artifact['attachments'] if Path(f['path']).name=='proxy.mp4')
 actual=json.loads(run([a.ffprobe,'-v','error','-show_entries','format=duration','-of','json',clip]))['format']['duration']
 assert clip.stat().st_size==report['data']['bytes']<=request['maxBytes']
 assert Fraction(actual)*1000<=request['maxDurationMs'],(actual,request)
 assert report['data']['verification']['durationWithinBudget']

source=root/'source.mp4'
run([a.ffmpeg,'-v','error','-f','lavfi','-i','testsrc2=s=640x360:r=30:d=30','-f','lavfi','-i','sine=frequency=440:duration=30','-c:v','libx264','-preset','ultrafast','-threads','2','-g','240','-bf','2','-keyint_min','240','-sc_threshold','0','-pix_fmt','yuv420p','-x264-params','colorprim=bt709:transfer=bt709:colormatrix=bt709','-color_range','tv','-c:a','aac','-shortest',source])
original_sha=hashlib.sha256(source.read_bytes()).hexdigest();call('create',name='Source views regression');call('import',source=str(source),id='source',expectedRevision=0,key='source-import')
q={'assetId':'source','sampleEveryMs':10000,'limit':2}
page,t,artifact=view('source-storyboard',q,'story-page-first');d=page['data'];assert d['totalSamples']==3 and d['nextOffset']==2
assert [t['sourceTimeMs'] for t in d['tiles']]==[0,10000],d
for tile in d['tiles']:assert_suggestion(tile,d['scopeStartMs'],d['scopeEndMs'])
suggestion=assert_suggestion(d['tiles'][0],d['scopeStartMs'],d['scopeEndMs'])
suggested,_,suggested_artifact=view('budget-preview',suggestion,'story-suggested-preview')
assert_preview_budget(suggested,suggested_artifact,suggestion)
assert any(pair['sourcePts']==d['tiles'][0]['observedPts'] for pair in suggested['data']['frameMapping'])
assert not d['coverage']['continuousVideoInspected']
media=call('agent-media',media={'kind':'storyboard','project':str(project),'jobId':artifact['jobId']})
assert base64.b64decode(media['content']['data']).startswith(b'\x89PNG\r\n\x1a\n')
assert media['metadata']['storyboard']['tiles']==d['tiles']
legacy={'exercised':False}
if a.baseline_avw:
 assert run([a.baseline_avw,'--version']).decode().strip()=='avw 0.5.1'
 current_avw=a.avw
 a.avw=a.baseline_avw
 old_story,_,old_story_artifact=view('source-storyboard',q,'legacy-storyboard')
 assert all('suggestedPreview' not in tile for tile in old_story['data']['tiles'])
 old_query={'assetId':'source','startMs':1300,'endMs':4300,'maxDurationMs':4000,
            'maxBytes':1500000,'width':320,'height':180}
 _,_,old_preview_artifact=view('budget-preview',old_query,'legacy-budget-preview')
 a.avw=current_avw
 old_media=call('agent-media',media={'kind':'storyboard','project':str(project),'jobId':old_story_artifact['jobId']})
 assert all('suggestedPreview' not in tile for tile in old_media['metadata']['storyboard']['tiles'])
 refreshed,_,refreshed_artifact=view('source-storyboard',q,'current-storyboard-cache')
 assert refreshed_artifact['path']!=old_story_artifact['path']
 assert all('suggestedPreview' in tile for tile in refreshed['data']['tiles'])
 _,_,reused_preview=view('budget-preview',old_query,'current-budget-preview-cache')
 assert reused_preview['path']==old_preview_artifact['path']
 assert call('job-status',id=reused_preview['jobId'])['result']['cacheHit']
 legacy={'exercised':True,'storyboardJobId':old_story_artifact['jobId'],
         'legacyStoryboardReadable':True,'storyboardCacheRefreshed':True,
         'budgetPreviewCacheReused':True}
page2,_,_=view('source-storyboard',{**q,'offset':2},'story-page-last');assert page2['data']['nextOffset'] is None and page2['data']['tiles'][0]['sourceTimeMs']==20000
cached,hit,cache_artifact=view('source-storyboard',q,'story-page-cached');assert cache_artifact['path']==artifact['path']
assert call('job-status',id=cache_artifact['jobId'])['result']['cacheHit']
assert all('suggestedPreview' in tile for tile in cached['data']['tiles'])
# A non-frame-aligned request must anchor the suggestion to the observed PTS,
# using floor rather than the rounded display timestamp.
mid,_,_=view('source-storyboard',{'assetId':'source','startMs':10034,'endMs':25034,
                                  'sampleEveryMs':1000,'offset':5,'limit':1},'observed-anchor-story')
mid_tile=mid['data']['tiles'][0]
mid_suggestion=assert_suggestion(mid_tile,10034,25034)
assert mid_tile['requestedSourceTimeMs']==15034 and mid_tile['sourceTimeMs']==15067,mid_tile
assert mid_suggestion['startMs']==10566 and mid_suggestion['endMs']==19566,mid_tile
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
hint=re.search(r'smallest observed attempt was (\d+) bytes',failed['error']);assert hint and int(hint[1])>16384,failed
assert failed['result'] is None
call('budget-preview',ok=False,request={**preview,'maxDurationMs':9999},expectedRevision=1,key='invalid-duration',noLaunch=True)
# Sparse VFR, nonzero container origin and deliberately delayed audio.
vfr=root/'vfr-offset.mp4'
run([a.ffmpeg,'-v','error','-f','lavfi','-i','testsrc2=s=320x180:r=30:d=8','-f','lavfi','-i',"aevalsrc=0.5*sin(2*PI*1000*t)*between(t\\,1\\,1.1):s=48000:d=8",'-vf',"drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='between(t,1,1.1)',select='not(mod(n,3))+eq(mod(n,30),1)',setpts=PTS+5/TB",'-af','asetpts=PTS+5.25/TB','-fps_mode','vfr','-copyts','-c:v','libx264','-preset','ultrafast','-threads','2','-pix_fmt','yuv420p','-x264-params','colorprim=bt709:transfer=bt709:colormatrix=bt709','-color_range','tv','-c:a','aac',vfr])
call('import',source=str(vfr),id='vfr',expectedRevision=1,key='vfr-import')
vr,_,vra=view('source-storyboard',{'assetId':'vfr','startMs':550,'endMs':4500,'sampleEveryMs':1000,'limit':4},'vfr-storyboard')
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
hp,_,hpa=view('source-storyboard',{'assetId':'hlg','sampleEveryMs':1000,'limit':3},'hlg-storyboard')
assert len(hp['data']['tiles'])==3 and hp['data']['nextOffset'] is None,hp['data']
hr,_,ha=view('budget-preview',{'assetId':'hlg','startMs':300,'endMs':2300,'maxDurationMs':2000,'maxBytes':1000000,'width':320,'height':180},'hlg-budget-preview')
hclip=next(Path(f['path']) for f in ha['attachments'] if Path(f['path']).name=='proxy.mp4')
hprobe=json.loads(run([a.ffprobe,'-v','error','-select_streams','v:0','-show_streams','-of','json',hclip]))['streams'][0]
assert hprobe['color_transfer']=='bt709' and hprobe['pix_fmt']=='yuv420p',hprobe
rotated=root/'rotated.mp4';run([a.ffmpeg,'-v','error','-display_rotation','90','-i',source,'-t','3','-c','copy',rotated])
call('import',source=str(rotated),id='rotated',expectedRevision=4,key='rotate-view-import')
rot,_,_=view('budget-preview',{'assetId':'rotated','startMs':300,'endMs':2300,'maxDurationMs':2000,'maxBytes':2000000,'width':360,'height':640},'rotated-budget-preview')
assert rot['data']['width']==360 and rot['data']['height']==640,rot['data']
# Independently validate sparse-cell placement and actual image content.
dense,_,da=view('source-storyboard',{'assetId':'source','sampleEveryMs':1000,'limit':24},'dense-storyboard')
assert len(dense['data']['tiles'])==24 and dense['data']['nextOffset']==24
assert_story_cells(dense,da,source)
ragged,_,ra=view('source-storyboard',{'assetId':'source','sampleEveryMs':1000,'offset':3,'limit':6},'ragged-storyboard')
assert [t['sourceTimeMs'] for t in ragged['data']['tiles']]==list(range(3000,9000,1000))
assert_story_cells(ragged,ra,source)
single,_,sa=view('source-storyboard',{'assetId':'source','sampleEveryMs':1000,'offset':29,'limit':24},'single-last-tile')
assert single['data']['nextOffset'] is None and len(single['data']['tiles'])==1
assert_story_cells(single,sa,source)
assert_story_cells(vr,vra,vfr)
rot_story,_,rsa=view('source-storyboard',{'assetId':'rotated','sampleEveryMs':1000,'limit':3},'rotated-storyboard')
assert_story_cells(rot_story,rsa,rotated,autorotate=True)
hdr_filter='zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=mobius:param=0.3:desat=2:peak=10,zscale=t=bt709:m=bt709:r=limited:dither=error_diffusion,format=yuv420p'
assert_story_cells(hp,hpa,hdr,hdr_filter)
large=root/'large.mp4'
run([a.ffmpeg,'-v','error','-f','lavfi','-i','testsrc2=s=3840x2160:r=2:d=6','-c:v','libx264','-preset','ultrafast','-threads','2','-x264-params','colorprim=bt709:transfer=bt709:colormatrix=bt709','-color_range','tv',large])
call('import',source=str(large),id='large',expectedRevision=5,key='large-view-import')
large_story,_,lsa=view('source-storyboard',{'assetId':'large','sampleEveryMs':1000,'limit':5},'large-storyboard')
assert_story_cells(large_story,lsa,large)
silent_preview,_,_=view('budget-preview',{'assetId':'large','startMs':500,'endMs':2500,'maxDurationMs':2000,'maxBytes':1000000,'width':640,'height':360},'video-only-preview')
assert silent_preview['data']['verification']['videoFrames']==4,silent_preview['data']
# Observe the actual backend invocations: output metadata probing must not
# request frames, and one full audio/video validation decode must remain.
actual_ffmpeg,actual_ffprobe=a.ffmpeg,a.ffprobe
for name in ('ffmpeg','ffprobe'):
 actual=getattr(a,name);wrapper=root/(name+'-observer');log_path=root/(name+'-invocations.jsonl')
 wrapper.write_text('#!/usr/bin/env python3\nimport json,os,sys\n'+f'with open({str(log_path)!r},"a") as log: log.write(json.dumps(sys.argv[1:])+"\\n")\n'+f'os.execv({actual!r},[{actual!r},*sys.argv[1:]])\n')
 wrapper.chmod(0o755);setattr(a,name,str(wrapper))
observed,_,_=view('budget-preview',{**preview,'maxBytes':2000011},'observed-one-decode')
a.ffmpeg,a.ffprobe=actual_ffmpeg,actual_ffprobe
probes=[json.loads(l) for l in (root/'ffprobe-invocations.jsonl').read_text().splitlines()]
assert len(probes)==2 and sum('-show_frames' in p for p in probes)==1,probes
metadata_probe=next(p for p in probes if '-show_frames' not in p)
assert 'frame=' not in metadata_probe[metadata_probe.index('-show_entries')+1],metadata_probe
ffmpeg_calls=[json.loads(l) for l in (root/'ffmpeg-invocations.jsonl').read_text().splitlines()]
assert sum('framecrc' in p for p in ffmpeg_calls)==1 and not any('null' in p for p in ffmpeg_calls),ffmpeg_calls
assert observed['data']['verification']['frameCountMethod']=='ffmpeg-framecrc-single-decode'
before=len(ffmpeg_calls)
# Observe exactly four image processes for twelve normal-resolution samples.
a.ffmpeg=str(root/'ffmpeg-observer')
view('source-storyboard',{'assetId':'source','sampleEveryMs':1000,'limit':12,'maxBytes':1048587},'observed-row-graphs')
row_calls=[json.loads(l) for l in (root/'ffmpeg-invocations.jsonl').read_text().splitlines()][before:]
assert sum('-filter_complex' in p for p in row_calls)==4,row_calls
assert all(p.count('-i')<=4 for p in row_calls),row_calls
before+=len(row_calls)
view('source-storyboard',{'assetId':'large','sampleEveryMs':1000,'limit':5,'maxBytes':1048588},'observed-large-rows')
large_calls=[json.loads(l) for l in (root/'ffmpeg-invocations.jsonl').read_text().splitlines()][before:]
source_reads=[p for p in large_calls if '-ss' in p]
assert len(source_reads)==5 and all(p.count('-i')==1 for p in source_reads),source_reads
a.ffmpeg=actual_ffmpeg
# Seven low-resolution frames span over 25 hours. Frames around the 24h
# boundary are one second apart; large PTS gaps keep this fixture quick.
long_source=root/'long-source.mp4'
pts="setpts='if(lt(N,2),N,if(lt(N,6),86397+N,89994+N))/TB'"
run([a.ffmpeg,'-v','error','-f','lavfi','-i','testsrc2=s=64x64:r=1:d=7',
     '-vf',pts,'-fps_mode','passthrough','-c:v','libx264','-preset','ultrafast',
     '-threads','1','-g','1','-bf','0','-pix_fmt','yuv420p','-x264-params',
     'colorprim=bt709:transfer=bt709:colormatrix=bt709',long_source])
frame_probe=json.loads(run([a.ffprobe,'-v','error','-select_streams','v:0','-show_frames',
                            '-show_entries','frame=best_effort_timestamp_time','-of','json',long_source]))
assert [Fraction(f['best_effort_timestamp_time']) for f in frame_probe['frames']]==[
    0,1,86399,86400,86401,86402,90000],frame_probe
revision=call('status')['revision']
call('import',source=str(long_source),id='long',expectedRevision=revision,key='long-source-import')
long_query={'assetId':'long','startMs':86399000,'endMs':86403000,'sampleEveryMs':1000,'limit':2}
late1,_,late1_artifact=view('source-storyboard',long_query,'long-first-page')
late2,_,late2_artifact=view('source-storyboard',{**long_query,'offset':2},'long-second-page')
assert [t['sourceTimeMs'] for t in late1['data']['tiles']]==[86399000,86400000],late1['data']
assert [t['sourceTimeMs'] for t in late2['data']['tiles']]==[86401000,86402000],late2['data']
assert (late1['data']['pageStartMs'],late1['data']['pageEndMs'],late1['data']['nextOffset'])==(86399000,86401000,2)
assert (late2['data']['pageStartMs'],late2['data']['pageEndMs'],late2['data']['nextOffset'])==(86401000,86403000,None)
assert late2['data']['coverage']['scopeContinues']
assert_story_cells(late1,late1_artifact,long_source)
assert_story_cells(late2,late2_artifact,long_source)
for page in (late1,late2):
 for tile in page['data']['tiles']:
  assert_suggestion(tile,86399000,86403000)
selected=late1['data']['tiles'][1]
long_preview=assert_suggestion(selected,86399000,86403000)
assert (long_preview['startMs'],long_preview['endMs'])==(86399000,86403000)
long_result,_,long_artifact=view('budget-preview',long_preview,'long-suggested-preview')
assert_preview_budget(long_result,long_artifact,long_preview)
assert any(pair['sourcePts']==selected['observedPts'] for pair in long_result['data']['frameMapping'])
late_default,_,_=view('source-storyboard',{'assetId':'long','startMs':86400000,
                                        'sampleEveryMs':1000,'limit':2},'long-default-scope')
assert late_default['data']['scopeEndMs']>=86403000
assert [t['sourceTimeMs'] for t in late_default['data']['tiles']]==[86400000,86401000]
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
summary.update({'exactStoryboardCells':True,'raggedAndDensePages':True,'largeFrameSequentialFallback':True,'singlePreviewValidationDecode':True,'measuredImpossibleBudgetHint':True,'videoOnlyPreview':True,'twelveTileImageProcesses':4})
summary['limitations'][0]='Generated local media; no semantic model or host-specific media acceptance'
summary.update({'longSourceBeyond24Hours':True,'suggestedPreviewFromObservedPts':True,
                'legacyCompatibility':legacy})
(root/'summary.json').write_text(json.dumps(summary,indent=2));print(json.dumps(summary,indent=2))
