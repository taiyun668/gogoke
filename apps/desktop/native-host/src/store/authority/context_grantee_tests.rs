//! Native grant-grantee Context reads; definitions only until controlled native execution.
use super::bootstrap::{initialize_profile, OwnerIssuer};
use super::catalog::{delegate_owner_grant, issue_owner_grant, revoke_owner_grant};
use super::context_read::{ContextReadRequest, GranteeContextReadRequest};
use super::context_read_set::read_grantee_context_set;
use super::model::{GrantRef, GrantSpec};
use crate::root::RootLock;
use crate::store::context::{apply_context_schema, commit_context_version, ContextCommand};
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(run: impl FnOnce(&RootLock, &mut VerifiedDatabaseConnection<'_>, &OwnerIssuer)) {
    let _guard=route_b_test_guard();let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path=std::env::temp_dir().join(format!("gogoke-context-grantee-{}-{nonce}",std::process::id()));
    std::fs::create_dir(&path).unwrap();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
    let mut db=create_new(&root,&database).unwrap();apply_context_schema(&mut db).unwrap();
    let owner=initialize_profile(&mut db,&root).unwrap();run(&root,&mut db,&owner);
    db.close_checked().unwrap();drop(root);std::fs::remove_file(database).unwrap();
    if let Err(e)=std::fs::remove_dir(path){eprintln!("owned fixture retained:{e}");}
}
fn spec(principal:&str,seat:&str,depth:u8)->GrantSpec{GrantSpec{
    principal_id:principal.into(),seat_id:seat.into(),permission:"context.read".into(),promotion_kind:"PROJECT_ONLY".into(),
    source_domain_id:"domain-source".into(),destination_domain_id:"domain-target".into(),destination_scope:"PROJECT".into(),
    delegable_depth:depth}}
fn delegated(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer)->GrantRef{
    let parent=issue_owner_grant(db,owner,"1","0",spec(owner.principal_id(),owner.seat_id(),2)).unwrap();
    delegate_owner_grant(db,owner,"1",&parent,spec("principal-one","seat-one",1)).unwrap()
}
fn source(db:&mut VerifiedDatabaseConnection<'_>, grant:&GrantRef, visibility:&str, version:&str){
    commit_context_version(db,ContextCommand{operation_id:format!("source-op-{version}"),context_id:"source-context".into(),
      version:version.into(),scope:"PROJECT".into(),domain_id:"domain-source".into(),kind:"fact".into(),
      content_hash:format!("sha256:{}","a".repeat(64)),source_ref:"source://one".into(),source_hash:format!("sha256:{}","b".repeat(64)),
      source_authority_kind:"repository".into(),source_authority_ref:"authority://one".into(),derived_from:vec![],supersedes:vec![],
      access_policy_revision:"1".into(),visibility:visibility.into(),
      read_grant_refs:if visibility=="DOMAIN_GRANTED"{vec![grant.grant_id.clone()]}else{vec![]},promotion:None}).unwrap();
}
fn read(grant:GrantRef,version:&str)->GranteeContextReadRequest{GranteeContextReadRequest{
    principal_id:"principal-one".into(),seat_id:"seat-one".into(),source:ContextReadRequest{
      source_domain_id:"domain-source".into(),context_id:"source-context".into(),version:version.into(),expected_scope:"PROJECT".into(),
      expected_content_hash:format!("sha256:{}","a".repeat(64)),expected_access_policy_revision:"1".into(),
      destination_domain_id:"domain-target".into(),destination_scope:"PROJECT".into(),promotion_kind:"PROJECT_ONLY".into(),
      policy_revision:"1".into(),grant}}}

#[test]
fn delegated_domain_grant_returns_current_lifecycle_revision_without_owner_capability(){
    fixture(|_,db,owner|{let grant=delegated(db,owner);source(db,&grant,"DOMAIN_GRANTED","1");
      let set=read_grantee_context_set(db,&[read(grant,"1")]).unwrap();
      assert_eq!(set.principal_id,"principal-one");assert_eq!(set.seat_id,"seat-one");
      assert_eq!(set.sources.len(),1);assert_eq!(set.sources[0].state,"ACTIVE");
      assert_eq!(set.sources[0].state_revision,"1");assert_eq!(set.sources[0].source.context_id,"source-context");});
}
#[test]
fn owner_private_context_is_not_readable_by_non_owner_even_with_matching_read_grant(){
    fixture(|_,db,owner|{let grant=delegated(db,owner);source(db,&grant,"OWNER_PRIVATE","1");
      assert!(read_grantee_context_set(db,&[read(grant,"1")]).is_err());});
}
#[test]
fn domain_granted_acl_must_name_the_exact_current_grant(){
    fixture(|_,db,owner|{let grant=delegated(db,owner);let other=delegated(db,owner);
      source(db,&other,"DOMAIN_GRANTED","1");assert!(read_grantee_context_set(db,&[read(grant,"1")]).is_err());});
}
#[test]
fn revocation_is_rechecked_on_every_grantee_read(){
    fixture(|_,db,owner|{let grant=delegated(db,owner);source(db,&grant,"DOMAIN_GRANTED","1");
      assert!(read_grantee_context_set(db,&[read(grant.clone(),"1")]).is_ok());
      let _=revoke_owner_grant(db,owner,"1",&grant).unwrap();
      assert!(read_grantee_context_set(db,&[read(grant,"1")]).is_err());});
}
#[test]
fn superseded_context_cannot_be_replayed_as_visible_active_material(){
    fixture(|_,db,owner|{let grant=delegated(db,owner);source(db,&grant,"DOMAIN_GRANTED","1");
      let replacement=ContextCommand{operation_id:"replace-op".into(),context_id:"source-context".into(),version:"2".into(),
        scope:"PROJECT".into(),domain_id:"domain-source".into(),kind:"fact".into(),content_hash:format!("sha256:{}","c".repeat(64)),
        source_ref:"source://two".into(),source_hash:format!("sha256:{}","d".repeat(64)),source_authority_kind:"repository".into(),
        source_authority_ref:"authority://one".into(),derived_from:vec![],supersedes:vec!["source-context@1".into()],
        access_policy_revision:"1".into(),visibility:"DOMAIN_GRANTED".into(),read_grant_refs:vec![grant.grant_id.clone()],promotion:None};
      commit_context_version(db,replacement.clone()).unwrap();
      assert!(read_grantee_context_set(db,&[read(grant,"1")]).is_err());});
}
#[test]
fn one_read_set_cannot_mix_grantee_identity_or_two_versions_of_one_context(){
    fixture(|_,db,owner|{let grant=delegated(db,owner);source(db,&grant,"DOMAIN_GRANTED","1");
      let mut other=read(grant.clone(),"1");other.seat_id="seat-other".into();
      assert!(read_grantee_context_set(db,&[read(grant.clone(),"1"),other]).is_err());
      // A second immutable version exists but one assembly set may not select both.
      source(db,&grant,"DOMAIN_GRANTED","2");
      assert!(read_grantee_context_set(db,&[read(grant.clone(),"1"),read(grant,"2")]).is_err());});
}
#[test]
fn stale_grant_head_or_wrong_subject_never_falls_back_to_service_actor_identity(){
    fixture(|_,db,owner|{let grant=delegated(db,owner);source(db,&grant,"DOMAIN_GRANTED","1");
      let mut wrong=read(grant.clone(),"1");wrong.principal_id="principal-other".into();
      assert!(read_grantee_context_set(db,&[wrong]).is_err());
      let other=issue_owner_grant(db,owner,"1","0",spec(owner.principal_id(),owner.seat_id(),0)).unwrap();
      let _new_head=revoke_owner_grant(db,owner,"1",&other).unwrap();
      // The delegated read still carries the old revocation head and must fail.
      assert!(read_grantee_context_set(db,&[read(grant,"1")]).is_err());});
}
