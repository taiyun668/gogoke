//! Durable, candidate-only DreamRun and DreamProposal records.
//! No operation in this module promotes a candidate or writes production facts.
use super::catalog::current_profile;
use super::model::{denied,identifier,revision};
use super::transaction::{self,Result,Transaction};
use super::super::atomic::{DomainRecordInput,DomainRecordReceipt};
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct DreamObjectRef { pub object_type:String,pub object_id:String,pub revision:String,pub content_hash:String }
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct DreamEvaluationRef { pub evaluation_id:String,pub revision:String,pub content_hash:String }
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct DreamBudgetLease { pub lease_ref:String,pub operation_id:String,pub resource_ref:String,pub resource_revision:String,pub units:u64 }
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct DreamAllowedChange { pub key:String,pub before_hash:String,pub after_hash:String }
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct DreamVersionRef { pub revision:String,pub content_hash:String }
#[derive(Clone,Debug)]
pub(crate) struct AppendDreamRun {
    pub domain_id:String,pub run_id:String,pub revision:String,pub operation_id:String,pub event_id:String,pub receipt_id:String,pub recorded_at:String,
    pub source_identity:String,pub input_snapshot:DreamObjectRef,pub dataset_namespace:String,pub dataset_split:String,pub dataset_split_hash:String,
    pub recipe_ref:DreamObjectRef,pub budget_lease:DreamBudgetLease,pub evaluation_refs:Vec<DreamEvaluationRef>,pub previous:Option<DreamVersionRef>,
}
#[derive(Clone,Debug)]
pub(crate) struct AppendDreamProposal {
    pub domain_id:String,pub proposal_id:String,pub revision:String,pub operation_id:String,pub event_id:String,pub receipt_id:String,pub recorded_at:String,
    pub source_identity:String,pub run_ref:DreamObjectRef,pub candidate_kind:String,pub before_hash:String,pub after_hash:String,
    pub allowed_change_set:Vec<DreamAllowedChange>,pub heldout_receipt:Option<DreamEvaluationRef>,pub rollback_ref:DreamObjectRef,
    pub base_policy_revision:String,pub namespace:String,pub test_only:bool,pub activation_grant:Option<String>,pub previous:Option<DreamVersionRef>,
}
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct DreamReceipt { pub domain_id:String,pub object_id:String,pub revision:String,pub content_hash:String,pub canonical_record:Vec<u8> }

#[derive(Clone, Debug)]
pub(crate) struct R2TestRollbackPlan {
    pub disposition: &'static str,
    pub reference: DreamObjectRef,
    pub before_hash: String,
    pub after_hash: String,
}

/// Fixed public-fixture rollback reference for the R2 test candidate. It
/// records a proposal reversal only; no production parameter is changed.
pub(crate) fn prepare_r2_test_rollback_plan(
    c:&mut VerifiedDatabaseConnection<'_>, recorded_at:&str,
)->Result<R2TestRollbackPlan>{
    let domain="domain-r2-02-test";
    let id="rollback-r2-02-test";
    let before_hash=content_hash(b"gogoke.r2-02.fixture.parameter.temperature=0");
    let after_hash=content_hash(b"gogoke.r2-02.fixture.parameter.temperature=0.1");
    let bytes=obj(vec![
        ("afterHash".into(),q(&after_hash)),
        ("beforeHash".into(),q(&before_hash)),
        ("changeKey".into(),q("candidate.parameter.temperature")),
        ("domainId".into(),q(domain)),
        ("labelSource".into(),q("TEST_ONLY_ROLLBACK_PLAN")),
        ("planId".into(),q(id)),
        ("revision".into(),q("1")),
        ("rollbackAction".into(),q("RESTORE_BEFORE")),
        ("testOnly".into(),"true".into()),
    ]).into_bytes();
    let receipt=transaction::run(c,|tx| tx.apply_domain_record(DomainRecordInput{
        domain_id:domain.into(),object_type:"RollbackPlan".into(),object_id:id.into(),object_version:"1".into(),object_bytes:bytes,native_identity:None,
        event_id:"r2-02-rollback-event".into(),stream_id:"gogoke.r2-02.rollback-plan.v1/rollback-r2-02-test".into(),expected_previous_counter:None,counter:"0".into(),
        event_type:"RollbackPlanRecorded".into(),occurred_at:recorded_at.into(),event_bytes:obj(vec![("planId".into(),q(id)),("testOnly".into(),"true".into())]).into_bytes(),
        receipt_id:"r2-02-rollback-receipt".into(),operation_id:"r2-02-rollback".into(),receipt_type:"RollbackPlanRecorded".into(),recorded_at:recorded_at.into(),receipt_bytes:obj(vec![("planId".into(),q(id)),("schema".into(),q("gogoke.r2-02.rollback-plan.v1"))]).into_bytes(),
    }))?;
    Ok(R2TestRollbackPlan{disposition:receipt.disposition,reference:DreamObjectRef{object_type:"RollbackPlan".into(),object_id:id.into(),revision:"1".into(),content_hash:receipt.object_hash},before_hash,after_hash})
}

fn hash(v:&str)->Result<()>{if v.len()!=71||!v.starts_with("sha256:")||!v.as_bytes()[7..].iter().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(b)){return denied();}Ok(())}
fn q(v:&str)->String{let mut o=String::from("\"");for c in v.chars(){match c{'"'=>o.push_str("\\\""),'\\'=>o.push_str("\\\\"),'\n'=>o.push_str("\\n"),'\r'=>o.push_str("\\r"),'\t'=>o.push_str("\\t"),c if (c as u32)<0x20=>o.push_str(&format!("\\u{:04x}",c as u32)),c=>o.push(c)}}o.push('"');o}
fn obj(mut fs:Vec<(String,String)>)->String{fs.sort_by(|a,b|a.0.encode_utf16().cmp(b.0.encode_utf16()));let body=fs.into_iter().map(|(k,v)|format!("{}:{v}",q(&k))).collect::<Vec<_>>().join(",");format!("{{{body}}}")}
fn jt(tx:&mut Transaction<'_,'_>,json:&str,path:&str)->Result<String>{let r=tx.query("SELECT json_type(?,?),json_extract(?,?)",&[json,path,json,path],2)?;if r.len()!=1||r[0][0]!="text"||r[0][1].is_empty(){return denied();}Ok(r[0][1].clone())}
fn raw_json(tx:&mut Transaction<'_,'_>,json:&str,path:&str)->Result<String>{let r=tx.query("SELECT json_extract(?,?)",&[json,path],1)?;if r.len()!=1||r[0][0].is_empty(){return denied();}Ok(r[0][0].clone())}
fn verify_record(tx:&mut Transaction<'_,'_>,domain:&str,r:&DreamObjectRef,expected_receipt:&str)->Result<String>{
    identifier(&r.object_id)?;revision(&r.revision)?;hash(&r.content_hash)?;
    let rows=tx.query("SELECT o.content_hash,CAST(o.canonical_json AS TEXT),r.receipt_type,r.content_hash,CAST(r.canonical_json AS TEXT),r.operation_id FROM main.gogoke_objects o JOIN main.gogoke_receipts r ON r.domain_id=o.domain_id AND r.object_type=o.object_type AND r.object_id=o.object_id AND r.object_version=o.object_version WHERE o.domain_id=? AND o.object_type=? AND o.object_id=? AND o.object_version=?",&[domain,&r.object_type,&r.object_id,&r.revision],6)?;
    if rows.len()!=1||rows[0][0]!=r.content_hash||rows[0][2]!=expected_receipt||content_hash(rows[0][1].as_bytes())!=rows[0][0]||content_hash(rows[0][4].as_bytes())!=rows[0][3]{return denied();}Ok(rows[0][1].clone())
}
fn validate_snapshot(tx:&mut Transaction<'_,'_>,domain:&str,r:&DreamObjectRef)->Result<()> {
    if r.object_type!="ContextManifest"{return denied();}let body=verify_record(tx,domain,r,"ContextManifestCommitted")?;
    if jt(tx,&body,"$.manifestId")?!=r.object_id||jt(tx,&body,"$.domainId")?!=domain{return denied();}
    let snap=tx.query("SELECT operation_id FROM main.gogoke_context_assembly_snapshots WHERE domain_id=? AND manifest_id=?",&[domain,&r.object_id],1)?;
    if snap.len()!=1{return denied();}Ok(())
}
fn validate_recipe(tx:&mut Transaction<'_,'_>,domain:&str,r:&DreamObjectRef)->Result<()> {
    if r.object_type!="ExecutionRecipe"{return denied();}let body=verify_record(tx,domain,r,"ExecutionRecipeVersionCommitted")?;
    if jt(tx,&body,"$.recipeId")?!=r.object_id||jt(tx,&body,"$.revision")?!=r.revision{return denied();}
    let head=tx.query("SELECT recipe_revision,content_hash FROM main.gogoke_execution_recipe_heads WHERE domain_id=? AND recipe_id=?",&[domain,&r.object_id],2)?;
    if head.len()!=1||head[0][0]!=r.revision||head[0][1]!=r.content_hash{return denied();}Ok(())
}
fn validate_budget(tx:&mut Transaction<'_,'_>,domain:&str,b:&DreamBudgetLease)->Result<()> {
    identifier(&b.lease_ref)?;identifier(&b.operation_id)?;identifier(&b.resource_ref)?;revision(&b.resource_revision)?;
    if b.units==0||b.units>9_007_199_254_740_991{return denied();}
    let rows=tx.query("SELECT resource_reservation_ref,operation_id,resource_ref,resource_revision,CAST(units AS TEXT) FROM main.gogoke_decision_capacity_leases WHERE resource_reservation_ref=?",&[&b.lease_ref],5)?;
    if rows.len()!=1||rows[0][0]!=b.lease_ref||rows[0][1]!=b.operation_id||rows[0][2]!=b.resource_ref||rows[0][3]!=b.resource_revision||rows[0][4]!=b.units.to_string(){return denied();}
    let decision=super::decision_replay::read_in_transaction(tx,domain,&b.operation_id)?;
    if decision.resource_reservation_ref!=b.lease_ref||decision.record.operation_id!=b.operation_id{return denied();}
    Ok(())
}
fn validate_evals(tx:&mut Transaction<'_,'_>,domain:&str,refs:&[DreamEvaluationRef],namespace:&str,split:&str,split_hash:&str,snapshot:&DreamObjectRef)->Result<()> {
    if refs.is_empty()||refs.len()>64{return denied();}let mut xs=refs.to_vec();for r in &xs{identifier(&r.evaluation_id)?;revision(&r.revision)?;hash(&r.content_hash)?;}xs.sort_by(|a,b|(&a.evaluation_id,&a.revision).cmp(&(&b.evaluation_id,&b.revision)));
    if xs.windows(2).any(|p|p[0].evaluation_id==p[1].evaluation_id&&p[0].revision==p[1].revision){return denied();}
    for r in &xs{let d=DreamObjectRef{object_type:"EvaluationRecord".into(),object_id:r.evaluation_id.clone(),revision:r.revision.clone(),content_hash:r.content_hash.clone()};let json=verify_record(tx,domain,&d,"EvaluationRecorded")?;
        if jt(tx,&json,"$.labelSource")?!="DURABLE_EVALUATION"||jt(tx,&json,"$.evaluationId")?!=r.evaluation_id||jt(tx,&json,"$.revision")?!=r.revision
            ||jt(tx,&json,"$.datasetNamespace")?!=namespace||jt(tx,&json,"$.datasetSplit")?!=split{return denied();}
        let outcomes=tx.query("SELECT json_extract(value,'$.outcomeId'),json_extract(value,'$.revision'),json_extract(value,'$.contentHash') FROM json_each(?,'$.outcomeRefs')",&[&json],3)?;
        if outcomes.is_empty(){return denied();}
        for outcome in outcomes {
            let source=DreamObjectRef{object_type:"OutcomeRecord".into(),object_id:outcome[0].clone(),revision:outcome[1].clone(),content_hash:outcome[2].clone()};
            let outcome_json=verify_record(tx,domain,&source,"ObjectiveOutcomeAppended")?;
            if jt(tx,&outcome_json,"$.labelSource")?!="OBJECTIVE"||jt(tx,&outcome_json,"$.domainId")?!=domain
                ||jt(tx,&outcome_json,"$.manifestId")?!=snapshot.object_id||jt(tx,&outcome_json,"$.manifestVersion")?!=snapshot.revision{return denied();}
            let manifest_hash=jt(tx,&outcome_json,"$.manifestHash")?;
            if super::objective_outcome::validate_manifest(tx,domain,&snapshot.object_id,&snapshot.revision,&manifest_hash)?.0!=snapshot.content_hash{return denied();}
        }
    }
    // Evaluation v1 has no datasetSplitHash field. The Dream request carries
    // its typed hash; the run records it verbatim alongside the exact split
    // label and Evaluation refs, without claiming it was sourced from them.
    hash(split_hash)?;
    Ok(())
}
fn validate_run(tx:&mut Transaction<'_,'_>,x:&AppendDreamRun)->Result<()> {
    for s in [&x.domain_id,&x.run_id,&x.operation_id,&x.event_id,&x.receipt_id,&x.source_identity,&x.dataset_namespace,&x.dataset_split]{identifier(s)?;}
    revision(&x.revision)?;
    hash(&x.dataset_split_hash)?;validate_snapshot(tx,&x.domain_id,&x.input_snapshot)?;validate_recipe(tx,&x.domain_id,&x.recipe_ref)?;
    validate_budget(tx,&x.domain_id,&x.budget_lease)?;validate_evals(tx,&x.domain_id,&x.evaluation_refs,&x.dataset_namespace,&x.dataset_split,&x.dataset_split_hash,&x.input_snapshot)
}
fn eval_array(rs:&[DreamEvaluationRef])->String{let mut xs=rs.to_vec();xs.sort_by(|a,b|(&a.evaluation_id,&a.revision).cmp(&(&b.evaluation_id,&b.revision)));format!("[{}]",xs.iter().map(|r|obj(vec![("contentHash".into(),q(&r.content_hash)),("evaluationId".into(),q(&r.evaluation_id)),("revision".into(),q(&r.revision))])).collect::<Vec<_>>().join(","))}
fn run_body(x:&AppendDreamRun)->Vec<u8>{obj(vec![("budgetLease".into(),obj(vec![("leaseRef".into(),q(&x.budget_lease.lease_ref)),("operationId".into(),q(&x.budget_lease.operation_id)),("resourceRef".into(),q(&x.budget_lease.resource_ref)),("resourceRevision".into(),q(&x.budget_lease.resource_revision)),("units".into(),q(&x.budget_lease.units.to_string()))])),("datasetNamespace".into(),q(&x.dataset_namespace)),("datasetSplit".into(),q(&x.dataset_split)),("datasetSplitHash".into(),q(&x.dataset_split_hash)),("domainId".into(),q(&x.domain_id)),("evaluationRefs".into(),eval_array(&x.evaluation_refs)),("inputSnapshot".into(),obj(vec![("contentHash".into(),q(&x.input_snapshot.content_hash)),("objectId".into(),q(&x.input_snapshot.object_id)),("objectType".into(),q(&x.input_snapshot.object_type)),("revision".into(),q(&x.input_snapshot.revision))])),("labelSource".into(),q("DURABLE_DREAM_RUN")),("recipeRef".into(),obj(vec![("contentHash".into(),q(&x.recipe_ref.content_hash)),("objectId".into(),q(&x.recipe_ref.object_id)),("objectType".into(),q(&x.recipe_ref.object_type)),("revision".into(),q(&x.recipe_ref.revision))])),("revision".into(),q(&x.revision)),("runId".into(),q(&x.run_id)),("sourceIdentity".into(),q(&x.source_identity)),("testOnly".into(),"true".into())]).into_bytes()}
fn validate_proposal(tx:&mut Transaction<'_,'_>,x:&AppendDreamProposal)->Result<(String,Vec<DreamEvaluationRef>,String,String,String)>{
    for s in [&x.domain_id,&x.proposal_id,&x.operation_id,&x.event_id,&x.receipt_id,&x.source_identity,&x.namespace]{identifier(s)?;}
    revision(&x.revision)?;
    if !x.test_only||x.activation_grant.is_some()||!x.namespace.starts_with("test/")||x.candidate_kind!="PARAMETER_TUNING"{return denied();}
    revision(&x.base_policy_revision)?;hash(&x.before_hash)?;hash(&x.after_hash)?;if x.before_hash==x.after_hash{return denied();}
    let profile=current_profile(tx)?;if profile.policy_revision!=x.base_policy_revision{return denied();}
    if x.run_ref.object_type!="DreamRun"{return denied();}let run=verify_record(tx,&x.domain_id,&x.run_ref,"DreamRunRecorded")?;
    let run_id=jt(tx,&run,"$.runId")?;let run_domain=jt(tx,&run,"$.domainId")?;let run_test_only=raw_json(tx,&run,"$.testOnly")?;if run_id!=x.run_ref.object_id||run_domain!=x.domain_id||run_test_only!="1"{return denied();}
    let namespace=jt(tx,&run,"$.datasetNamespace")?;let split=jt(tx,&run,"$.datasetSplit")?;let split_hash=jt(tx,&run,"$.datasetSplitHash")?;
    let evals=tx.query("SELECT json_extract(value,'$.evaluationId'),json_extract(value,'$.revision'),json_extract(value,'$.contentHash') FROM json_each(?,'$.evaluationRefs') ORDER BY CAST(key AS INTEGER)",&[&run],3)?;
    let refs=evals.iter().map(|r|DreamEvaluationRef{evaluation_id:r[0].clone(),revision:r[1].clone(),content_hash:r[2].clone()}).collect::<Vec<_>>();
    let snapshot_json=raw_json(tx,&run,"$.inputSnapshot")?;
    let snapshot=DreamObjectRef{object_type:jt(tx,&snapshot_json,"$.objectType")?,object_id:jt(tx,&snapshot_json,"$.objectId")?,revision:jt(tx,&snapshot_json,"$.revision")?,content_hash:jt(tx,&snapshot_json,"$.contentHash")?};
    validate_snapshot(tx,&x.domain_id,&snapshot)?;
    validate_evals(tx,&x.domain_id,&refs,&namespace,&split,&split_hash,&snapshot)?;
    if x.heldout_receipt.as_ref().is_some_and(|h|!refs.contains(h)){return denied();}
    if refs.is_empty(){return denied();}
    let rollback=verify_record(tx,&x.domain_id,&x.rollback_ref,"RollbackPlanRecorded")?;if x.rollback_ref.object_type!="RollbackPlan"||jt(tx,&rollback,"$.domainId")?!=x.domain_id{return denied();}
    if x.allowed_change_set.is_empty()||x.allowed_change_set.len()>32{return denied();}
    let mut changes=x.allowed_change_set.clone();changes.sort_by(|a,b|a.key.cmp(&b.key));
    if changes.iter().any(|c|!c.key.starts_with("candidate.parameter.")||c.key.len()>128||c.before_hash==c.after_hash||hash(&c.before_hash).is_err()||hash(&c.after_hash).is_err())||changes.windows(2).any(|p|p[0].key==p[1].key){return denied();}
    Ok((run_id,refs,namespace,split,split_hash))
}
fn proposal_body(x:&AppendDreamProposal,run_id:&str,evals:&[DreamEvaluationRef],dataset_namespace:&str,dataset_split:&str,dataset_split_hash:&str)->Vec<u8>{let held=x.heldout_receipt.as_ref().map(|r|obj(vec![("contentHash".into(),q(&r.content_hash)),("evaluationId".into(),q(&r.evaluation_id)),("revision".into(),q(&r.revision))])).unwrap_or_else(||"null".into());let mut changes=x.allowed_change_set.clone();changes.sort_by(|a,b|a.key.cmp(&b.key));let change_json=format!("[{}]",changes.iter().map(|c|obj(vec![("afterHash".into(),q(&c.after_hash)),("beforeHash".into(),q(&c.before_hash)),("key".into(),q(&c.key))])).collect::<Vec<_>>().join(","));obj(vec![("activationGrant".into(),"null".into()),("afterHash".into(),q(&x.after_hash)),("allowedChangeSet".into(),change_json),("basePolicyRevision".into(),q(&x.base_policy_revision)),("beforeHash".into(),q(&x.before_hash)),("candidateKind".into(),q(&x.candidate_kind)),("datasetNamespace".into(),q(dataset_namespace)),("datasetSplit".into(),q(dataset_split)),("datasetSplitHash".into(),q(dataset_split_hash)),("domainId".into(),q(&x.domain_id)),("evaluationRefs".into(),eval_array(evals)),("heldoutReceipt".into(),held),("labelSource".into(),q("DURABLE_DREAM_PROPOSAL")),("namespace".into(),q(&x.namespace)),("proposalId".into(),q(&x.proposal_id)),("revision".into(),q(&x.revision)),("rollbackRef".into(),obj(vec![("contentHash".into(),q(&x.rollback_ref.content_hash)),("objectId".into(),q(&x.rollback_ref.object_id)),("objectType".into(),q(&x.rollback_ref.object_type)),("revision".into(),q(&x.rollback_ref.revision))])),("runId".into(),q(run_id)),("runRef".into(),obj(vec![("contentHash".into(),q(&x.run_ref.content_hash)),("objectId".into(),q(&x.run_ref.object_id)),("objectType".into(),q(&x.run_ref.object_type)),("revision".into(),q(&x.run_ref.revision))])),("sourceIdentity".into(),q(&x.source_identity)),("state".into(),q("DRAFT")),("testOnly".into(),"true".into())]).into_bytes()}
fn append_record(tx:&mut Transaction<'_,'_>,domain:&str,id:&str,revision_value:&str,operation:&str,event_id:&str,receipt_id:&str,recorded:&str,object_type:&str,event_type:&str,receipt_type:&str,bytes:Vec<u8>,previous:Option<&DreamVersionRef>)->Result<DomainRecordReceipt>{
    let replay=tx.query("SELECT receipt_id,event_id,object_type,object_id,object_version FROM main.gogoke_receipts WHERE domain_id=? AND operation_id=?",&[domain,operation],5)?;
    if !replay.is_empty()&&(replay.len()!=1||replay[0][0]!=receipt_id||replay[0][1]!=event_id||replay[0][2]!=object_type||replay[0][3]!=id||replay[0][4]!=revision_value){return Err(OrchestrationError::OperationConflict);}
    let existing=tx.query("SELECT CAST(canonical_json AS TEXT) FROM main.gogoke_objects WHERE domain_id=? AND object_type=? AND object_id=? AND object_version=?",&[domain,object_type,id,revision_value],1)?;
    if !replay.is_empty()&&(existing.len()!=1||existing[0][0].as_bytes()!=bytes.as_slice()){return Err(OrchestrationError::OperationConflict);}
    if replay.is_empty(){
      let head=tx.query("SELECT object_version,content_hash FROM main.gogoke_objects WHERE domain_id=? AND object_type=? AND object_id=? ORDER BY length(object_version) DESC,object_version DESC LIMIT 1",&[domain,object_type,id],2)?;
      match (previous,head.first()){
        (None,None) if revision_value=="1"=>{},
        (Some(p),Some(h)) if revision(&p.revision)?.checked_add(1)==Some(revision(revision_value)?)&&h[0]==p.revision&&h[1]==p.content_hash=>{hash(&p.content_hash)?},
        _=>return Err(OrchestrationError::OperationConflict)
      }
    }
    let prev=previous.map(|r|q(&r.content_hash)).unwrap_or_else(||"null".into());let event=obj(vec![("contentHash".into(),q(&content_hash(&bytes))),("objectId".into(),q(id)),("previousHash".into(),prev),("revision".into(),q(revision_value))]);let receipt=obj(vec![("schema".into(),q(receipt_type)),("sourceIdentity".into(),q("PRODUCT_AUTHORITY"))]);
    tx.apply_domain_record(DomainRecordInput{domain_id:domain.into(),object_type:object_type.into(),object_id:id.into(),object_version:revision_value.into(),object_bytes:bytes,native_identity:None,event_id:event_id.into(),stream_id:format!("gogoke.{}.v1/{id}",if object_type=="DreamRun"{"dream-run"}else{"dream-proposal"}),expected_previous_counter:revision(revision_value)?.checked_sub(2).map(|n|n.to_string()),counter:(revision(revision_value)?-1).to_string(),event_type:event_type.into(),occurred_at:recorded.into(),event_bytes:event.into_bytes(),receipt_id:receipt_id.into(),operation_id:operation.into(),receipt_type:receipt_type.into(),recorded_at:recorded.into(),receipt_bytes:receipt.into_bytes()})
}
fn append_run_tx(tx:&mut Transaction<'_,'_>,x:&AppendDreamRun)->Result<DomainRecordReceipt>{validate_run(tx,x)?;let bytes=run_body(x);append_record(tx,&x.domain_id,&x.run_id,&x.revision,&x.operation_id,&x.event_id,&x.receipt_id,&x.recorded_at,"DreamRun","DreamRunRecorded","DreamRunRecorded",bytes,x.previous.as_ref())}
fn append_proposal_tx(tx:&mut Transaction<'_,'_>,x:&AppendDreamProposal)->Result<DomainRecordReceipt>{let(run,evals,namespace,split,split_hash)=validate_proposal(tx,x)?;let bytes=proposal_body(x,&run,&evals,&namespace,&split,&split_hash);append_record(tx,&x.domain_id,&x.proposal_id,&x.revision,&x.operation_id,&x.event_id,&x.receipt_id,&x.recorded_at,"DreamProposal","DreamProposalRecorded","DreamProposalRecorded",bytes,x.previous.as_ref())}
pub(crate) fn append_dream_run(c:&mut VerifiedDatabaseConnection<'_>,x:&AppendDreamRun)->Result<DomainRecordReceipt>{transaction::run(c,|tx|append_run_tx(tx,x))}
pub(crate) fn append_dream_proposal(c:&mut VerifiedDatabaseConnection<'_>,x:&AppendDreamProposal)->Result<DomainRecordReceipt>{transaction::run(c,|tx|append_proposal_tx(tx,x))}
pub(crate) fn read_dream_run(c:&mut VerifiedDatabaseConnection<'_>,domain:&str,id:&str,revision_value:&str)->Result<DreamReceipt>{read_record(c,domain,id,revision_value,"DreamRun","DreamRunRecorded")}
pub(crate) fn read_dream_proposal(c:&mut VerifiedDatabaseConnection<'_>,domain:&str,id:&str,revision_value:&str)->Result<DreamReceipt>{read_record(c,domain,id,revision_value,"DreamProposal","DreamProposalRecorded")}
fn read_record(c:&mut VerifiedDatabaseConnection<'_>,domain:&str,id:&str,ver:&str,kind:&str,receipt_type:&str)->Result<DreamReceipt>{identifier(domain)?;identifier(id)?;revision(ver)?;transaction::run(c,|tx|{let r=tx.query("SELECT o.content_hash,CAST(o.canonical_json AS TEXT),rec.content_hash,CAST(rec.canonical_json AS TEXT),rec.object_type,rec.object_id,rec.object_version,rec.receipt_type FROM main.gogoke_objects o JOIN main.gogoke_receipts rec ON rec.domain_id=o.domain_id AND rec.object_type=o.object_type AND rec.object_id=o.object_id AND rec.object_version=o.object_version WHERE o.domain_id=? AND o.object_type=? AND o.object_id=? AND o.object_version=?",&[domain,kind,id,ver],8)?;if r.len()!=1||content_hash(r[0][1].as_bytes())!=r[0][0]||content_hash(r[0][3].as_bytes())!=r[0][2]||r[0][4]!=kind||r[0][5]!=id||r[0][6]!=ver||r[0][7]!=receipt_type{return denied();}Ok(DreamReceipt{domain_id:domain.into(),object_id:id.into(),revision:ver.into(),content_hash:r[0][0].clone(),canonical_record:r[0][1].as_bytes().to_vec()})})}
