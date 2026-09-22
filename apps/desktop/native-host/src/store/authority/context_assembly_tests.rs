//! Native pre-search assembly authority regressions. Definitions are not execution evidence.
use super::bootstrap::{initialize_profile, OwnerIssuer};
use super::catalog::{issue_owner_grant, revoke_owner_grant};
use super::context_assembly::{list_context_assembly_sources, read_context_assembly_basis};
use super::context_manifest::{
    initialize_context_manifest_schema, publish_context_assembly_snapshot, ContextAssemblySnapshot,
    ContextManifestReplayIdentity, ContextPartitionGrantBinding,
};
use super::model::{GrantRef, GrantSpec};
use super::task_context::{commit_task_context_requirements, CommitTaskContextRequirements, MandatoryContextRef};
use crate::root::RootLock;
use crate::store::action::{apply_action_schema, record_action_outcome, reserve_action, ActionReservation};
use crate::store::context::{apply_context_schema, commit_context_version, ContextCommand};
use crate::store::atomic::apply_core_schema;
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::time::{SystemTime, UNIX_EPOCH};

const ACTION:&str="opr_22222222222222222222222222222222";
const OPERATION:&str="assembly-operation";

fn digest(ch:char)->String{format!("sha256:{}",ch.to_string().repeat(64))}
fn fixture(run:impl FnOnce(&mut VerifiedDatabaseConnection<'_>,&OwnerIssuer)){
 let _guard=route_b_test_guard();let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
 let path=std::env::temp_dir().join(format!("gogoke-context-assembly-{}-{nonce}",std::process::id()));
 std::fs::create_dir(&path).unwrap();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
 let mut db=create_new(&root,&database).unwrap();apply_core_schema(&mut db).unwrap();apply_action_schema(&mut db).unwrap();apply_context_schema(&mut db).unwrap();
 let owner=initialize_profile(&mut db,&root).unwrap();initialize_context_manifest_schema(&mut db).unwrap();
 commit_task_context_requirements(&mut db,&CommitTaskContextRequirements{operation_id:"task-create".into(),domain_id:"domain-one".into(),task_id:"task-one".into(),
  expected_previous_revision:None,mandatory_refs:vec![],event_id:"task-event".into(),receipt_id:"task-receipt".into(),recorded_at:"2026-09-21T00:00:00Z".into()}).unwrap();
 run(&mut db,&owner);
 db.close_checked().unwrap();drop(root);std::fs::remove_file(database).unwrap();if let Err(error)=std::fs::remove_dir(path){eprintln!("owned fixture retained: {error}");}
}
fn spec(principal:&str,seat:&str,source:&str)->GrantSpec{GrantSpec{principal_id:principal.into(),seat_id:seat.into(),
 permission:"context.read".into(),promotion_kind:"PROJECT_ONLY".into(),source_domain_id:source.into(),
 destination_domain_id:"domain-one".into(),destination_scope:"PROJECT".into(),delegable_depth:0}}
fn issue(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,principal:&str,seat:&str,source:&str)->GrantRef{
 issue_owner_grant(db,owner,"1","0",spec(principal,seat,source)).unwrap()
}
fn action(db:&mut VerifiedDatabaseConnection<'_>){reserve_action(db,ActionReservation{operation_id:ACTION.into(),semantic_digest:digest('c'),
 reservation_id:"assembly-action".into(),binding_id:"binding-one".into(),session_id:"session-one".into(),execution_id:"execution-one".into(),runtime_instance_id:"runtime-one".into(),
 profile_id:"profile-one".into(),auth_revision:"2".into(),generation:"7".into(),lane:"work".into(),action_kind:"queue".into(),payload_hex:"7b7d".into(),commitment:crate::store::action::test_commitment("session-one","execution-one","7")}).unwrap();}
fn binding(grant:GrantRef,source:&str)->ContextPartitionGrantBinding{ContextPartitionGrantBinding{source_domain_id:source.into(),
 destination_scope:"PROJECT".into(),promotion_kind:"PROJECT_ONLY".into(),grant}}
fn snapshot(principal:&str,seat:&str,bindings:Vec<ContextPartitionGrantBinding>,max:u64)->ContextAssemblySnapshot{ContextAssemblySnapshot{
 operation_id:OPERATION.into(),principal_id:principal.into(),seat_id:seat.into(),task_id:"task-one".into(),session_id:"session-one".into(),
 domain_id:"domain-one".into(),binding_id:"binding-one".into(),binding_generation:"7".into(),source_epoch:"9".into(),runtime_instance_id:"runtime-one".into(),
 task_revision:"1".into(),policy_revision:"1".into(),auth_revision:"2".into(),revocation_head:"0".into(),selection_decision_id:"decision-one".into(),
 manifest_id:"manifest-one".into(),admission_action_operation_id:ACTION.into(),admission_digest:digest('c'),max_content_bytes:4096,max_candidates:max,
 partition_grant_bindings:bindings}}
fn identity()->ContextManifestReplayIdentity{ContextManifestReplayIdentity{operation_id:OPERATION.into(),principal_id:"principal-one".into(),
 seat_id:"seat-one".into(),task_id:"task-one".into(),session_id:"session-one".into(),domain_id:"domain-one".into(),binding_id:"binding-one".into(),
 binding_generation:"7".into(),source_epoch:"9".into(),runtime_instance_id:"runtime-one".into()}}
fn source(db:&mut VerifiedDatabaseConnection<'_>,operation:&str,domain:&str,id:&str,visibility:&str,refs:Vec<String>){
 commit_context_version(db,ContextCommand{operation_id:operation.into(),context_id:id.into(),version:"1".into(),scope:"PROJECT".into(),domain_id:domain.into(),
  kind:"fact".into(),content_hash:digest('a'),source_ref:format!("source://{id}"),source_hash:digest('b'),source_authority_kind:"repository".into(),
  source_authority_ref:"authority://one".into(),derived_from:vec![],supersedes:vec![],access_policy_revision:"1".into(),visibility:visibility.into(),
  read_grant_refs:refs,promotion:None}).unwrap();
}
fn revise_task(db:&mut VerifiedDatabaseConnection<'_>,operation:&str,previous:&str,refs:Vec<MandatoryContextRef>){
 commit_task_context_requirements(db,&CommitTaskContextRequirements{operation_id:operation.into(),domain_id:"domain-one".into(),task_id:"task-one".into(),
  expected_previous_revision:Some(previous.into()),mandatory_refs:refs,event_id:format!("event-{operation}"),receipt_id:format!("receipt-{operation}"),
  recorded_at:"2026-09-21T00:00:00Z".into()}).unwrap();
}

#[test]
fn basis_and_list_return_only_stored_basis_and_stably_ordered_authorized_metadata(){
 fixture(|db,owner|{let grant=issue(db,owner,"principal-one","seat-one","domain-source");action(db);
  source(db,"source-z","domain-source","z-context","DOMAIN_GRANTED",vec![grant.grant_id.clone()]);
  source(db,"source-a","domain-source","a-context","DOMAIN_GRANTED",vec![grant.grant_id.clone()]);
  publish_context_assembly_snapshot(db,&snapshot("principal-one","seat-one",vec![binding(grant.clone(),"domain-source")],8)).unwrap();
  let basis=read_context_assembly_basis(db,&identity()).unwrap();assert_eq!(basis.max_content_bytes,4096);assert_eq!(basis.max_candidates,8);
  assert_eq!(basis.task_revision,"1");assert_eq!(basis.partition_grant_bindings,vec![binding(grant.clone(),"domain-source")]);
  let listed=list_context_assembly_sources(db,&identity()).unwrap();
  assert_eq!(listed.iter().map(|x|x.context_id.as_str()).collect::<Vec<_>>(),vec!["a-context","z-context"]);
  assert!(listed.iter().all(|x|x.source_domain_id=="domain-source"&&x.state_revision=="1"));
  assert!(listed.iter().all(|x|x.grant_id==grant.grant_id&&x.grant_revision==grant.revision));
 });
}

#[test]
fn current_grant_and_complete_replay_identity_are_rechecked_before_each_read(){
 fixture(|db,owner|{let grant=issue(db,owner,"principal-one","seat-one","domain-source");action(db);
  publish_context_assembly_snapshot(db,&snapshot("principal-one","seat-one",vec![binding(grant.clone(),"domain-source")],8)).unwrap();
  assert!(read_context_assembly_basis(db,&identity()).is_ok());revoke_owner_grant(db,owner,"1",&grant).unwrap();
  assert!(read_context_assembly_basis(db,&identity()).is_err());assert!(list_context_assembly_sources(db,&identity()).is_err());
 });
 fixture(|db,owner|{let grant=issue(db,owner,"principal-one","seat-one","domain-source");action(db);
  publish_context_assembly_snapshot(db,&snapshot("principal-one","seat-one",vec![binding(grant,"domain-source")],8)).unwrap();
  record_action_outcome(db,"assembly-action",ACTION,&digest('c'),"dispatched","","receipt-one").unwrap();
  assert!(read_context_assembly_basis(db,&identity()).is_err());assert!(list_context_assembly_sources(db,&identity()).is_err());
 });
 fixture(|db,owner|{let grant=issue(db,owner,"principal-one","seat-one","domain-source");action(db);
  publish_context_assembly_snapshot(db,&snapshot("principal-one","seat-one",vec![binding(grant,"domain-source")],8)).unwrap();
  let original=identity();let changes:[fn(&mut ContextManifestReplayIdentity);10]=[
   |x|x.operation_id="other".into(),|x|x.principal_id="other".into(),|x|x.seat_id="other".into(),|x|x.task_id="other".into(),|x|x.session_id="other".into(),
   |x|x.domain_id="other".into(),|x|x.binding_id="other".into(),|x|x.binding_generation="8".into(),|x|x.source_epoch="10".into(),
   |x|x.runtime_instance_id="other".into()];
  for change in changes{let mut swapped=original.clone();change(&mut swapped);assert!(read_context_assembly_basis(db,&swapped).is_err());assert!(list_context_assembly_sources(db,&swapped).is_err());}
 });
}

#[test]
fn other_grant_prefix_private_and_other_domain_rows_never_enter_the_relation(){
 fixture(|db,owner|{let bound=issue(db,owner,"principal-one","seat-one","domain-source");let other=issue(db,owner,"principal-one","seat-one","domain-source");action(db);
  source(db,"accessible","domain-source","same-id","DOMAIN_GRANTED",vec![bound.grant_id.clone()]);
  source(db,"other-grant","domain-source","other-grant","DOMAIN_GRANTED",vec![other.grant_id.clone()]);
  source(db,"prefix","domain-source","prefix","DOMAIN_GRANTED",vec![format!("{}-suffix",bound.grant_id)]);
  source(db,"private","domain-source","private","OWNER_PRIVATE",vec![]);
  source(db,"decoy","domain-other","same-id","DOMAIN_GRANTED",vec![bound.grant_id.clone()]);
  for index in 0..70{source(db,&format!("invisible-operation-{index}"),"domain-source",&format!("invisible-{index:03}"),"DOMAIN_GRANTED",vec![other.grant_id.clone()]);}
  publish_context_assembly_snapshot(db,&snapshot("principal-one","seat-one",vec![binding(bound,"domain-source")],8)).unwrap();
  let listed=list_context_assembly_sources(db,&identity()).unwrap();assert_eq!(listed.len(),1);assert_eq!(listed[0].context_id,"same-id");
 });
}

#[test]
fn owner_private_requires_the_current_profile_owner_and_same_domain_multi_grant_dedupes(){
 fixture(|db,owner|{let first=issue(db,owner,owner.principal_id(),owner.seat_id(),"domain-source");let second=issue(db,owner,owner.principal_id(),owner.seat_id(),"domain-source");action(db);
  source(db,"private","domain-source","private","OWNER_PRIVATE",vec![]);
  source(db,"shared","domain-source","shared","DOMAIN_GRANTED",vec![first.grant_id.clone(),second.grant_id.clone()]);
  publish_context_assembly_snapshot(db,&snapshot(owner.principal_id(),owner.seat_id(),vec![binding(first,"domain-source"),binding(second,"domain-source")],8)).unwrap();
  let mut owner_identity=identity();owner_identity.principal_id=owner.principal_id().into();owner_identity.seat_id=owner.seat_id().into();
  let listed=list_context_assembly_sources(db,&owner_identity).unwrap();assert_eq!(listed.iter().map(|x|x.context_id.as_str()).collect::<Vec<_>>(),vec!["private","shared"]);
 });
}

#[test]
fn inactive_malformed_duplicate_acl_and_missing_partition_rows_fail_closed(){
 fixture(|db,owner|{let grant=issue(db,owner,"principal-one","seat-one","domain-source");action(db);
  source(db,"inactive","domain-source","inactive","DOMAIN_GRANTED",vec![grant.grant_id.clone()]);
  db.execute("UPDATE gogoke_context_states SET state='STALE' WHERE domain_id='domain-source'").unwrap();
  publish_context_assembly_snapshot(db,&snapshot("principal-one","seat-one",vec![binding(grant,"domain-source")],8)).unwrap();
  assert!(list_context_assembly_sources(db,&identity()).unwrap().is_empty());
 });
 for acl in ["bad token","DUPLICATE"]{
  fixture(|db,owner|{let grant=issue(db,owner,"principal-one","seat-one","domain-source");action(db);
   let refs=if acl=="DUPLICATE"{vec![grant.grant_id.clone(),grant.grant_id.clone()]}else{vec!["bad token".into()]};
   if acl=="DUPLICATE"{source(db,"bad-acl","domain-source","bad-acl","DOMAIN_GRANTED",refs);}else{
    source(db,"bad-acl","domain-source","bad-acl","DOMAIN_GRANTED",vec![grant.grant_id.clone()]);
    db.execute("UPDATE gogoke_context_access SET read_grant_refs='bad token' WHERE domain_id='domain-source'").unwrap();
   }
   publish_context_assembly_snapshot(db,&snapshot("principal-one","seat-one",vec![binding(grant,"domain-source")],8)).unwrap();
   assert!(list_context_assembly_sources(db,&identity()).is_err());
  });
 }
 fixture(|db,owner|{let grant=issue(db,owner,"principal-one","seat-one","domain-source");action(db);
  publish_context_assembly_snapshot(db,&snapshot("principal-one","seat-one",vec![binding(grant,"domain-source")],8)).unwrap();
  db.execute("DELETE FROM gogoke_context_assembly_partition_grant_bindings WHERE operation_id='assembly-operation'").unwrap();
  assert!(read_context_assembly_basis(db,&identity()).is_err());assert!(list_context_assembly_sources(db,&identity()).is_err());
 });
}

#[test]
fn authorized_relation_is_capped_only_after_filtering_and_stable_deduplication(){
 fixture(|db,owner|{let grant=issue(db,owner,"principal-one","seat-one","domain-source");let other=issue(db,owner,"principal-one","seat-one","domain-source");action(db);
  for index in 0..70{source(db,&format!("hidden-operation-{index}"),"domain-source",&format!("a-hidden-{index:03}"),"DOMAIN_GRANTED",vec![other.grant_id.clone()]);}
  for id in ["b-authorized","c-authorized","d-authorized"]{source(db,&format!("operation-{id}"),"domain-source",id,"DOMAIN_GRANTED",vec![grant.grant_id.clone()]);}
  publish_context_assembly_snapshot(db,&snapshot("principal-one","seat-one",vec![binding(grant,"domain-source")],2)).unwrap();
  let listed=list_context_assembly_sources(db,&identity()).unwrap();assert_eq!(listed.iter().map(|x|x.context_id.as_str()).collect::<Vec<_>>(),vec!["b-authorized","c-authorized"]);
 });
}

#[test]
fn mandatory_source_must_be_currently_authorized_and_task_revision_change_invalidates_snapshot_reads(){
 fixture(|db,owner|{let grant=issue(db,owner,"principal-one","seat-one","domain-source");action(db);
  let mandatory=MandatoryContextRef{source_domain_id:"domain-source".into(),context_id:"required".into(),version:"1".into()};
  revise_task(db,"task-mandatory","1",vec![mandatory.clone()]);
  let mut current=snapshot("principal-one","seat-one",vec![binding(grant.clone(),"domain-source")],8);current.task_revision="2".into();
  assert!(publish_context_assembly_snapshot(db,&snapshot("principal-one","seat-one",vec![binding(grant.clone(),"domain-source")],8)).is_err());
  publish_context_assembly_snapshot(db,&current).unwrap();
  assert_eq!(read_context_assembly_basis(db,&identity()).unwrap().mandatory_refs,vec![mandatory.clone()]);
  assert!(list_context_assembly_sources(db,&identity()).is_err());
  source(db,"required-source","domain-source","required","DOMAIN_GRANTED",vec![grant.grant_id]);
  assert_eq!(list_context_assembly_sources(db,&identity()).unwrap()[0].context_id,"required");
  revise_task(db,"task-race","2",vec![mandatory]);
  assert!(read_context_assembly_basis(db,&identity()).is_err());
  assert!(list_context_assembly_sources(db,&identity()).is_err());
 });
}
