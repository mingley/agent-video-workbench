// Official MCP SDK client exercising the shared tool service and CLI replay.
import { pathToFileURL } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
const [avw, ffmpeg, ffprobe, font, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) throw new Error('Usage: node agent_smoke.mjs AVW FFMPEG FFPROBE FONT NEW_ROOT SDK_ROOT');
const { Client } = await import(pathToFileURL(path.join(sdkRoot,'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot,'dist/esm/client/stdio.js')));
fs.mkdirSync(root,{recursive:false});
const source=path.join(root,'source.mp4');
const generated=spawnSync(ffmpeg,['-v','error','-f','lavfi','-i','color=c=red:s=640x360:r=30:d=12','-f','lavfi','-i','sine=frequency=440:duration=12','-vf',"drawbox=color=lime:t=fill:enable='gte(t,7)'",'-c:v','libx264','-preset','ultrafast','-pix_fmt','yuv420p','-c:a','aac','-shortest',source],{encoding:'utf8'});
if(generated.status!==0)throw new Error(generated.stderr);
fs.copyFileSync(font,path.join(root,'font.ttf'));
const transport=new StdioClientTransport({command:avw,args:['--ffmpeg',ffmpeg,'--ffprobe',ffprobe,'mcp','--root',root],stderr:'inherit'});
const client=new Client({name:'avw-conformance',version:'1.0.0'});
const ledger=[];
try {
 await client.connect(transport);
 const tools=await client.listTools();
 if(tools.tools[0].name!=='avw'||tools.tools[0].inputSchema.type!=='object')throw new Error('invalid tool schema');
 async function call(args,shouldFail=false){
   const result=await client.callTool({name:'avw',arguments:args});
   const value=result.structuredContent??JSON.parse(result.content[0].text);
   ledger.push({request:args,response:value});fs.writeFileSync(path.join(root,'commands.json'),JSON.stringify(ledger,null,2));
   if((value.ok===false)!==shouldFail)throw new Error(JSON.stringify(value));
   return value.result??value;
 }
 await call({command:'doctor'});await call({command:'agent-guide'});
 await call({command:'create',project:'project',name:'MCP creator loop'});
 const state=()=>call({command:'status',project:'project'});
 for(const [id,file] of [['phone','source.mp4'],['font','font.ttf']]){
   const p=await state();await call({command:'import',project:'project',source:file,id,expectedRevision:p.revision,key:'import-'+id});
 }
 let p=await state();
 await call({command:'transcript-import',project:'project',expectedRevision:p.revision,key:'transcript',transcript:{assetId:'phone',provider:'reviewed fixture',language:'en',cues:[{id:'opening',startMs:0,endMs:3000,text:"Literal 'quote' \\ slash; 100%"},{id:'demonstration',startMs:7000,endMs:10000,text:'Product demonstration'}]}});
 const search=await call({command:'transcript-search',project:'project',query:'demonstration'});if(search.items.length!==1)throw new Error('cue search failed');
 const frame=await call({command:'inspect',project:'project',assetId:'phone',kind:'frame',startMs:8000});if(!fs.existsSync(frame.path))throw new Error('frame not published');
 const cached=await call({command:'inspect',project:'project',assetId:'phone',kind:'frame',startMs:8000});if(!cached.cacheHit)throw new Error('frame cache missed');
 p=await state();
 const compose={command:'compose',project:'project',expectedRevision:p.revision,key:'compose-short',edit:{outputId:'product_demo',name:'Product demonstration',fontAssetId:'font',cuts:[{id:'intro',assetId:'phone',startMs:0,endMs:3000},{id:'demo',assetId:'phone',startMs:7000,endMs:10000}],width:360,height:640,fontSize:24}};
 const edited=await call(compose);if(!edited.createdIds.includes('product_demo'))throw new Error('missing created IDs');
 const requestPath=path.join(root,'retry.json');fs.writeFileSync(requestPath,JSON.stringify(compose));
 const replay=spawnSync(avw,['--ffmpeg',ffmpeg,'--ffprobe',ffprobe,'--workspace',root,'request',requestPath],{encoding:'utf8'});
 if(replay.status!==0||JSON.stringify(JSON.parse(replay.stdout).result)!==JSON.stringify(edited))throw new Error('CLI/MCP retry conformance failed');
 const wrong=await call({...compose,edit:{...compose.edit,name:'different'}},true);if(wrong.error.code!=='E_IDEMPOTENCY_CONFLICT')throw new Error('wrong conflict');
 const job=await call({command:'render-start',project:'project',sequence:'product_demo',expectedRevision:edited.revision,key:'render-short'});
 const replayJob=await call({command:'render-start',project:'project',sequence:'product_demo',expectedRevision:edited.revision,key:'render-short'});if(job.id!==replayJob.id)throw new Error('duplicate job');
 let status;
 for(let i=0;i<240;i++){status=await call({command:'job-status',project:'project',id:job.id});if(['succeeded','failed','cancelled','interrupted'].includes(status.state))break;await new Promise(resolve=>setTimeout(resolve,250));}
 if(status.state!=='succeeded')throw new Error(JSON.stringify(status));
 const artifact=await call({command:'artifact',project:'project',id:job.id});if(!fs.existsSync(artifact.path)||artifact.mimeType!=='video/mp4')throw new Error('artifact unavailable');
 const manifest=JSON.parse(fs.readFileSync(status.result.manifest,'utf8'));if(manifest.verification.expectedFrames!==180)throw new Error('wrong decoded duration');
 const originalOutput=JSON.stringify((await state()).sequences.find(s=>s.id==='product_demo'));
 let finalRevision=edited.revision;
 for(const [outputId,name] of [['hook_b','Alternative opening'],['hook_c','Third short']]) {
   const outcome=await call({...compose,key:'compose-'+outputId,expectedRevision:finalRevision,edit:{...compose.edit,outputId,name,cuts:[...compose.edit.cuts].reverse()}});
   finalRevision=outcome.revision;
 }
 if(JSON.stringify((await state()).sequences.find(s=>s.id==='product_demo'))!==originalOutput)throw new Error('composing variants changed the first output');
 for(const sequence of ['hook_b','hook_c']) {
   const queued=await call({command:'render-start',project:'project',sequence,expectedRevision:finalRevision,key:'render-'+sequence});
   let done;
   for(let i=0;i<240;i++){done=await call({command:'job-status',project:'project',id:queued.id});if(['succeeded','failed','cancelled','interrupted'].includes(done.state))break;await new Promise(resolve=>setTimeout(resolve,250));}
   if(done.state!=='succeeded')throw new Error(JSON.stringify(done));
   await call({command:'artifact',project:'project',id:queued.id});
 }
 const protectedResult=await call({command:'protect',project:'project',expectedRevision:finalRevision,key:'protect-demo',range:{id:'demo',assetId:'phone',sequenceId:'product_demo',start:{value:210,rate:{numerator:30,denominator:1}},end:{value:300,rate:{numerator:30,denominator:1}}}});
 finalRevision=protectedResult.revision;
 const unprotect={command:'unprotect',project:'project',id:'demo',expectedRevision:finalRevision,key:'remove-protection'};
 const removed=await call(unprotect);
 if(JSON.stringify(await call(unprotect))!==JSON.stringify(removed))throw new Error('unprotect replay diverged');
 finalRevision=removed.revision;
 await call({command:'resume',project:'project'});await call({command:'history',project:'project',limit:2});
 const escape=await call({command:'status',project:'../outside'},true);if(escape.ok!==false)throw new Error('workspace escape accepted');
 await call({command:'backup',project:'project',destination:'backup'});
 const reopened=await call({command:'status',project:'backup'});if(reopened.revision!==finalRevision)throw new Error('backup lost revision');
 fs.writeFileSync(path.join(root,'summary.json'),JSON.stringify({passed:true,officialMcpSdk:true,cliMcpOutcomeEquivalent:true,transcriptMappedCaptions:true,independentNamedOutputs:3,cacheReused:true,decodedFrames:180,jobId:job.id,artifact:artifact.path},null,2));
 console.log('Agent interface smoke passed:',path.join(root,'summary.json'));
} finally {await client.close();}
