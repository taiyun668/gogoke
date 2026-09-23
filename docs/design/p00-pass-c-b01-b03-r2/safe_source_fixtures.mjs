/** Evidence-only fixtures. Reads pinned TS as text, transpiles with local TypeScript, and links ONLY
 * allowlisted synthetic dependencies. No real child, socket, home or FS target.
 * Exact-source observations are separated from future-contract model assertions.
 * Usage: node safe_source_fixtures.mjs SOURCE_ROOT
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve, posix } from 'node:path';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { dirname } from 'node:path';
const require=createRequire(import.meta.url);
const ts=require(process.env.P00_TYPESCRIPT_PATH||resolve(dirname(process.execPath),'../lib/node_modules/typescript'));
const compiled=new Map();
function compile(code){if(!compiled.has(code))compiled.set(code,ts.transpileModule(code,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText);return compiled.get(code);}
import vm from 'node:vm';
import { EventEmitter } from 'node:events';
const root=resolve(process.argv[2]||'');
if(!process.argv[2]) throw Error('SOURCE_ROOT required');
const results=[]; const sources=new Map();
const flush=async()=>{ for(let i=0;i<24;i++) await Promise.resolve(); };
class Clock {
  now=0; serial=0; timers=[];
  setTimeout=(fn,ms)=>{const x={id:++this.serial,at:this.now+ms,fn,cancel:false,unref(){}};this.timers.push(x);return x;};
  clearTimeout=(x)=>{if(x)x.cancel=true;};
  async to(t){await flush(); for(let count=0;;count++){
    if(count>1000)throw Error('fake clock runaway');
    this.timers.sort((a,b)=>a.at-b.at||a.id-b.id);
    const x=this.timers.find(x=>!x.cancel&&x.at<=t);
    if(!x)break;x.cancel=true;this.now=x.at;x.fn();await flush();
  } this.now=t; await flush(); }
}
class FakeChild extends EventEmitter {
  pid=77; exitCode=null; signalCode=null;
  stdout=Object.assign(new EventEmitter(),{setEncoding(){}});
  stderr=Object.assign(new EventEmitter(),{setEncoding(){}});
  stdin={end:()=>{this.eof=true;},write:()=>true}; eof=false; kills=[];
  kill(s){this.kills.push(s);return false;}
}
function text(path){const b=readFileSync(resolve(root,path));sources.set(path,createHash('sha256').update(b).digest('hex'));return b.toString();}
async function environment({platform='linux',ticks='200',fail=null}={}){
 const clock=new Clock(),store=new Map(),fds=new Map(),trace=[],loaded=new Map();let uuid=0,fd=0,readHook=null;
 const fs={
  existsSync:p=>store.has(String(p))||String(p).startsWith('/fake/'),
  mkdirSync:(p)=>trace.push(['mkdir',String(p)]),
  readFileSync:(p)=>{p=String(p);if(p.startsWith('/proc/'))return `77 (fixture) ${Array(19).fill('0').join(' ')} ${ticks}`;
    if(!store.has(p))throw Error('ENOENT '+p);const prior=store.get(p);const h=readHook;readHook=null;if(h)h();return prior;},
  openSync:(p,flags,mode)=>{p=String(p);if(flags==='wx'){assert(!store.has(p));store.set(p,'');}const n=++fd;fds.set(n,{p,flags});trace.push(['open',p,flags,mode]);return n;},
  writeFileSync:(n,s)=>{if(fail==='write')throw Error('injected write');const p=typeof n==='number'?fds.get(n).p:String(n);store.set(p,String(s));trace.push(['write',p]);},
  fsyncSync:(n)=>{const f=fds.get(n);if(fail==='fsync-file'&&f.flags==='wx'||fail==='fsync-dir'&&f.flags==='r')throw Error('injected fsync');trace.push(['fsync',f.p]);},
  closeSync:(n)=>{fds.delete(n);},
  renameSync:(a,b)=>{if(fail==='rename')throw Error('injected rename');store.set(String(b),store.get(String(a)));store.delete(String(a));trace.push(['rename',String(a),String(b)]);}
 };
 const children=[];const childProcess={spawn:(name,args)=>{trace.push(['spawn-mock',clock.now,name,...args]);const c=new FakeChild();children.push(c);clock.setTimeout(()=>{c.exitCode=fail==='kill-command'?1:0;c.emit('exit',c.exitCode);},0);return c;}};
 const sandbox={process:{platform,arch:'x64',pid:11,env:{},cwd:()=>'/audit',exit:code=>{throw Object.assign(Error('HOST_EXIT'),{code});}},
  console:{log:(...x)=>trace.push(['log',...x]),warn:(...x)=>trace.push(['warn',...x]),error:(...x)=>trace.push(['error',...x])},
  setTimeout:clock.setTimeout,clearTimeout:clock.clearTimeout,Buffer,URL,TextDecoder,TextEncoder};
 const context=vm.createContext(sandbox);
 const dependencies={'node:fs':fs,'node:path':posix,'node:crypto':{randomUUID:()=>`fixture-${++uuid}`},'node:os':{homedir:()=>'/real-user'},'node:child_process':childProcess};
 function load(path){
  if(loaded.has(path))return loaded.get(path);
  const allowed=['seat-runtime.ts','seat.ts','close.ts','persistence.ts','claude-seat.ts','grok-acp-seat.ts'];
  if(!allowed.includes(path))throw Error('disallowed module '+path);
  const module={exports:{}};loaded.set(path,module.exports);
  const request=spec=>{
   if(spec.startsWith('./'))return load(spec.slice(2));
   if(!Object.hasOwn(dependencies,spec))throw Error('disallowed native import '+spec);
   return dependencies[spec];
  };
  vm.runInContext('(function(require,module,exports){'+compile(text('packages/seat-runtime/src/'+path))+'\n})',context,{filename:path})(request,module,module.exports);
  return module.exports;
 }
 async function ns(path){return load(path);}
 async function roomFunction(name){const source=text('packages/room/src/server.ts');let code;
  if(name==='shutdown')code=source.slice(source.indexOf('const SHUTDOWN_DEADLINE_MS ='),source.indexOf('\nfor (const sig of',source.indexOf('const SHUTDOWN_DEADLINE_MS =')));
  else code=source.slice(source.indexOf('const SEAT_CLOSE_TIMEOUT_EXIT ='),source.indexOf('\nasync function safelyCloseSeat('));
  assert(code.includes('async function '+name+'('));
  sandbox.seats=new Map();sandbox.seatState=new Map();sandbox.reapLogins=()=>trace.push(['reap-mock']);sandbox.instanceStore={releaseAllFor:id=>trace.push(['release',id])};
  return vm.runInContext('(function(){'+compile(code)+'; return '+name+';})()',context,{filename:'extracted-server-'+name});
 }
 return {clock,store,trace,fs,ns,roomFunction,sandbox,readHook:h=>{readHook=h;},children};
}
async function test(id,scope,fn){console.error('CASE',id);const detail=await fn();results.push({id,scope,status:'PASS_EXPECTED_OBSERVATION',detail});}
for(const exitCode of [0,7])await test('B02-natural-'+exitCode,'EXACT_SOURCE_MOCKED_DEPENDENCIES',async()=>{
 const e=await environment();const close=await e.ns('close.ts');const c=new FakeChild();const r=await close.closeSeatProcess({child:c,exited:Promise.resolve({code:exitCode,signal:null}),timeoutMs:10000,recordedStartTicks:null});assert.equal(r.outcome,'exited');assert.equal(r.exitCode,exitCode);assert(!e.children.length);return {exitCode,os_exit_proven:false};});
for(const failure of [null,'kill-command'])await test('B02-kill-without-exit-'+(failure||'success'),'EXACT_SOURCE_MOCKED_DEPENDENCIES',async()=>{
 const e=await environment({fail:failure});const close=await e.ns('close.ts');const c=new FakeChild();let result;const p=close.closeSeatProcess({child:c,exited:new Promise(()=>{}),timeoutMs:10000,recordedStartTicks:null}).then(r=>result=r);
 await e.clock.to(9999);assert(!e.children.length);await e.clock.to(15000);await p;assert.equal(result.exitCode,124);assert.equal(c.exitCode,null);assert.equal(e.trace.find(x=>x[0]==='spawn-mock')[1],10000);if(failure)assert.deepEqual(c.kills,['SIGKILL']);return {exitCode:124,child_still_live:true,kill_is_not_exit:true,fallback:!!failure};});
await test('B02-refused-identity','EXACT_SOURCE_MOCKED_DEPENDENCIES',async()=>{
 const e=await environment();const close=await e.ns('close.ts');let result;const p=close.closeSeatProcess({child:new FakeChild(),exited:new Promise(()=>{}),timeoutMs:10000,recordedStartTicks:'100'}).then(r=>result=r);await e.clock.to(10000);await p;assert.equal(result.exitCode,125);assert.equal(e.children.length,0);return {exitCode:125,no_kill:true};});
await test('B02-host-8000-before-grace-10000','EXTRACTED_EXACT_CALLER_WITH_FAKE_CLOCK',async()=>{
 const e=await environment();const close=await e.ns('close.ts');const shutdown=await e.roomFunction('shutdown');const c=new FakeChild();e.sandbox.seats.set('a',{close:async()=> (await close.closeSeatProcess({child:c,exited:new Promise(()=>{}),timeoutMs:10000,recordedStartTicks:null})).exitCode});let exited;
 const done=shutdown('fixture').catch(x=>{assert.equal(x.message,'HOST_EXIT');exited=x.code;});await e.clock.to(8000);await done;assert.equal(exited,0);assert.equal(e.children.length,0);assert(e.sandbox.seats.has('a'));return {host_exit_ms:8000,kill_start_ms:10000,host_result:0,child_still_live:true};});
for(const code of [124,125])await test('B02-shutdown-ignores-'+code,'EXTRACTED_EXACT_CALLER_WITH_FAKE_DEPENDENCIES',async()=>{
 const e=await environment();const shutdown=await e.roomFunction('shutdown');e.sandbox.seats.set('a',{close:async()=>code});await assert.rejects(shutdown('fixture'),x=>x.message==='HOST_EXIT'&&x.code===0);assert(!e.sandbox.seats.has('a'));return {numeric_result:code,seat_deleted:true,incorrect_success_log:e.trace.some(x=>x[0]==='log'&&String(x[1]).includes('已关闭'))};});
for(const [file,klass,extra] of [['seat-runtime.ts','PersistentCodexSeat',{codexExecutable:'/fake/codex'}],['grok-acp-seat.ts','PersistentGrokSeat',{grokExecutable:'/fake/grok.exe'}],['claude-seat.ts','PersistentClaudeSeat',{claudeExecutable:'/fake/claude'}]]){
 await test('B02-repeat-close-'+klass,'EXACT_SOURCE_AND_EXTRACTED_CALLER_MOCKED_DEPENDENCIES',async()=>{
  const e=await environment();const mod=await e.ns(file);const obj=new mod[klass]({dataRoot:'/audit/data',seatId:'a',workspace:'/audit/project',...extra});obj.child=new FakeChild();obj.exitPromise=new Promise(()=>{});obj.startTicks=Promise.resolve('100');
  const release=await e.roomFunction('closeAndRelease');e.sandbox.seats.set('a',obj);e.sandbox.seatState.set('a',{runtimeOwner:{pid:77},activeTurn:'t'});let first;
  const done=release('a').then(x=>first=x);await e.clock.to(10000);await done;assert(String(first).includes('身份'));assert(e.sandbox.seats.has('a'));assert.equal(obj.child,null);
  const second=await release('a');assert.equal(second,null);assert(!e.sandbox.seats.has('a'));assert(e.trace.some(x=>x[0]==='release'));return {first:125,second:0,owner_released_on_second_call:true,production_fix_applied:false};
 });
 for(const mode of ['isolated','bound','host-denied','host-explicit'])await test('B03-home-'+klass+'-'+mode,'EXACT_SOURCE_CONSTRUCTOR_WITH_SYNTHETIC_PATHS',async()=>{
  const e=await environment();const mod=await e.ns(file);const opts={dataRoot:'/audit/data',seatId:'a',workspace:'/audit/project',...extra};if(mode==='bound')opts.credentialHome='/audit/profile';if(mode.startsWith('host'))opts.credentialHome='/real-user';if(mode==='host-explicit')opts.allowRealUserHome=true;
  if(mode==='host-denied'){assert.throws(()=>new mod[klass](opts));return {rejected:true,real_home_touched:false};}
  const obj=new mod[klass](opts);assert.equal(obj.seatRoot,'/audit/data/seats/a');const home=obj.home??obj.credentialHome;assert.equal(home,mode==='isolated'?'/audit/data/seats/a/home':mode==='bound'?'/audit/profile':'/real-user');
  const p=obj.writePrompt(1,'synthetic prompt');assert(p.startsWith('/audit/data/seats/a/prompts/'));assert([...e.store.keys()].every(p=>p.startsWith('/audit/data/seats/a/')));return {home,logs_root:obj.seatRoot,prompt_path:p,authorization_proven:false,real_home_touched:false};
 });
}
for(const fail of ['write','fsync-file','rename','fsync-dir'])await test('B03-persistence-'+fail,'EXACT_SOURCE_PERSISTENCE_WITH_FAKE_FILESYSTEM',async()=>{
 const e=await environment({fail});e.store.set('/audit/log','old');const p=await e.ns('persistence.ts');if(fail==='fsync-dir'){p.atomicWriteFile('/audit/log','new');assert.equal(e.store.get('/audit/log'),'new');return {directory_durability_proven:false,error_swallowed:true};}
 assert.throws(()=>p.atomicWriteFile('/audit/log','new'));assert.equal(e.store.get('/audit/log'),'old');assert([...e.store.keys()].some(x=>x.endsWith('.tmp')));return {old_preserved_in_fakefs:true,temp_retained:true,failed_step:fail};});
await test('B03-competing-append-intents','EXACT_HELPER_CONTROLLED_STORAGE_INTERLEAVING_NOT_OS_RACE',async()=>{
 const e=await environment();e.store.set('/audit/events','');const p=await e.ns('persistence.ts');e.readHook(()=>p.appendJsonLineAtomic('/audit/events',{id:'B'}));p.appendJsonLineAtomic('/audit/events',{id:'A'});assert.equal(e.store.get('/audit/events'),'{"id":"A"}\n');return {lost_intent:'B',schedule:'A snapshot -> B publish -> A publish',actual_multiprocess_race_run:false};});
await test('B03-temp-mode-and-rename','EXACT_SOURCE_PERSISTENCE_WITH_FAKE_FILESYSTEM',async()=>{
 const e=await environment();const p=await e.ns('persistence.ts');p.atomicWriteJson('/audit/owner',{pid:77});const o=e.trace.find(x=>x[0]==='open'&&x[2]==='wx');assert.equal(o[3],0o600);assert(e.trace.some(x=>x[0]==='rename'));return {exclusive_temp_mode:'0600',cross_writer_lock_proven:false};});
// Future contract oracle: evidence only, not installed production behavior.
const releasable=r=>r.directExited===true&&r.descendantsExited===true&&r.writerDrained===true&&r.identityMatched===true&&r.persistedReceipt===true&&!r.hostDeadline;
for(const key of ['directExited','descendantsExited','writerDrained','identityMatched','persistedReceipt','hostDeadline'])await test('B02-oracle-reject-'+key,'FUTURE_CONTRACT_MODEL_ONLY',async()=>{
 const r={directExited:true,descendantsExited:true,writerDrained:true,identityMatched:true,persistedReceipt:true,hostDeadline:false};r[key]=!r[key];assert(!releasable(r));return {owner_quarantined:true};});
await test('B02-oracle-release-verified','FUTURE_CONTRACT_MODEL_ONLY',async()=>{assert(releasable({directExited:true,descendantsExited:true,writerDrained:true,identityMatched:true,persistedReceipt:true}));return {release_requires_all_receipts:true};});
const allowWrite=(path,root)=>path===root||path.startsWith(root+'/');
await test('B03-oracle-undeclared-writer','FUTURE_CONTRACT_MODEL_ONLY',async()=>{assert(!allowWrite('/audit/profile/transcript.jsonl','/audit/data/seats/a'));assert(!allowWrite('/audit/data/seats/ab/events.jsonl','/audit/data/seats/a'));assert(allowWrite('/audit/data/seats/a/events.jsonl','/audit/data/seats/a'));return {wrong_home_and_prefix_collision_rejected:true};});

const authorizeHost=(binding,approval)=>Boolean(approval&&approval.explicit===true&&approval.expires>100&&['provider','host','accountId','home','domain','authRevision','generation'].every(k=>binding[k]===approval[k])&&binding.effects.every(x=>approval.effects?.includes(x)));
const binding={provider:'fixture-provider',host:'fixture-host',accountId:'fixture-account',home:'/real-user',domain:'d1',authRevision:'r1',generation:3,effects:['native-auth-use']};
for(const field of ['missing','provider','host','accountId','home','domain','authRevision','generation','expiry','effects','not-explicit'])await test('B03-oracle-host-approval-'+field,'FUTURE_CONTRACT_MODEL_ONLY',async()=>{
 const a={...binding,explicit:true,expires:200};if(field==='expiry')a.expires=1;else if(field==='effects')a.effects=[];else if(field==='not-explicit')a.explicit=false;else if(field!=='missing')a[field]='different';assert(!authorizeHost(binding,field==='missing'?null:a));return {rejected:true,synthetic_home_only:true};});
await test('B03-oracle-host-explicit-bound','FUTURE_CONTRACT_MODEL_ONLY',async()=>{assert(authorizeHost(binding,{...binding,explicit:true,expires:200}));return {approval_binding_required:true,does_not_authorize_secret_migration:true};});
console.log(JSON.stringify({schema:'gogoke.p00.pass-c.safe-fixtures.r2',node:process.version,typescript:ts.version,source_sha256:Object.fromEntries(sources),case_count:results.length,cases:results,real_child_processes_spawned:0,real_network_calls:0,real_home_reads_or_writes:0,product_runtime_verified:false,production_files_changed:false,scope:'SOURCE_ALGORITHMS_WITH_SYNTHETIC_DEPENDENCIES_PLUS_SEPARATE_FUTURE_ORACLES; NOT_NATIVE_PROCESS_OR_DURABILITY_PROOF'},null,2));
