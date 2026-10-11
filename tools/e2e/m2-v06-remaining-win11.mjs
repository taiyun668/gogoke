// Executable installed-product V06 continuation. Root supplies one private
// fixture and invokes this only in the ordinary Interactive/Limited task.
// H model action evidence is verified by the closed immutable reader.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, id, readJson, sha256 } from './product-cdp.mjs';
import { RemainingHostRoot } from './m2-v06-remaining-host.mjs';

const here=path.dirname(fileURLToPath(import.meta.url));
const check=(ok,why)=>{if(!ok)throw Error(why)};
const atom=s=>typeof s==='string'&&/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(s);
const same=(a,b)=>JSON.stringify(a,Object.keys(a??{}).sort())===
  JSON.stringify(b,Object.keys(b??{}).sort());
const configFile=process.argv[2];
check(process.platform==='win32'&&process.argv.length===3&&
  path.isAbsolute(configFile),'Absolute private V06 remaining config required');
const c=readJson(configFile),f=c.v06Remaining;
check(c.testerArmy!==false&&c.repositoryId==='gogokeSeatTestbed'&&
  f?.lifecycleOwnership==='EXCLUSIVE_M2_V06_REMAINING'&&
  f.domainId===c.domainId&&f.instanceId===c.instanceId&&
  ['parentSeatId','childSeatId','parentTreeId','childTreeId','templateId',
    'model','effort','permissionTier'].every(k=>atom(f[k]))&&
  new Set([f.parentSeatId,f.childSeatId,f.parentTreeId,f.childTreeId]).size===4&&
  f.maxConcurrent===1&&
  f.takeover?.questionId==='m2PrivateScope'&&
  f.takeover?.prompt==='Which scope is authorized for this M2 product test?'&&
  f.takeover?.option==='Private testbed only'&&
  c.cliVersion==='0.160.0'&&/^[a-f0-9]{64}$/.test(c.cliSha256)&&
  Array.isArray(c.observers)&&['formal','memory','ledger'].every(name=>
    c.observers.some(row=>row.name===name))&&
  path.isAbsolute(c.evidenceDirectory)&&
  path.parse(c.evidenceDirectory).root[0]?.toUpperCase()==='D'&&
  ['TEMP','TMP'].every(name=>process.env[name]&&
    path.isAbsolute(process.env[name])&&
    path.parse(process.env[name]).root[0]?.toUpperCase()==='D')&&
  c.result===path.join(c.evidenceDirectory,'result.json')&&
  !fs.existsSync(c.evidenceDirectory),
  'Private original installed fixture/identity or fresh output boundary differs');
const oldIds=new Set([c.seatId,c.childSeatId,c.v06NativeUser?.parentSeatId,
  c.v06NativeUser?.childSeatId,c.v06NativeUser?.directSeatId,
  c.v06NativeUser?.deniedChildSeatId,
  ...(c.historyBoundary?.cases??[]).flatMap(row=>
    [row.projectA?.seatId,row.projectB?.seatId,row.sideBinding?.seatId])].filter(Boolean));
check(!oldIds.has(f.parentSeatId)&&!oldIds.has(f.childSeatId),
  'New V06 identities overlap historical V06/V10 subjects');
check(c.observers.length===3&&new Set(c.observers.map(o=>o.name)).size===3,
  'Exactly the three original formal/memory/ledger observers are required');
for(const o of c.observers){
  const pinned=c.observerTools?.[o.name];
  check(pinned&&o.runtime===pinned.runtime&&
    Array.isArray(o.args)&&o.args.includes(pinned.script)&&
    o.args.filter(a=>a==='{output}').length===1&&
    Array.isArray(o.equalFields)&&o.equalFields.length>0&&
    JSON.stringify(o.equalFields)===JSON.stringify(pinned.equalFields)&&
    sha256(pinned.script)===pinned.sha256,
  `Original ${o.name} observer source differs`);
  for(const [name,expected] of Object.entries(pinned.companions??{}))
    check(sha256(path.join(path.dirname(pinned.script),name))===expected,
      `Original ${o.name} observer companion differs: ${name}`);
}
for(const [name,expected] of Object.entries(c.installedSha256))
  check(sha256(path.join(c.installed,name))===expected,
    `Actual installed candidate bytes differ: ${name}`);
const index=readJson(path.join(c.installed,'resource-index.json'));
check(index.sourceCommit===c.sourceCommit&&index.version===c.version,
  'Original installed index/source differs');
const reader=path.join(here,'m2-v06-remaining-readback.py');
const journal={schema:'gogoke.37.m2-v06-remaining-installed.v1',
  entry:'m2-v06-remaining-win11',caseId:id('m2V06Remaining'),
  sourceCommit:c.sourceCommit,installedSha256:c.installedSha256,
  cliSha256:c.cliSha256,installedVersion:c.version,
  preflightSetId:c.installedSha256['resource-index.json'],
  preflightGenerationId:index.generationId,
  state:'RUNNING',acceptance:false,authenticationActions:false,
  credentialReads:false,databaseWritesByReader:false,
  domainId:c.domainId,repositoryId:c.repositoryId,fixture:f,
  operations:[],sessions:[],launches:[],closes:[],snapshots:{},
  modelAttempts:[],takeoverCards:[],notRun:[],readbacks:[]};
const product=new ActualProduct(c,journal);
journal.driverBytes['m2-v06-remaining-win11.mjs']=sha256(path.join(here,'m2-v06-remaining-win11.mjs'));
journal.driverBytes['m2-v06-remaining-host.mjs']=sha256(path.join(here,'m2-v06-remaining-host.mjs'));
journal.driverBytes['m2-v06-remaining-readback.py']=sha256(reader);
journal.configSha256=sha256(configFile);
const root=new RemainingHostRoot(product,c,journal);
let closeAttempted=false;
const childAlive=()=>product.child&&product.child.exitCode===null&&product.child.signalCode===null;
const live=()=>childAlive()&&product.socket?.readyState===WebSocket.OPEN;
const runChild=(runtime,args,label)=>{
  const p=spawnSync(runtime,args,{windowsHide:true,encoding:'utf8',maxBuffer:1024*1024});
  check(p.status===0,`${label} exit=${p.status} signal=${p.signal} `+
    `spawnError=${p.error?.stack??p.error??'none'} stderr=${p.stderr??'none'}`);
};
const snapshot=phase=>{
  for(const o of c.observers){
    const file=path.join(c.evidenceDirectory,`${o.name}-m2-v06-remaining-${phase}.json`);
    check(!fs.existsSync(file),`Original ${o.name} ${phase} snapshot exists`);
    runChild(o.runtime,o.args.map(arg=>arg==='{output}'?file:arg),
      `Original ${o.name} ${phase} observer`);
    journal.snapshots[`${o.name}-${phase}`]={file:path.basename(file),sha256:sha256(file)};
    product.save();
  }
};
const compare=()=>{
  for(const o of c.observers){
    const a=journal.snapshots[`${o.name}-before`],b=journal.snapshots[`${o.name}-after`];
    const ap=path.join(c.evidenceDirectory,a.file),bp=path.join(c.evidenceDirectory,b.file);
    check(sha256(ap)===a.sha256&&sha256(bp)===b.sha256,'Observer bytes changed');
    const av=readJson(ap),bv=readJson(bp);
    for(const field of o.equalFields)
      check(Object.hasOwn(av,field)&&Object.hasOwn(bv,field)&&
        JSON.stringify(av[field])===JSON.stringify(bv[field]),
      `Original ${o.name}.${field} changed`);
    if(o.name==='memory')for(const m of [av,bv])
      check(m.memoryDataUnchangedByRead===true&&m.stage1OutputCount===0&&
        m.memoryJobCount===0,'Original memory observer incomplete');
  }
};
const readback=phase=>{
  const output=path.join(c.evidenceDirectory,`m2-v06-remaining-${phase}.json`);
  runChild(c.python,[reader,c.stateRoot,output,c.result,phase],
    `Original V06 ${phase} closed immutable reader`);
  const proof=readJson(output);
  check(proof.schema==='gogoke.37.private-m2-v06-remaining-readback.v1'&&
    proof.phase===phase&&proof.caseId===journal.caseId&&
    proof.sourceCommit===c.sourceCommit&&proof.readerSha256===journal.driverBytes['m2-v06-remaining-readback.py']&&
    proof.measurementPreservedDatabaseBytes===true&&proof.acceptance===false,
  `Original ${phase} V06 reader facts differ`);
  const ref={phase,file:path.basename(output),sha256:sha256(output)};
  journal.readbacks.push(ref);product.save();return proof;
};
const closeOnce=async()=>{
  check(live()&&!closeAttempted,'Original caption close unavailable or already attempted');
  closeAttempted=true;journal.captionAttemptedAt=new Date().toISOString();product.save();
  await product.closeNormally();
};
const createTree=async(seat,tree)=>{
  const made=await root.op(c.domainId,'K-WORKTREE','create',tree,
    {repositoryId:c.repositoryId,seatId:seat,layout:'single'});
  check(made.reply.result?.state==='CREATED'&&made.reply.revision==='1'&&
    made.reply.result?.seatId===seat,'Original F create receipt differs');
  const registered=await root.op(c.domainId,'K-WORKTREE','register',tree,{},made.reply.revision);
  check(registered.reply.result?.worktreeId===tree,
    'Original F register receipt differs');
  const graph=await root.graph(c.domainId,tree);
  check(graph.result?.state==='REGISTERED'&&graph.result.members?.some(m=>
    m.domainId===c.domainId&&m.repositoryId===c.repositoryId&&
    m.seatId===seat&&m.worktreeId===tree&&m.instanceId===c.instanceId),
  'Original F graph differs');
  (journal.trees??=[]).push({seatId:seat,worktreeId:tree,
    createRequestId:made.entry.request.requestId,
    registerRequestId:registered.entry.request.requestId,graph});product.save();
};
async function main(){
  await product.custody(true);product.verifyBytes();
  fs.mkdirSync(c.evidenceDirectory);
  product.save();
  try{
    snapshot('before');
    const baseline=readback('baseline'); // product closed; no live SQLite
    check(baseline.parentAbsent===true&&baseline.childAbsent===true&&
      baseline.sourceTemplateMatches===true&&baseline.projectCap>0,
    'Original disjoint USER/LEAD/template baseline differs');
    await product.launch();
    const instances=await product.instances();
    check(instances.instances.some(r=>r.instanceId===c.instanceId&&
      r.driverId==='codex'&&r.version===c.cliVersion&&r.state==='LOGGED_IN'),
    'Exact fixed Codex instance is not already LOGGED_IN');
    const parent=(await root.op(c.domainId,'K-SEAT','create-from-template',f.parentSeatId,
      {layer:'USER',templateId:f.templateId})).reply;
    check(parent.result?.layer==='USER'&&parent.result?.state==='IDLE'&&
      parent.result?.templateId===f.templateId&&parent.result?.instanceId===null&&
      parent.revision==='1','Original USER parent create differs');
    journal.parentCreateRequestId=journal.operations.at(-1).request.requestId;product.save();
    const scope=parent.result.settings?.orchestrationScope??{};
    check((scope.maxConcurrent==null)&&
      same(scope.instanceIds,[c.instanceId])&&same(scope.models,[f.model])&&
      same(scope.reasoningEfforts,[f.effort])&&
      scope.maxPermissionTier===f.permissionTier,
    'Original parent copied template scope differs');
    const bound=(await root.op(c.domainId,'K-SEAT','bind-instance',
      f.parentSeatId,{instanceId:c.instanceId},parent.revision)).reply;
    check(bound.result?.layer==='USER'&&bound.result?.state==='IDLE'&&
      bound.result?.instanceId===c.instanceId&&
      BigInt(bound.revision)===BigInt(parent.revision)+1n,
    'Original USER bind-instance receipt differs');
    journal.bindRequestId=journal.operations.at(-1).request.requestId;product.save();
    const bounded=(await root.op(c.domainId,'K-SEAT','set-orchestration-bounds',
      f.parentSeatId,{...scope,maxConcurrent:1},bound.revision)).reply;
    check(bounded.result?.layer==='USER'&&bounded.result?.state==='IDLE'&&
      bounded.result.settings?.orchestrationScope?.maxConcurrent===1&&
      BigInt(bounded.revision)===BigInt(bound.revision)+1n,
    'Original USER bounds1 receipt differs');
    journal.boundsRequestId=journal.operations.at(-1).request.requestId;product.save();
    await createTree(f.parentSeatId,f.parentTreeId);
    const parentH=await root.open(c.domainId,f.parentSeatId,f.parentTreeId,'PARENT_MODEL_CREATE');
    await root.takeover(parentH);
    const createArgs={operation:'create-from-template',targetId:f.childSeatId,
      expectedRevision:'0',payload:{layer:'LEAD',templateId:f.templateId,
        instanceId:c.instanceId}};
    const created=await root.modelTool(parentH,'child-create',createArgs);
    created.expectedStatus='APPLIED';product.save();
    const child=await root.card(c.domainId,f.childSeatId);
    check(child.result?.layer==='LEAD'&&child.result?.state==='IDLE'&&
      child.result?.templateId===f.templateId&&child.result?.instanceId===c.instanceId,
    'Actual native child has no matching fresh E card');
    journal.childCard=child;product.save();
    await root.stopRelease(parentH);
    await createTree(f.childSeatId,f.childTreeId);
    const childH=await root.open(c.domainId,f.childSeatId,f.childTreeId,'BUSY_LEAD');
    await root.takeover(childH);
    const busy=await root.card(c.domainId,f.childSeatId);
    check(busy.result?.state==='BUSY'&&busy.result?.layer==='LEAD',
      'Actual H open did not make exact LEAD BUSY');
    journal.busyCard=busy;product.save();
    journal.notRun.push({axis:'MODEL_LEAD_BOUNDS_DENIAL',state:'NOT_RUN_NO_REGISTERED_TOOL',
      attempted:false,
      reason:'The exact LEAD thread/start omitted native dynamicTools; no bounds-denial request was dispatched.'});
    product.save();
    if(atom(f.changeInstanceId)&&f.changeInstanceId!==c.instanceId&&
       instances.instances.some(r=>r.instanceId===f.changeInstanceId)){
      const change=(await root.op(c.domainId,'K-SEAT','change-instance',f.childSeatId,
        {instanceId:f.changeInstanceId,model:f.model,effort:f.effort,
          permissionTier:f.permissionTier},busy.revision,['CONFLICT'])).reply;
      check(change.revision===busy.revision,
        'BUSY change-instance refusal changed E revision');
      journal.busyChangeRequestId=journal.operations.at(-1).request.requestId;
    }else journal.notRun.push({axis:'BUSY_CHANGE_INSTANCE',
      reason:'No separate already registered logical instance in original fixture.'});
    const reclaimBusy=(await root.op(c.domainId,'K-SEAT','reclaim',f.childSeatId,
      {},busy.revision,['CONFLICT'])).reply;
    check(reclaimBusy.revision===busy.revision,
      'BUSY reclaim refusal changed E revision');
    journal.busyReclaimRequestId=journal.operations.at(-1).request.requestId;product.save();
    const afterBusyRefusals=await root.card(c.domainId,f.childSeatId);
    check(afterBusyRefusals.result?.state==='BUSY'&&
      afterBusyRefusals.revision===busy.revision&&
      afterBusyRefusals.result.generation===busy.result.generation,
    'BUSY User refusals changed E revision or generation');
    journal.afterBusyRefusalsCard=afterBusyRefusals;product.save();
    await root.stopRelease(childH);
    const idle=await root.card(c.domainId,f.childSeatId);
    check(idle.result?.state==='IDLE'&&idle.result?.layer==='LEAD',
      'LEAD not IDLE after same-session H stop/release');
    journal.idleCard=idle;product.save();
    const reclaim=(await root.op(c.domainId,'K-SEAT','reclaim',
      f.childSeatId,{},idle.revision)).reply;
    check(reclaim.result?.state==='RECLAIMED'&&reclaim.result?.layer==='LEAD',
      'Original USER reclaim after H stop/release differs');
    journal.reclaimRequestId=journal.operations.at(-1).request.requestId;product.save();
    journal.notRun.push(
      {axis:'SHORT_TO_LONG',reason:'Original USER template create is fixed Kind::Long; no independent SHORT source configured.'},
      {axis:'MODEL_ADMISSION_CAPACITY',reason:'No independent active H-claim fixture; E child grant cap is a distinct axis.'},
      {axis:'HOST_CHOICES',reason:'No separate original host-choice producer configured.'});
    journal.historicalV06ChildCap=f.priorV06?.path&&f.priorV06?.sha256?
      {state:'HISTORICAL_CANDIDATE49_PROOF_ONLY_NOT_REDISPATCHED',
        path:f.priorV06.path,sha256:f.priorV06.sha256}:
      {state:'NOT_RUN_NO_PINNED_PRIOR_49_PROOF'};
    journal.state='FLOW_COMPLETE_CLOSED_READBACK_REQUIRED';product.save();
    await closeOnce();
    const final=readback('final');
    check(final.directChildCreateEvidence===true&&
      final.directBusyStopReleaseEvidence===true&&
      final.modelLeadBoundsDenial?.state==='NOT_RUN_NO_REGISTERED_TOOL'&&
      JSON.stringify(final.rootIdentity)===JSON.stringify(baseline.rootIdentity)&&
      JSON.stringify(final.candidateIdentity)===JSON.stringify(baseline.candidateIdentity),
    'Original closed H/E/F proof or physical identity differs');
    snapshot('after');compare();
    journal.state='PARTIAL_V06_REMAINING_SLICE_REVIEW_REQUIRED';
    journal.acceptance=false;product.save();
  }catch(error){
    journal.state='FAIL_OR_NOT_RUN_PRESERVE_ORIGINAL';
    journal.originalError=String(error.stack??error);
    journal.currentEndpoint=product.endpoint??null;product.save();
    process.exitCode=1;
    if(live()&&!root.unknown){
      try{await root.settleFailure()}catch(settleError){
        journal.failureSettlementError=String(settleError.stack??settleError);product.save();
      }
    }
    if(live()&&!closeAttempted&&root.sessions.every(s=>s.releaseRequestId)){
      try{await closeOnce()}catch(closeError){
        journal.failureCloseError=String(closeError.stack??closeError);product.save();
      }
    }
    if(childAlive()){
      try{await product.preserveFailure()}catch(preserveError){
        journal.preserveError=String(preserveError.stack??preserveError);product.save();
      }
    }
  }
}
main().catch(error=>{console.error(String(error.stack??error));process.exitCode=1});
