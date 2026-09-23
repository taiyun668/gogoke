use super::native_binding::{AccountRefSnapshot, AppendRuntimeInstanceIdentity, CommitNativeBinding, NativeBindingIdentity, RuntimeInstanceIdentitySnapshot};
use super::session_lineage::{NativeSessionIdentity, SessionLineageCommand, SessionLineageOperation};
use crate::root::RootLock;
use crate::store::product_database::ProductDatabase;
use crate::store::same_open::{open_existing,route_b_test_guard};
use std::path::{Path,PathBuf};
use std::time::{SystemTime,UNIX_EPOCH};

fn scratch() -> PathBuf {
    let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path=std::env::temp_dir().join(format!("gogoke-native-binding-{}-{nonce}",std::process::id()));
    std::fs::create_dir(&path).unwrap();path
}
fn cleanup(root:&Path) {
    for name in ["state.sqlite","state.sqlite-wal","state.sqlite-shm"] {let _=std::fs::remove_file(root.join(name));}
    if let Err(error)=std::fs::remove_dir(root){eprintln!("owned native binding test root retained: {error}");}
}
fn identity(domain:&str,instance:&str,account:&str)->RuntimeInstanceIdentitySnapshot {
    RuntimeInstanceIdentitySnapshot{domain_id:domain.into(),instance_id:instance.into(),version:"1".into(),driver_id:"driver-one".into(),profile_ref:"profile-one".into(),profile_revision:"1".into(),account_ref:AccountRefSnapshot::Present(account.into()),auth_revision:"1".into()}
}
fn append(product:&mut ProductDatabase<'_>,value:RuntimeInstanceIdentitySnapshot){
    let tag=format!("{}-{}",value.domain_id,value.instance_id);
    product.append_runtime_instance_identity(&AppendRuntimeInstanceIdentity{operation_id:format!("identity-op-{tag}"),snapshot:value,event_id:format!("identity-event-{tag}"),receipt_id:format!("identity-receipt-{tag}"),recorded_at:"2026-09-23T12:00:00Z".into(),expected_previous_version:None}).unwrap();
}
fn lineage(product:&mut ProductDatabase<'_>,domain:&str,session:&str,binding:&str,native:&str){
    product.apply_session_lineage(&SessionLineageCommand{operation_id:format!("lineage-op-{domain}-{session}"),domain_id:domain.into(),event_id:format!("lineage-event-{domain}-{session}"),receipt_id:format!("lineage-receipt-{domain}-{session}"),recorded_at:"2026-09-23T12:00:00Z".into(),operation:SessionLineageOperation::NewClean{session_id:session.into(),native:NativeSessionIdentity{native_session_id:native.into(),binding_id:binding.into(),generation:"1".into(),source_epoch:"1".into(),domain_id:domain.into()}}}).unwrap();
}
fn bind(product:&mut ProductDatabase<'_>,domain:&str,instance:&str,binding:&str,session:&str,native:&str)->Result<(),String>{
    bind_version(product,domain,instance,"1",binding,session,native)
}
fn binding_input(domain:&str,instance:&str,version:&str,binding:&str,session:&str,native:&str)->CommitNativeBinding{
    CommitNativeBinding{operation_id:format!("binding-op-{domain}-{binding}"),domain_id:domain.into(),binding_id:binding.into(),generation:"1".into(),expected_previous_generation:None,source_epoch:"1".into(),instance_id:instance.into(),instance_version:version.into(),native_identity:native.into(),lineage_ref:session.into(),custody_ref:"custody-one".into(),event_id:format!("binding-event-{domain}-{binding}"),receipt_id:format!("binding-receipt-{domain}-{binding}"),recorded_at:"2026-09-23T12:00:00Z".into()}
}
fn bind_version(product:&mut ProductDatabase<'_>,domain:&str,instance:&str,version:&str,binding:&str,session:&str,native:&str)->Result<(),String>{
    product.commit_native_binding(&binding_input(domain,instance,version,binding,session,native)).map(|_|()).map_err(|error|format!("{error:?}"))
}

#[test]
fn account_revision_and_domain_namespace_are_distinct_and_conflicts_leave_no_binding(){
    let _guard=route_b_test_guard();let path=scratch();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
    let mut product=ProductDatabase::open(&root,&database).unwrap();
    append(&mut product,identity("domain-one","instance-a","account-a"));
    lineage(&mut product,"domain-one","session-old","binding-old","native-shared");
    bind(&mut product,"domain-one","instance-a","binding-old","session-old","native-shared").unwrap();

    let mut updated=identity("domain-one","instance-a","account-b");updated.version="2".into();
    product.append_runtime_instance_identity(&AppendRuntimeInstanceIdentity{operation_id:"identity-op-new-account".into(),snapshot:updated,event_id:"identity-event-new-account".into(),receipt_id:"identity-receipt-new-account".into(),recorded_at:"2026-09-23T12:00:00Z".into(),expected_previous_version:Some("1".into())}).unwrap();
    assert_eq!(product.read_runtime_instance_identity("domain-one","instance-a","1").unwrap().unwrap().account_ref,AccountRefSnapshot::Present("account-a".into()));
    let old=NativeBindingIdentity{domain_id:"domain-one".into(),binding_id:"binding-old".into(),generation:"1".into(),source_epoch:"1".into(),instance_id:"instance-a".into(),instance_version:"1".into()};
    assert!(product.read_native_binding(&old).is_err(),"old account binding must not stay current");

    lineage(&mut product,"domain-one","session-new","binding-new","native-shared");
    bind_version(&mut product,"domain-one","instance-a","2","binding-new","session-new","native-shared").unwrap();
    let current=NativeBindingIdentity{binding_id:"binding-new".into(),instance_version:"2".into(),..old.clone()};
    assert_eq!(product.read_native_binding(&current).unwrap().unwrap().instance.account_ref,AccountRefSnapshot::Present("account-b".into()));

    append(&mut product,identity("domain-two","instance-a","account-a"));
    lineage(&mut product,"domain-two","session-domain","binding-domain","native-shared");
    bind(&mut product,"domain-two","instance-a","binding-domain","session-domain","native-shared").unwrap();
    let different_domain=NativeBindingIdentity{domain_id:"domain-two".into(),binding_id:"binding-domain".into(),instance_version:"1".into(),..current.clone()};
    assert_eq!(product.read_native_binding(&different_domain).unwrap().unwrap().native_identity,"native-shared");
    assert!(product.read_native_binding(&NativeBindingIdentity{source_epoch:"2".into(),..current.clone()}).is_err());

    lineage(&mut product,"domain-one","session-duplicate","binding-duplicate","native-shared");
    assert!(bind_version(&mut product,"domain-one","instance-a","2","binding-duplicate","session-duplicate","native-shared").is_err());
    let duplicate=NativeBindingIdentity{binding_id:"binding-duplicate".into(),..current.clone()};
    assert!(product.read_native_binding(&duplicate).unwrap().is_none(),"failed identity collision must not leave projection");
    let mut changed=binding_input("domain-one","instance-a","2","binding-new","session-new","native-shared");
    changed.custody_ref="custody-different".into();
    assert!(product.commit_native_binding(&changed).is_err(),"same operation with different bytes must conflict");

    product.close_checked().unwrap();drop(root);cleanup(&path);
}

#[test]
fn binding_generation_can_replace_an_obsolete_account_and_lineage(){
    let _guard=route_b_test_guard();let path=scratch();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
    let mut product=ProductDatabase::open(&root,&database).unwrap();
    append(&mut product,identity("domain-one","instance-a","account-a"));
    lineage(&mut product,"domain-one","session-old","binding-stable","native-old");
    bind(&mut product,"domain-one","instance-a","binding-stable","session-old","native-old").unwrap();
    let mut next=identity("domain-one","instance-a","account-b");next.version="2".into();
    product.append_runtime_instance_identity(&AppendRuntimeInstanceIdentity{operation_id:"identity-next-op".into(),snapshot:next,event_id:"identity-next-event".into(),receipt_id:"identity-next-receipt".into(),recorded_at:"2026-09-23T12:00:00Z".into(),expected_previous_version:Some("1".into())}).unwrap();
    product.apply_session_lineage(&SessionLineageCommand{operation_id:"lineage-next-op".into(),domain_id:"domain-one".into(),event_id:"lineage-next-event".into(),receipt_id:"lineage-next-receipt".into(),recorded_at:"2026-09-23T12:00:00Z".into(),operation:SessionLineageOperation::NewClean{session_id:"session-next".into(),native:NativeSessionIdentity{native_session_id:"native-next".into(),binding_id:"binding-stable".into(),generation:"2".into(),source_epoch:"2".into(),domain_id:"domain-one".into()}}}).unwrap();
    let mut replacement=binding_input("domain-one","instance-a","2","binding-stable","session-next","native-next");
    replacement.operation_id="binding-next-op".into();replacement.event_id="binding-next-event".into();replacement.receipt_id="binding-next-receipt".into();replacement.generation="2".into();replacement.source_epoch="2".into();replacement.expected_previous_generation=Some("1".into());
    product.commit_native_binding(&replacement).unwrap();
    let current=NativeBindingIdentity{domain_id:"domain-one".into(),binding_id:"binding-stable".into(),generation:"2".into(),source_epoch:"2".into(),instance_id:"instance-a".into(),instance_version:"2".into()};
    assert_eq!(product.read_native_binding(&current).unwrap().unwrap().instance.account_ref,AccountRefSnapshot::Present("account-b".into()));
    assert!(product.read_native_binding(&NativeBindingIdentity{generation:"1".into(),source_epoch:"1".into(),instance_version:"1".into(),..current}).is_err());
    product.close_checked().unwrap();
    let mut raw=open_existing(&root,&database).unwrap();
    raw.execute("UPDATE main.gogoke_events SET canonical_json=x'7b7d' WHERE domain_id='domain-one' AND object_type='NativeBinding' AND object_id='binding-stable' AND object_version='1'").unwrap();
    raw.close_checked().unwrap();
    let mut product=ProductDatabase::open(&root,&database).unwrap();
    assert!(product.read_native_binding(&NativeBindingIdentity{domain_id:"domain-one".into(),binding_id:"binding-stable".into(),generation:"2".into(),source_epoch:"2".into(),instance_id:"instance-a".into(),instance_version:"2".into()}).is_err(),"corrupt historical event must poison current binding");
    product.close_checked().unwrap();drop(root);cleanup(&path);
}

fn seed_single_binding(root:&RootLock,path:&Path){
    let mut product=ProductDatabase::open(root,path).unwrap();
    append(&mut product,identity("domain-one","instance-a","account-a"));
    lineage(&mut product,"domain-one","session-one","binding-one","native-one");
    bind(&mut product,"domain-one","instance-a","binding-one","session-one","native-one").unwrap();
    product.close_checked().unwrap();
}
fn original_binding()->NativeBindingIdentity{
    NativeBindingIdentity{domain_id:"domain-one".into(),binding_id:"binding-one".into(),generation:"1".into(),source_epoch:"1".into(),instance_id:"instance-a".into(),instance_version:"1".into()}
}

#[test]
fn projection_cannot_reassign_old_binding_to_new_account_version(){
    let _guard=route_b_test_guard();let path=scratch();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
    seed_single_binding(&root,&database);
    let mut product=ProductDatabase::open(&root,&database).unwrap();
    let mut next=identity("domain-one","instance-a","account-b");next.version="2".into();
    product.append_runtime_instance_identity(&AppendRuntimeInstanceIdentity{operation_id:"identity-change-op".into(),snapshot:next,event_id:"identity-change-event".into(),receipt_id:"identity-change-receipt".into(),recorded_at:"2026-09-23T12:00:00Z".into(),expected_previous_version:Some("1".into())}).unwrap();
    product.close_checked().unwrap();
    let mut raw=open_existing(&root,&database).unwrap();
    raw.execute("UPDATE main.gogoke_native_binding_versions SET instance_version='2' WHERE domain_id='domain-one' AND binding_id='binding-one'").unwrap();
    raw.close_checked().unwrap();
    let mut product=ProductDatabase::open(&root,&database).unwrap();
    assert!(product.read_native_binding(&NativeBindingIdentity{instance_version:"2".into(),..original_binding()}).is_err());
    product.close_checked().unwrap();drop(root);cleanup(&path);
}

#[test]
fn old_head_rollback_and_missing_head_schema_fail_closed(){
    let _guard=route_b_test_guard();
    for drop_table in [false,true] {
        let path=scratch();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
        seed_single_binding(&root,&database);
        if !drop_table {
            let mut product=ProductDatabase::open(&root,&database).unwrap();
            let mut next=identity("domain-one","instance-a","account-b");next.version="2".into();
            product.append_runtime_instance_identity(&AppendRuntimeInstanceIdentity{operation_id:"identity-head-op".into(),snapshot:next,event_id:"identity-head-event".into(),receipt_id:"identity-head-receipt".into(),recorded_at:"2026-09-23T12:00:00Z".into(),expected_previous_version:Some("1".into())}).unwrap();
            product.close_checked().unwrap();
        }
        let mut raw=open_existing(&root,&database).unwrap();
        if drop_table {raw.execute("DROP TABLE main.gogoke_native_binding_heads").unwrap();}
        else {raw.execute("UPDATE main.gogoke_runtime_instance_identity_heads SET identity_version='1',content_hash=(SELECT content_hash FROM main.gogoke_runtime_instance_identity_versions WHERE domain_id='domain-one' AND instance_id='instance-a' AND identity_version='1') WHERE domain_id='domain-one' AND instance_id='instance-a'").unwrap();}
        raw.close_checked().unwrap();
        if drop_table {assert!(ProductDatabase::open(&root,&database).is_err(),"partial authority schema was silently rebuilt");}
        else {let mut product=ProductDatabase::open(&root,&database).unwrap();assert!(product.read_native_binding(&original_binding()).is_err(),"old account head rollback resurrected a binding");product.close_checked().unwrap();}
        drop(root);cleanup(&path);
    }
}

#[test]
fn damaged_event_or_receipt_body_cannot_read_or_reconcile(){
    let _guard=route_b_test_guard();
    for object_type in ["RuntimeInstanceIdentity","NativeBinding"] {
        for table in ["gogoke_events","gogoke_receipts"] {
            let path=scratch();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
            seed_single_binding(&root,&database);
            let mut raw=open_existing(&root,&database).unwrap();
            let sql=format!("UPDATE main.{table} SET canonical_json=x'7b7d' WHERE domain_id='domain-one' AND object_type='{object_type}'");
            raw.execute(&sql).unwrap();raw.close_checked().unwrap();
            let mut product=ProductDatabase::open(&root,&database).unwrap();
            assert!(product.read_native_binding(&original_binding()).is_err(),"{table}/{object_type} body corruption was accepted");
            product.close_checked().unwrap();drop(root);cleanup(&path);
        }
    }
}

#[test]
fn stream_counter_rollback_cannot_leave_a_current_binding(){
    let _guard=route_b_test_guard();let path=scratch();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
    seed_single_binding(&root,&database);
    let mut raw=open_existing(&root,&database).unwrap();
    raw.execute("UPDATE main.gogoke_stream_heads SET counter='1' WHERE domain_id='domain-one' AND stream_id='gogoke.native-binding.v1/binding-one'").unwrap();
    raw.close_checked().unwrap();
    let mut product=ProductDatabase::open(&root,&database).unwrap();
    assert!(product.read_native_binding(&original_binding()).is_err(),"stream head was changed without an event");
    product.close_checked().unwrap();drop(root);cleanup(&path);
}

#[test]
fn typed_binding_and_identity_survive_reopen_without_cross_instance_collision(){
    let _guard=route_b_test_guard();let path=scratch();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
    let mut product=ProductDatabase::open(&root,&database).unwrap();
    append(&mut product,identity("domain-one","instance-a","account-a"));
    append(&mut product,identity("domain-one","instance-b","account-b"));
    lineage(&mut product,"domain-one","session-a","binding-a","native-shared");
    lineage(&mut product,"domain-one","session-b","binding-b","native-shared");
    bind(&mut product,"domain-one","instance-a","binding-a","session-a","native-shared").unwrap();
    bind(&mut product,"domain-one","instance-b","binding-b","session-b","native-shared").unwrap();
    product.close_checked().unwrap();
    let mut product=ProductDatabase::open(&root,&database).unwrap();
    for (instance,binding,account) in [("instance-a","binding-a","account-a"),("instance-b","binding-b","account-b")] {
        let found=product.read_native_binding(&NativeBindingIdentity{domain_id:"domain-one".into(),binding_id:binding.into(),generation:"1".into(),source_epoch:"1".into(),instance_id:instance.into(),instance_version:"1".into()}).unwrap().unwrap();
        assert_eq!(found.native_identity,"native-shared");
        assert_eq!(found.instance.account_ref,AccountRefSnapshot::Present(account.into()));
    }
    product.close_checked().unwrap();drop(root);cleanup(&path);
}
