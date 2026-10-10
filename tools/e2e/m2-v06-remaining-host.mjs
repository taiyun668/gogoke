// Actual installed H/User bridge for the bounded V06 continuation.
// Importing this file performs no product action.
import { id, delay } from './product-cdp.mjs';

const check=(ok,why)=>{if(!ok)throw Error(why)};
const atom=s=>typeof s==='string'&&/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(s);
const decimal=s=>typeof s==='string'&&/^(0|[1-9][0-9]*)$/.test(s);

export class RemainingHostRoot {
  constructor(product,config,journal) {
    this.product=product;this.config=config;this.journal=journal;
    this.sessions=[];this.unknown=false;
  }
  save(){this.product.save()}
  async op(domainId,family,operation,targetId,payload={},revision='0',allowed=['APPLIED']) {
    const request={schema:'gogoke.37.operations.v1',family,operation,
      requestId:id('m2V06Remaining'),domainId,targetId,
      expectedRevision:String(revision),payload};
    const entry={request,rawFrame:JSON.stringify(request),startedAt:new Date().toISOString(),
      receipt:null};
    this.journal.operations.push(entry);this.save();
    try {
      entry.rawReceipt=await this.product.evaluate(
        `window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(entry.rawFrame)}})`);
      entry.receipt=JSON.parse(entry.rawReceipt);entry.finishedAt=new Date().toISOString();this.save();
    }catch(error){this.unknown=true;entry.originalError=String(error.stack??error);this.save();throw error}
    const r=entry.receipt;
    if(r?.status==='UNKNOWN')this.unknown=true;
    check(r?.schema===request.schema&&r.family===family&&r.operation===operation&&
      r.requestId===request.requestId&&r.targetId===targetId&&
      decimal(r.previousRevision)&&decimal(r.revision)&&allowed.includes(r.status),
    `Original ${family}/${operation} status=${r?.status}; no replay`);
    return {entry,reply:r};
  }
  async read(domain,family,operation,target,payload={}) {
    let x=await this.op(domain,family,operation,target,payload,'0',['APPLIED','STALE']);
    if(x.reply.status==='STALE')
      x=await this.op(domain,family,operation,target,payload,x.reply.revision);
    return x.reply;
  }
  card(domain,seat){return this.read(domain,'K-SEAT','state-card',seat)}
  graph(domain,tree){return this.read(domain,'K-WORKTREE','graph-query',tree)}
  async h(s,operation,payload={},allowed=['APPLIED']) {
    const {entry,reply}=await this.op(s.domainId,'K-SESSION',operation,s.id,
      {generation:s.generation,...payload},s.revision,allowed);
    s.revision=reply.revision;
    if(reply.result?.newGeneration)s.generation=reply.result.newGeneration;
    this.save();
    return {entry,reply};
  }
  async output(s){
    let x=await this.h(s,'output-stream',{afterCursor:s.cursor},['APPLIED','STALE']);
    if(x.reply.status==='STALE')x=await this.h(s,'output-stream',{afterCursor:s.cursor});
    const result=x.reply.result;
    check(result?.generation===s.generation&&decimal(result.cursor)&&
      BigInt(result.cursor)>=BigInt(s.cursor)&&!result.sourceError,
    'Original H output/cursor/source differs');
    s.cursor=result.cursor;s.events.push(...(result.events??[]));this.save();
    return result;
  }
  async open(domain,seat,tree,phase){
    const card=await this.card(domain,seat),graph=await this.graph(domain,tree);
    check(card.result.state==='IDLE'&&card.result.instanceId===this.config.instanceId&&
      graph.result.state==='REGISTERED'&&graph.result.members?.some(m=>
        m.domainId===domain&&m.seatId===seat&&m.worktreeId===tree&&
        m.repositoryId===this.config.repositoryId&&m.instanceId===this.config.instanceId),
    'Exact E/F binding is not idle and registered');
    const s={id:id('m2V06RemainingH'),domainId,seatId:seat,worktreeId:tree,
      generation:String(BigInt(card.result.generation)+1n),revision:'0',cursor:'0',
      phase,events:[],turns:[],threadId:null,stopFact:null,releaseRequestId:null};
    this.sessions.push(s);this.journal.sessions.push(s);this.save();
    await this.h(s,'admission-reserve',{seatId:seat});s.reserveApplied=true;this.save();
    await this.h(s,'admission-commit',{seatId:seat});s.commitApplied=true;this.save();
    const opened=await this.h(s,'open',{seatId:seat,repositoryId:this.config.repositoryId,
      worktreeId:tree});
    s.openApplied=true;s.openRequestId=opened.entry.request.requestId;this.save();
    check(atom(opened.reply.result?.threadId),'Actual H open has no native thread');
    s.threadId=opened.reply.result.threadId;
    const cap=await this.h(s,'capability-probe');
    check(cap.reply.result?.driverId==='codex'&&
      cap.reply.result?.version===this.config.cliVersion&&
      cap.reply.result?.binaryDigest==='sha256:'+this.config.cliSha256,
    'Actual H process has a different fixed CLI');
    s.processOperationId=cap.reply.result.processOperationId;this.save();return s;
  }
  async completeTurn(s,turnId,answerCards=false){
    const deadline=Date.now()+600000;
    const seen=new Set();
    while(Date.now()<deadline){
      const page=await this.output(s);
      for(const ref of page.nativeCardRefs??[]){
        if(ref.state!=='OPEN'||seen.has(ref.cardId))continue;
        check(answerCards,'Unexpected native C card in one-tool model turn');
        seen.add(ref.cardId);
        const recovered=(await this.op(s.domainId,'K-QCARD','recover',ref.cardId,{},ref.revision)).reply;
        const q=recovered.result?.nativeQuestion;
        const plan=this.config.v06Remaining.takeover;
        check(recovered.result?.availableForAnswer===true&&
          recovered.result?.seatId===s.seatId&&
          recovered.result?.generation===s.generation&&
          q?.threadId===s.threadId&&q?.turnId===turnId&&
          q.questions?.length===1&&q.questions[0].isSecret===false&&
          q.questions[0].id===plan.questionId&&q.questions[0].question===plan.prompt,
        'Original non-secret takeover C question differs');
        const choices=q.questions[0].options.filter(v=>v.label===plan.option||
          v.label===plan.option+' (Recommended)');
        check(choices.length===1,'Authorized original C option is not unique');
        const answered=(await this.op(s.domainId,'K-QCARD','answer',ref.cardId,
          {generation:s.generation,answers:{[plan.questionId]:[choices[0].label]}},
          recovered.revision)).reply;
        check(answered.result?.state==='ANSWERED'&&
          answered.result?.deliveryBasis==='NATIVE_EXACT_WRITE_RECEIPT',
        'Actual C answer lacks native write receipt');
        (this.journal.takeoverCards??=[]).push({sessionId:s.id,cardId:ref.cardId,
          requestId:this.journal.operations.at(-1).request.requestId,turnId,
          option:choices[0].label});this.save();
      }
      const done=s.events.filter(e=>e._meta?.codexMethod==='turn/completed'&&
        e._meta.turnId===turnId&&e._meta.threadId===s.threadId);
      check(done.length<=1,'Duplicate original A completed turn');
      if(done.length===1){
        check(done[0]._meta?.turnStatus==='completed',
          'Original H turn failed; no model replay');
        return;
      }
      await delay(300);
    }
    throw Error('Original H turn completion deadline; no input replay');
  }
  async send(s,body,phase,answerCards=false){
    const sent=await this.h(s,'send',{body});
    check(sent.reply.result?.createdTurn===true&&atom(sent.reply.result?.turnId),
      'Original H send did not create a native turn');
    const turn={phase,sendRequestId:sent.entry.request.requestId,
      turnId:sent.reply.result.turnId,body};
    s.turns.push(turn);this.save();
    await this.completeTurn(s,turn.turnId,answerCards);
    return turn;
  }
  async takeover(s){
    const card=await this.card(s.domainId,s.seatId);
    if(card.result?.takeoverReady===true)return card;
    const plan=this.config.v06Remaining.takeover;
    check(card.result?.takeoverReady===false&&
      card.result.takeoverAnswers?.length===0&&
      card.result.takeoverQuestions?.length===1&&
      card.result.takeoverQuestions[0].id===plan.questionId&&
      card.result.takeoverQuestions[0].prompt===plan.prompt,
    'Original USER takeover contract differs');
    const prompt=`Private Owner-authorized V06 test on this seat ${s.seatId}. `+
      `Call native gogoke_seat state-card on self with expectedRevision ${card.revision}, `+
      'ask its exact takeoverQuestions through native request_user_input, offer the '+
      `non-secret test option ${plan.option}. After the native answer, call `+
      'gogoke_takeover takeover-answers using the state-card nativeAnswerSources. '+
      'Read self state-card and finish only if takeoverReady is true. '+
      'Do not create a child in this turn or invent a receipt.';
    await this.send(s,prompt,'takeover',true);
    const after=await this.card(s.domainId,s.seatId);
    const selected=this.journal.takeoverCards.filter(x=>x.sessionId===s.id);
    check(after.result?.takeoverReady===true&&
      after.result.takeoverAnswers?.some(a=>a.questionId===plan.questionId&&
        a.basis==='CITED'&&selected.some(x=>a.answer===x.option&&
          a.sourceRef?.startsWith(`C-QCARD:${x.cardId}:${x.requestId}:`))),
    'Original C-sourced native takeover was not completed');
    return after;
  }
  async modelTool(s,phase,args){
    check(args?.operation&&atom(args.targetId)&&args.expectedRevision!=null,
      'Exact model tool request missing');
    const prompt=`Use exactly one native gogoke_seat tool call with these exact JSON arguments: ${JSON.stringify(args)}. Do not call another tool or claim a result without the native tool response.`;
    const turn=await this.send(s,prompt,phase);
    const attempt={phase,sessionId:s.id,parentSeatId:s.seatId,domainId:s.domainId,
      targetId:args.targetId,expectedArguments:args,prompt,
      sendRequestId:turn.sendRequestId,turnId:turn.turnId};
    (this.journal.modelAttempts??=[]).push(attempt);this.save();
    return attempt;
  }
  async stopRelease(s){
    check(s.openApplied&&!s.stopFact&&!s.releaseRequestId,
      'Original H stop/release requires one applied open');
    await this.output(s);
    const stopped=await this.h(s,'stop',{seatId:s.seatId});
    check(typeof stopped.reply.result?.stopFact==='string'&&
      stopped.reply.result.stopFact.startsWith('sha256:'),
    'Original H physical StopFact missing');
    s.stopFact=stopped.reply.result.stopFact;
    s.stopRequestId=stopped.entry.request.requestId;this.save();
    const released=await this.h(s,'admission-release',{seatId:s.seatId});
    s.releaseRequestId=released.entry.request.requestId;this.save();
    return {sessionId:s.id,seatId:s.seatId,stopFact:s.stopFact,
      stopRequestId:s.stopRequestId,releaseRequestId:s.releaseRequestId};
  }
  async settleFailure(){
    const results=[];
    for(const s of this.sessions){
      if(s.releaseRequestId){results.push({sessionId:s.id,state:'ALREADY_RELEASED'});continue}
      if(this.unknown){results.push({sessionId:s.id,state:'UNKNOWN_NO_REPLAY'});continue}
      const attempts=name=>this.journal.operations.some(x=>x.request?.family==='K-SESSION'&&
        x.request.operation===name&&x.request.targetId===s.id);
      try{
        if(!s.reserveApplied){
          results.push({sessionId:s.id,state:'NO_RESERVE_APPLIED_NO_STOP_FACT'});continue;
        }
        if(s.openApplied&&!s.stopFact){
          check(!attempts('stop'),'Original stop already dispatched with unknown result');
          await this.output(s);
          const stopped=await this.h(s,'stop',{seatId:s.seatId});
          check(typeof stopped.reply.result?.stopFact==='string'&&
            stopped.reply.result.stopFact.startsWith('sha256:'),
          'Failure StopFact missing');
          s.stopFact=stopped.reply.result.stopFact;
          s.stopRequestId=stopped.entry.request.requestId;this.save();
        }
        if(!s.releaseRequestId){
          check(!attempts('admission-release'),'Original release already dispatched unknown');
          const released=await this.h(s,'admission-release',{seatId:s.seatId});
          s.releaseRequestId=released.entry.request.requestId;this.save();
        }
        results.push({sessionId:s.id,state:'ORIGINAL_STOP_RELEASE_APPLIED'});
      }catch(error){results.push({sessionId:s.id,state:'NOT_SETTLED_KEEP_PRODUCT',
        error:String(error.stack??error)})}
    }
    this.journal.failureSettlement=results;this.save();return results;
  }
}
