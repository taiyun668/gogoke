/** Exact C2-F01 source bodies under fake process/storage dependencies; no product import. */
import fs from 'node:fs';import path from 'node:path';import vm from 'node:vm';import assert from 'node:assert/strict';
import {parseFile,anchor,body,ts,sha} from './symbol_tools.mjs';
const [source,out]=process.argv.slice(2);if(!source||!out)throw Error('usage: node targeted_fixtures.mjs SOURCE OUT');
const I=parseFile(source,'packages/room/src/instances.ts'),R=parseFile(source,'packages/room/src/server.ts'),P=parseFile(source,'packages/room/src/proc.ts');
const extracts=[],cases=[];
function compile(p,symbol,args,globals={},async=false){
 const b=body(p,symbol),a=anchor(p,symbol);
 if(!extracts.some(x=>x.path===a.path&&x.symbol===a.symbol))extracts.push({...a,body_sha256:sha(Buffer.from(b))});
 const js=ts.transpileModule(`(${async?'async ':''}function(${args}) ${b})`,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.None}}).outputText;
 return vm.runInNewContext(js,globals,{timeout:1000,contextCodeGeneration:{strings:false,wasm:false}});
}
class FixedDate extends Date {constructor(...args){super(...(args.length?args:['2026-01-01T00:00:00Z']));}static now(){return 1767225600000;}}
async function test(id,fn){try{cases.push({id,status:'PASS_EXPECTED_SOURCE_OBSERVATION',scope:'PINNED_SOURCE_BODIES_WITH_FAKE_DEPENDENCIES',observed:await fn()});}catch(e){cases.push({id,status:'FAIL',error:String(e.stack||e)});}}
for(const mode of ['missing-record','missing-busy','binding-mismatch','missing-pid','query-unknown','managed-success','host-success','managed-save-fails','host-save-fails'])await test('noteProcess/'+mode,()=>{
 let queries=0,saves=0,error=null;
 const record={id:'i',kind:mode.startsWith('host')?'host':'managed',busy:{seatId:mode==='binding-mismatch'?'other':'s'}};
 if(mode==='missing-busy')record.busy=null;
 const self={get:()=>mode==='missing-record'?null:record,hostBusy:{},save:()=>{saves++;if(mode.endsWith('save-fails'))throw Error('fake save failure');}};
 const f=compile(I,'InstanceStore.noteProcess','id,seatId,pid',{processStartedAt:()=>{queries++;return mode==='query-unknown'?null:'known-start';}});
 try{f.call(self,'i','s',mode==='missing-pid'?undefined:41);}catch(e){error=e.message;}
 const early=['missing-record','missing-busy','binding-mismatch','missing-pid'].includes(mode);
 assert.equal(queries,early?0:1);assert.equal(saves,early||mode==='query-unknown'?0:1);assert.equal(!!error,mode.endsWith('save-fails'));
 const b=record.kind==='host'?self.hostBusy.i:record.busy;
 if(saves)assert.equal(b.pid,41);
 return {query_calls:queries,save_attempts:saves,error,memory_pid:b?.pid??null,proof_of_exit:false,durable_owner_confirmed:false};
});
for(const mode of ['invalid-pid','unix-empty','unix-error','unix-known','windows-empty','windows-error'])await test('processStartedAt/'+mode,()=>{
 let calls=0;const f=compile(P,'processStartedAt','pid',{process:{platform:mode.startsWith('windows')?'win32':'linux'},execFileSync:()=>{calls++;if(mode.endsWith('error'))throw Error('fake lookup error');return mode.endsWith('known')?'start-id\n':'';}});
 const value=f(mode==='invalid-pid'?0:41);assert.equal(calls,mode==='invalid-pid'?0:1);assert.equal(value,mode.endsWith('known')?'start-id':null);
 return {fake_command_calls:calls,value,null_does_not_prove_absence:true};
});
for(const mode of ['success','mkdir-error','write-error','rename-error'])await test('InstanceStore.save/'+mode,()=>{
 const calls=[],files=new Map([['/fake/instances.json','old']]);let error=null;
 const f=compile(I,'InstanceStore.save','',{
  mkdirSync:p=>{calls.push(['mkdir',p]);if(mode==='mkdir-error')throw Error('mkdir failed');},
  writeFileSync:(p,b)=>{calls.push(['write',p]);if(mode==='write-error')throw Error('write failed');files.set(p,b);},
  renameSync:(a,b)=>{calls.push(['rename',a,b]);if(mode==='rename-error')throw Error('rename failed');files.set(b,files.get(a));files.delete(a);}
 });
 try{f.call({root:'/fake',file:'/fake/instances.json',records:[{id:'i'}],hostBusy:{}});}catch(e){error=e.message;}
 assert.equal(!!error,mode!=='success');assert.equal(calls.length,mode==='mkdir-error'?1:mode==='write-error'?2:3);
 if(mode==='rename-error'){assert.equal(files.get('/fake/instances.json'),'old');assert(files.has('/fake/instances.json.tmp'));}
 return {calls,error,fixed_temporary_path:'/fake/instances.json.tmp',new_target_visible:mode==='success',real_disk_operations:0};
});
for(const mode of ['no-matching-owner','orphan-and-other-seat','clear-managed-and-host','save-failure'])await test('releaseAllFor/'+mode,()=>{
 const match=mode==='clear-managed-and-host'||mode==='save-failure';
 const records=[{id:'i',busy:{seatId:match?'s':'other'}},{id:'orphan',busy:{seatId:'s',orphaned:true}}];
 const hostBusy={host:{seatId:match?'s':'other'},orphanHost:{seatId:'s',orphaned:true}};let saves=0,error=null;
 const self={records,hostBusy,save:()=>{saves++;if(mode==='save-failure')throw Error('release save failed');}};
 const f=compile(I,'InstanceStore.releaseAllFor','seatId');try{f.call(self,'s');}catch(e){error=e.message;}
 assert.equal(saves,match?1:0);assert.equal(!!records[0].busy,!match);assert.equal('host' in hostBusy,!match);assert(records[1].busy.orphaned&&hostBusy.orphanHost.orphaned);assert.equal(!!error,mode==='save-failure');
 return {save_attempts:saves,error,matching_busy_cleared_in_memory:match,orphaned_busy_preserved:true,process_exit_check_performed:false};
});
for(const mode of ['success','unknown-query-still-publishes','note-save-fails-after-start','home-probe-fails-after-start','start-fails','claim-save-fails-before-marker','release-save-fails-masks-error','pre-claim-error'])await test('ensureSeat-chain/'+mode,async()=>{
 const log=[],record={id:'i',kind:'managed',provider:'codex',state:'ready',label:'synthetic',dir:'/fake/account',busy:null},seats=new Map(),seatState=new Map([['s',{}]]);
 let saves=0,started=false,releaseCalls=0,closeCalls=0,error=null;
 const store={records:[record],hostBusy:{},get:id=>id==='i'?record:null,save:()=>{saves++;log.push('save#'+saves);if(saves===1&&mode==='claim-save-fails-before-marker')throw Error('claim-save');if(saves===2&&['note-save-fails-after-start','release-save-fails-masks-error'].includes(mode))throw Error('note-save');if(saves===3&&mode==='release-save-fails-masks-error')throw Error('release-save');}};
 const claim=compile(I,'InstanceStore.claim','id,seatId',{ROOM_BOOT_ID:'fake-boot',Date:FixedDate});
 const note=compile(I,'InstanceStore.noteProcess','id,seatId,pid',{processStartedAt:()=>mode==='unknown-query-still-publishes'?null:'fake-start-id'});
 const release=compile(I,'InstanceStore.releaseAllFor','seatId');
 store.claim=(...a)=>{log.push('claim');return claim.apply(store,a);};
 store.noteProcess=(...a)=>{log.push('noteProcess');return note.apply(store,a);};
 store.releaseAllFor=(...a)=>{releaseCalls++;log.push('releaseAllFor');return release.apply(store,a);};
 const seat={kind:'persistent',start:async()=>{log.push('seat.start');if(mode==='start-fails')throw Error('start-error');started=true;return {pid:41};},probeEffectiveHome:async()=>{log.push('probeEffectiveHome');if(mode==='home-probe-fails-after-start')throw Error('home-error');return '/fake/account';},getAccountIdentity:()=>({status:'authenticated'}),close:async()=>{closeCalls++;return 0;}};
 const globals={seats,seatState,spec:()=>mode==='pre-claim-error'?null:{instanceId:'i',provider:'codex',providerFamily:'openai',kind:'persistent',role:'owner',access:'readonly'},roleKind:()=> 'owner',createNativeProtocolFixtureSeat:()=>null,process:{env:{}},config:{workspace:'/fake/workspace',seats:[]},instanceStore:store,cliNameFor:()=> 'codex',createHookSeat:()=>seat,push:()=>{log.push('push');},DATA_ROOT:'/fake/data',join:path.posix.join};
 const inner=compile(R,'ensureSeatInner','id,context,claim',globals,true);
 const outer=compile(R,'ensureSeat','id,context',{ensureSeatInner:inner,instanceStore:store},true);
 try{await outer('s',{});}catch(e){error=e.message;}
 const success=['success','unknown-query-still-publishes'].includes(mode);
 assert.equal(!!error,!success);assert.equal(seats.has('s'),success);assert.equal(closeCalls,0);
 assert.equal(releaseCalls,['note-save-fails-after-start','home-probe-fails-after-start','start-fails','release-save-fails-masks-error'].includes(mode)?1:0);
 if(mode==='unknown-query-still-publishes'){assert.equal(record.busy.pid,undefined);assert.equal(seatState.get('s').status,'alive');}
 if(mode==='claim-save-fails-before-marker'){assert(record.busy);assert.equal(started,false);assert.equal(releaseCalls,0);}
 if(mode==='release-save-fails-masks-error')assert.equal(error,'release-save');
 if(releaseCalls)assert.equal(record.busy,null);
 return {log,error,fake_child_started:started,seat_published:seats.has('s'),busy_retained_in_memory:!!record.busy,release_calls:releaseCalls,child_close_calls:closeCalls,meaning:'Observed legacy flow; a successful return, absent PID, or cleared busy record does not prove durable custody or process exit.'};
});
const result={schema:'gogoke.p00.c2-f01.exact-source-fixtures.r3',role:'AUTHOR_SIDE_NOT_INDEPENDENT_REVIEW',parser:ts.version,total:cases.length,passed:cases.filter(x=>x.status==='PASS_EXPECTED_SOURCE_OBSERVATION').length,failed:cases.filter(x=>x.status==='FAIL').length,extracts,cases,product_runtime_verified:false,real_native_processes:false,real_persistence:false,future_contract_model_cases:0,scope:'AST-extracted exact method/function bodies. Fake filesystem, process query, date, instance records and persistent Seat returned by a stub createHookSeat; no provider constructor or top-level server execution. Cases overlap prior review and must not be summed as product acceptance.'};
fs.writeFileSync(out,JSON.stringify(result)+'\n');console.log(JSON.stringify({total:result.total,passed:result.passed,failed:result.failed,failures:cases.filter(x=>x.status==='FAIL')}));process.exitCode=result.failed?1:0;
