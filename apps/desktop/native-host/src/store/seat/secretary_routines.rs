//! E.3 durable secretary schedule facts. Nothing here parses a time rule,
//! wakes a process, calls a model, or treats an intent as an H delivery receipt.
//! The host must authenticate the original User input and call this through
//! its existing coordinator and H transaction before exposing any operation.

use super::*;

const ROUTINES: &str = "CREATE TABLE gogoke_v37_seat_secretary_routines(routine_id TEXT PRIMARY KEY,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL,original_text TEXT NOT NULL CHECK(length(original_text)>0),source_operation_id TEXT NOT NULL,source_epoch TEXT NOT NULL,source_cursor TEXT NOT NULL,schedule_raw TEXT NOT NULL CHECK(length(schedule_raw)>0),timezone TEXT NOT NULL CHECK(length(timezone)>0),next_due_ms INTEGER NOT NULL CHECK(next_due_ms>=0),state TEXT NOT NULL CHECK(state IN ('ACTIVE','PAUSED','ABSENCE_PAUSED','WAITING_NEXT','DELETED')),revision INTEGER NOT NULL CHECK(revision>0),last_occurrence_id TEXT NOT NULL,last_result TEXT NOT NULL CHECK(last_result IN ('NONE','UNKNOWN','FAILED','DELIVERED')),last_reason TEXT NOT NULL,FOREIGN KEY(incarnation) REFERENCES gogoke_v37_seats(incarnation)) STRICT";
const OPERATIONS: &str = "CREATE TABLE gogoke_v37_seat_secretary_routine_operations(request_id TEXT PRIMARY KEY,fingerprint TEXT NOT NULL,routine_id TEXT NOT NULL,command TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),FOREIGN KEY(routine_id) REFERENCES gogoke_v37_seat_secretary_routines(routine_id)) STRICT";
const OCCURRENCES: &str = "CREATE TABLE gogoke_v37_seat_secretary_occurrences(occurrence_id TEXT PRIMARY KEY,routine_id TEXT NOT NULL,due_ms INTEGER NOT NULL CHECK(due_ms>0),state TEXT NOT NULL CHECK(state IN ('UNKNOWN','FAILED','DELIVERED')),h_receipt_id TEXT NOT NULL,original_reason TEXT NOT NULL,UNIQUE(routine_id,due_ms),FOREIGN KEY(routine_id) REFERENCES gogoke_v37_seat_secretary_routines(routine_id)) STRICT";
const PRESENCE: &str = "CREATE TABLE gogoke_v37_seat_secretary_presence(source_id TEXT PRIMARY KEY,kind TEXT NOT NULL CHECK(kind IN ('FOREGROUND','OPEN','INPUT')),source_operation_id TEXT NOT NULL,source_epoch TEXT NOT NULL,source_cursor TEXT NOT NULL,occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms>0),observed_at_ms INTEGER NOT NULL CHECK(observed_at_ms>0),CHECK(observed_at_ms>=occurred_at_ms)) STRICT";
const ABSENCE_POLICY: &str = "CREATE TABLE gogoke_v37_seat_secretary_absence_policy(singleton INTEGER PRIMARY KEY CHECK(singleton=1),revision INTEGER NOT NULL CHECK(revision>0),max_absent_ms INTEGER NOT NULL CHECK(max_absent_ms>0),source_id TEXT NOT NULL) STRICT";

pub(super) const SCHEMA: &[(&str,&str)] = &[
    ("gogoke_v37_seat_secretary_routines",ROUTINES),
    ("gogoke_v37_seat_secretary_routine_operations",OPERATIONS),
    ("gogoke_v37_seat_secretary_occurrences",OCCURRENCES),
    ("gogoke_v37_seat_secretary_presence",PRESENCE),
    ("gogoke_v37_seat_secretary_absence_policy",ABSENCE_POLICY),
];
pub(super) fn create_tables(db:&mut VerifiedDatabaseConnection<'_>)->Result<(),SeatError> {
    for (_,sql) in SCHEMA {db.execute(sql)?;}
    Ok(())
}

/// Product default recorded by the authenticated USER designation transaction.
/// An existing USER policy is never replaced or extended by designation replay.
/// This configures absence handling; it supplies no presence or launch evidence.
pub(super) fn ensure_product_absence_policy_in_transaction(
    db:&VerifiedDatabaseConnection<'_>,
)->Result<(),SeatError> {
    current_secretary(db)?;
    let existing=Statement::prepare(db.as_ptr(),
        "SELECT CAST(revision AS TEXT),CAST(max_absent_ms AS TEXT),source_id FROM main.gogoke_v37_seat_secretary_absence_policy WHERE singleton=1")?;
    if existing.step_row()? {
        let revision=existing.column_text(0)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
        let absent=existing.column_text(1)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
        let source=existing.column_text(2)?;
        if revision<=0 || absent<=0 || source.is_empty() || existing.step_row()? {
            return Err(SeatError::SchemaDrift);
        }
        return Ok(());
    }
    drop(existing);
    // One day is an explicit initial product policy, not an inferred Owner
    // presence, timing tolerance, upstream default or automatic resumption.
    Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_seat_secretary_absence_policy(singleton,revision,max_absent_ms,source_id) VALUES(1,1,86400000,'PRODUCT_DEFAULT_V1:ABSENCE_24_HOURS')")?.step_done()?;
    Ok(())
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct SecretaryRoutine {
    pub(crate) routine_id:String,
    pub(crate) seat_id:String,
    pub(crate) incarnation:String,
    pub(crate) original_text:String,
    pub(crate) source_operation_id:String,
    pub(crate) source_epoch:String,
    pub(crate) source_cursor:String,
    pub(crate) schedule_raw:String,
    pub(crate) timezone:String,
    /// Zero means that no next time has been supplied. It never means now.
    pub(crate) next_due_ms:i64,
    pub(crate) state:String,
    pub(crate) revision:i64,
    pub(crate) last_occurrence_id:String,
    pub(crate) last_result:String,
    pub(crate) last_reason:String,
}
pub(crate) struct SecretaryRoutineCreate<'a> {
    pub(crate) routine_id:&'a str,
    pub(crate) request_id:&'a str,
    pub(crate) request_bytes:&'a [u8],
    /// Exact User natural language, supplied from an authenticated original
    /// input source by Root. Model tool arguments alone are not this source.
    pub(crate) original_text:&'a str,
    pub(crate) source_operation_id:&'a str,
    pub(crate) source_epoch:&'a str,
    pub(crate) source_cursor:&'a str,
    pub(crate) schedule_raw:&'a str,
    pub(crate) timezone:&'a str,
    pub(crate) next_due_ms:i64,
    pub(crate) now_ms:i64,
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub(crate) enum SecretaryRoutineCommand {Pause,Resume,Delete}
impl SecretaryRoutineCommand {
    fn sql(self)->&'static str {match self {Self::Pause=>"PAUSE",Self::Resume=>"RESUME",Self::Delete=>"DELETE"}}
}
pub(crate) struct SecretaryRoutineChange<'a> {
    pub(crate) routine_id:&'a str,
    pub(crate) expected_revision:i64,
    pub(crate) request_id:&'a str,
    pub(crate) request_bytes:&'a [u8],
    pub(crate) command:SecretaryRoutineCommand,
    /// Resume requires a freshly resolved future time. There is no catch-up.
    pub(crate) next_due_ms:Option<i64>,
    pub(crate) now_ms:i64,
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub(crate) enum UserPresenceKind {Foreground,Open,Input}
impl UserPresenceKind {
    fn sql(self)->&'static str {match self {Self::Foreground=>"FOREGROUND",Self::Open=>"OPEN",Self::Input=>"INPUT"}}
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub(crate) enum SecretaryOccurrenceOutcome {Delivered,Failed,Unknown}
impl SecretaryOccurrenceOutcome {
    fn sql(self)->&'static str {match self {Self::Delivered=>"DELIVERED",Self::Failed=>"FAILED",Self::Unknown=>"UNKNOWN"}}
}
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) enum SecretaryRoutineDecision {
    NotDue,
    MissingFacts,
    PausedForAbsence {revision:i64,elapsed_ms:i64},
    /// Durable unknown intent. Root must compose H prepare/journal before
    /// committing this transaction; it must never call a model from this fact.
    Reserved {occurrence_id:String,revision:i64},
}
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct SecretaryPresenceFact {
    pub(crate) source_id:String,
    pub(crate) kind:String,
    pub(crate) source_operation_id:String,
    pub(crate) source_epoch:String,
    pub(crate) source_cursor:String,
    pub(crate) occurred_at_ms:i64,
    pub(crate) observed_at_ms:i64,
}
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct SecretaryAbsencePolicyFact {
    pub(crate) revision:i64,
    pub(crate) max_absent_ms:i64,
    pub(crate) source_id:String,
}
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct SecretaryOccurrenceFact {
    pub(crate) occurrence_id:String,
    pub(crate) due_ms:i64,
    pub(crate) state:String,
    pub(crate) h_receipt_id:String,
    pub(crate) original_reason:String,
}

fn positive(value:i64,name:&'static str)->Result<(),SeatError> {
    if value<=0 {Err(SeatError::Invalid(name))} else {Ok(())}
}
fn original(value:&str,name:&'static str)->Result<(),SeatError> {
    if value.is_empty()||value.len()>crate::ipc::MAX_FRAME_BYTES {Err(SeatError::Invalid(name))}
    else {Ok(())}
}
fn current_secretary(db:&VerifiedDatabaseConnection<'_>)->Result<Seat,SeatError> {
    let (seat_id,incarnation,_,_)=super::secretary::designation(db)?.ok_or(SeatError::Denied)?;
    let seat=read(db,"global",&seat_id)?.ok_or(SeatError::SchemaDrift)?;
    if seat.incarnation!=incarnation||seat.layer!=Layer::User||seat.kind!=Kind::Long||
        seat.parent_seat_id.is_some()||seat.state==State::Reclaimed {
        return Err(SeatError::Denied);
    }
    Ok(seat)
}
fn routine(db:&VerifiedDatabaseConnection<'_>,id:&str)->Result<Option<SecretaryRoutine>,SeatError> {
    let q=Statement::prepare(db.as_ptr(),"SELECT routine_id,seat_id,incarnation,original_text,source_operation_id,source_epoch,source_cursor,schedule_raw,timezone,CAST(next_due_ms AS TEXT),state,CAST(revision AS TEXT),last_occurrence_id,last_result,last_reason FROM main.gogoke_v37_seat_secretary_routines WHERE routine_id=?1")?;
    q.bind_text(1,id)?;
    if !q.step_row()? {return Ok(None);}
    let row=SecretaryRoutine {routine_id:q.column_text(0)?,seat_id:q.column_text(1)?,incarnation:q.column_text(2)?,
        original_text:q.column_text(3)?,source_operation_id:q.column_text(4)?,source_epoch:q.column_text(5)?,
        source_cursor:q.column_text(6)?,schedule_raw:q.column_text(7)?,timezone:q.column_text(8)?,
        next_due_ms:q.column_text(9)?.parse().map_err(|_|SeatError::SchemaDrift)?,state:q.column_text(10)?,
        revision:q.column_text(11)?.parse().map_err(|_|SeatError::SchemaDrift)?,last_occurrence_id:q.column_text(12)?,
        last_result:q.column_text(13)?,last_reason:q.column_text(14)?};
    if q.step_row()? {return Err(SeatError::SchemaDrift);}
    Ok(Some(row))
}
fn operation(db:&VerifiedDatabaseConnection<'_>,request_id:&str,expected_fp:&str,
    routine_id:&str,command:&str)->Result<Option<i64>,SeatError> {
    let q=Statement::prepare(db.as_ptr(),"SELECT fingerprint,routine_id,command,CAST(revision AS TEXT) FROM main.gogoke_v37_seat_secretary_routine_operations WHERE request_id=?1")?;
    q.bind_text(1,request_id)?;
    if !q.step_row()? {return Ok(None);}
    let same=q.column_text(0)?==expected_fp&&q.column_text(1)?==routine_id&&q.column_text(2)?==command;
    let revision=q.column_text(3)?.parse().map_err(|_|SeatError::SchemaDrift)?;
    if q.step_row()? {return Err(SeatError::SchemaDrift);}
    if !same {return Err(SeatError::Conflict);}
    Ok(Some(revision))
}
fn record_operation(db:&VerifiedDatabaseConnection<'_>,request_id:&str,fp:&str,id:&str,
    command:&str,revision:i64)->Result<(),SeatError> {
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO main.gogoke_v37_seat_secretary_routine_operations(request_id,fingerprint,routine_id,command,revision) VALUES(?1,?2,?3,?4,?5)")?;
    q.bind_text(1,request_id)?;q.bind_text(2,fp)?;q.bind_text(3,id)?;q.bind_text(4,command)?;
    q.bind_i64(5,revision)?;q.step_done()?;Ok(())
}
fn has_unresolved_occurrence(db:&VerifiedDatabaseConnection<'_>,routine_id:&str)->Result<bool,SeatError> {
    let q=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_v37_seat_secretary_occurrences WHERE routine_id=?1 AND state='UNKNOWN' LIMIT 1")?;
    q.bind_text(1,routine_id)?;
    Ok(q.step_row()?)
}

/// User-only storage entry. Root must first bind the input locator to the
/// original authenticated User ledger input, and resolve schedule/timezone
/// there. This API cannot grant a model tool or infer a schedule itself.
pub(crate) fn create_secretary_routine(db:&mut VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer,
    input:SecretaryRoutineCreate<'_>)->Result<(SecretaryRoutine,bool),SeatError> {
    validate_create(&input)?;
    transact(db,|db| {
        check_current_owner(db,issuer)?;
        let seat=current_secretary(db)?;
        create_secretary_routine_core(db,&seat,&input)
    })
}

/// H-only model path. The caller, original USER source and this E write must
/// all be checked in one existing BEGIN IMMEDIATE owned by ProductDatabase.
/// This deliberately accepts no OwnerIssuer and starts no nested transaction.
pub(crate) fn create_secretary_routine_from_model_in_transaction(
    db:&VerifiedDatabaseConnection<'_>,caller:&NativeSeatCall,input:SecretaryRoutineCreate<'_>,
)->Result<(SecretaryRoutine,bool),SeatError> {
    if caller.tool()!=Some("gogoke_routine") || caller.domain_id()!="global" {
        return Err(SeatError::Denied);
    }
    let actual=super::policy::current_caller(db,caller)?;
    let seat=current_secretary(db)?;
    if actual.seat_id!=seat.seat_id || actual.incarnation!=seat.incarnation
        || actual.generation!=seat.generation || actual.state!=State::Busy {
        return Err(SeatError::Denied);
    }
    create_secretary_routine_core(db,&seat,&input)
}

/// A replay reads the already committed E result. Re-resolving a relative
/// time against a later clock would change its fingerprint and must not occur.
pub(crate) fn replay_secretary_routine_from_model_in_transaction(
    db:&VerifiedDatabaseConnection<'_>,caller:&NativeSeatCall,routine_id:&str,request_id:&str,
    request_bytes:&[u8],original_text:&str,source_operation_id:&str,
    source_epoch:&str,source_cursor:&str,
)->Result<Option<SecretaryRoutine>,SeatError> {
    validate("global",routine_id,request_id,request_bytes)?;
    if caller.tool()!=Some("gogoke_routine") || caller.domain_id()!="global" {
        return Err(SeatError::Denied);
    }
    let actual=super::policy::current_caller(db,caller)?;
    let seat=current_secretary(db)?;
    if actual.seat_id!=seat.seat_id || actual.incarnation!=seat.incarnation
        || actual.generation!=seat.generation || actual.state!=State::Busy {
        return Err(SeatError::Denied);
    }
    let Some(row)=routine(db,routine_id)? else {return Ok(None)};
    if row.incarnation!=seat.incarnation || row.original_text!=original_text
        || row.source_operation_id!=source_operation_id || row.source_epoch!=source_epoch
        || row.source_cursor!=source_cursor {return Err(SeatError::Conflict);}
    let fp=fingerprint(&["secretary-routine-create",routine_id,original_text,
        source_operation_id,source_epoch,source_cursor,&row.schedule_raw,
        &row.timezone,&row.next_due_ms.to_string()],request_bytes);
    let Some(revision)=operation(db,request_id,&fp,routine_id,"CREATE")? else {
        return Err(SeatError::Conflict);
    };
    if row.revision!=revision {return Err(SeatError::Conflict);}
    Ok(Some(row))
}

fn validate_create(input:&SecretaryRoutineCreate<'_>)->Result<(),SeatError> {
    validate("global",input.routine_id,input.request_id,input.request_bytes)?;
    for (value,name) in [(input.original_text,"original_text"),(input.source_operation_id,"source_operation_id"),
        (input.source_epoch,"source_epoch"),(input.source_cursor,"source_cursor"),
        (input.schedule_raw,"schedule_raw"),(input.timezone,"timezone")] {original(value,name)?;}
    positive(input.next_due_ms,"next_due_ms")?;
    positive(input.now_ms,"now_ms")?;
    Ok(())
}
fn create_secretary_routine_core(db:&VerifiedDatabaseConnection<'_>,seat:&Seat,
    input:&SecretaryRoutineCreate<'_>)->Result<(SecretaryRoutine,bool),SeatError> {
    validate_create(input)?;
    let fp=fingerprint(&["secretary-routine-create",input.routine_id,input.original_text,
        input.source_operation_id,input.source_epoch,input.source_cursor,input.schedule_raw,
        input.timezone,&input.next_due_ms.to_string()],input.request_bytes);
        if let Some(recorded_revision) = operation(db,input.request_id,&fp,input.routine_id,"CREATE")? {
            let row=routine(db,input.routine_id)?.ok_or(SeatError::SchemaDrift)?;
            if row.incarnation!=seat.incarnation {return Err(SeatError::Denied);}
            if row.revision!=recorded_revision {return Err(SeatError::Conflict);}
            return Ok((row,true));
        }
        if input.next_due_ms<=input.now_ms {return Err(SeatError::Invalid("next_due_ms"));}
        if routine(db,input.routine_id)?.is_some() {return Err(SeatError::Conflict);}
        let q=Statement::prepare(db.as_ptr(),"INSERT INTO main.gogoke_v37_seat_secretary_routines(routine_id,seat_id,incarnation,original_text,source_operation_id,source_epoch,source_cursor,schedule_raw,timezone,next_due_ms,state,revision,last_occurrence_id,last_result,last_reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'ACTIVE',1,'','NONE','')")?;
        for (index,value) in [input.routine_id,seat.seat_id.as_str(),seat.incarnation.as_str(),input.original_text,
            input.source_operation_id,input.source_epoch,input.source_cursor,input.schedule_raw,input.timezone].iter().enumerate() {
            q.bind_text((index+1) as i32,value)?;
        }
        q.bind_i64(10,input.next_due_ms)?;q.step_done()?;
        record_operation(db,input.request_id,&fp,input.routine_id,"CREATE",1)?;
        Ok((routine(db,input.routine_id)?.ok_or(SeatError::SchemaDrift)?,false))
}

pub(crate) fn change_secretary_routine(db:&mut VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer,
    input:SecretaryRoutineChange<'_>)->Result<(SecretaryRoutine,bool),SeatError> {
    validate("global",input.routine_id,input.request_id,input.request_bytes)?;
    positive(input.expected_revision,"expected_revision")?;
    positive(input.now_ms,"now_ms")?;
    if input.command==SecretaryRoutineCommand::Resume {
        let next=input.next_due_ms.ok_or(SeatError::Invalid("next_due_ms"))?;
        positive(next,"next_due_ms")?;
    } else if input.next_due_ms.is_some() {return Err(SeatError::Invalid("next_due_ms"));}
    let next=input.next_due_ms.unwrap_or(0);
    let command=input.command.sql();
    let fp=fingerprint(&["secretary-routine-change",input.routine_id,command,
        &input.expected_revision.to_string(),&next.to_string()],input.request_bytes);
    transact(db,|db| {
        check_current_owner(db,issuer)?;
        let seat=current_secretary(db)?;
        if let Some(recorded_revision)=operation(db,input.request_id,&fp,input.routine_id,command)? {
            let row=routine(db,input.routine_id)?.ok_or(SeatError::SchemaDrift)?;
            if row.incarnation!=seat.incarnation {return Err(SeatError::Denied);}
            if row.revision!=recorded_revision {return Err(SeatError::Conflict);}
            return Ok((row,true));
        }
        if input.command==SecretaryRoutineCommand::Resume&&next<=input.now_ms {
            return Err(SeatError::Invalid("next_due_ms"));
        }
        let before=routine(db,input.routine_id)?.ok_or(SeatError::Denied)?;
        if before.incarnation!=seat.incarnation||before.seat_id!=seat.seat_id {return Err(SeatError::Denied);}
        if before.revision!=input.expected_revision {return Err(SeatError::Conflict);}
        if input.command==SecretaryRoutineCommand::Resume&&has_unresolved_occurrence(db,input.routine_id)? {
            return Err(SeatError::Denied);
        }
        let state=match input.command {
            SecretaryRoutineCommand::Pause if before.state=="ACTIVE"||before.state=="WAITING_NEXT"=>"PAUSED",
            SecretaryRoutineCommand::Resume if before.state=="PAUSED"||before.state=="ABSENCE_PAUSED"||before.state=="WAITING_NEXT"=>"ACTIVE",
            SecretaryRoutineCommand::Delete if before.state!="DELETED"=>"DELETED",
            _=>return Err(SeatError::Denied),
        };
        let revision=before.revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let due=if input.command==SecretaryRoutineCommand::Resume {next}else{before.next_due_ms};
        let q=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_seat_secretary_routines SET state=?1,next_due_ms=?2,revision=?3 WHERE routine_id=?4 AND revision=?5")?;
        q.bind_text(1,state)?;q.bind_i64(2,due)?;q.bind_i64(3,revision)?;
        q.bind_text(4,input.routine_id)?;q.bind_i64(5,before.revision)?;q.step_done()?;
        record_operation(db,input.request_id,&fp,input.routine_id,command,revision)?;
        let after=routine(db,input.routine_id)?.ok_or(SeatError::SchemaDrift)?;
        if after.revision!=revision {return Err(SeatError::Conflict);}
        Ok((after,false))
    })
}

/// The caller supplies the current transaction. Read includes tombstones and
/// original text/reason; no projection silently turns UNKNOWN into success.
pub(crate) fn read_secretary_routines_in_transaction(db:&VerifiedDatabaseConnection<'_>,
    issuer:&OwnerIssuer)->Result<Vec<SecretaryRoutine>,SeatError> {
    check_current_owner(db,issuer)?;
    let seat=current_secretary(db)?;
    let q=Statement::prepare(db.as_ptr(),"SELECT routine_id FROM main.gogoke_v37_seat_secretary_routines WHERE seat_id=?1 AND incarnation=?2 ORDER BY routine_id")?;
    q.bind_text(1,&seat.seat_id)?;q.bind_text(2,&seat.incarnation)?;
    let mut ids=Vec::new();while q.step_row()? {ids.push(q.column_text(0)?);}
    ids.into_iter().map(|id|routine(db,&id)?.ok_or(SeatError::SchemaDrift)).collect()
}

pub(crate) fn read_secretary_presence_in_transaction(db:&VerifiedDatabaseConnection<'_>,
    issuer:&OwnerIssuer)->Result<(Option<SecretaryPresenceFact>,Option<SecretaryAbsencePolicyFact>),SeatError> {
    check_current_owner(db,issuer)?;current_secretary(db)?;
    let q=Statement::prepare(db.as_ptr(),"SELECT source_id,kind,source_operation_id,source_epoch,source_cursor,CAST(occurred_at_ms AS TEXT),CAST(observed_at_ms AS TEXT) FROM main.gogoke_v37_seat_secretary_presence ORDER BY occurred_at_ms DESC,observed_at_ms DESC LIMIT 1")?;
    let presence=if q.step_row()? {Some(SecretaryPresenceFact {
        source_id:q.column_text(0)?,kind:q.column_text(1)?,source_operation_id:q.column_text(2)?,
        source_epoch:q.column_text(3)?,source_cursor:q.column_text(4)?,
        occurred_at_ms:q.column_text(5)?.parse().map_err(|_|SeatError::SchemaDrift)?,
        observed_at_ms:q.column_text(6)?.parse().map_err(|_|SeatError::SchemaDrift)?,
    })} else {None};
    if q.step_row()? {return Err(SeatError::SchemaDrift);}
    let q=Statement::prepare(db.as_ptr(),"SELECT CAST(revision AS TEXT),CAST(max_absent_ms AS TEXT),source_id FROM main.gogoke_v37_seat_secretary_absence_policy WHERE singleton=1")?;
    let policy=if q.step_row()? {Some(SecretaryAbsencePolicyFact {
        revision:q.column_text(0)?.parse().map_err(|_|SeatError::SchemaDrift)?,
        max_absent_ms:q.column_text(1)?.parse().map_err(|_|SeatError::SchemaDrift)?,
        source_id:q.column_text(2)?,
    })} else {None};
    if q.step_row()? {return Err(SeatError::SchemaDrift);}
    Ok((presence,policy))
}

pub(crate) fn read_secretary_occurrences_in_transaction(db:&VerifiedDatabaseConnection<'_>,
    issuer:&OwnerIssuer,routine_id:&str)->Result<Vec<SecretaryOccurrenceFact>,SeatError> {
    check_current_owner(db,issuer)?;
    if !valid_id(routine_id) {return Err(SeatError::Invalid("routine_id"));}
    let seat=current_secretary(db)?;
    let row=routine(db,routine_id)?.ok_or(SeatError::Denied)?;
    if row.seat_id!=seat.seat_id||row.incarnation!=seat.incarnation {return Err(SeatError::Denied);}
    let q=Statement::prepare(db.as_ptr(),"SELECT occurrence_id,CAST(due_ms AS TEXT),state,h_receipt_id,original_reason FROM main.gogoke_v37_seat_secretary_occurrences WHERE routine_id=?1 ORDER BY due_ms,occurrence_id")?;
    q.bind_text(1,routine_id)?;
    let mut rows=Vec::new();
    while q.step_row()? {rows.push(SecretaryOccurrenceFact {
        occurrence_id:q.column_text(0)?,due_ms:q.column_text(1)?.parse().map_err(|_|SeatError::SchemaDrift)?,
        state:q.column_text(2)?,h_receipt_id:q.column_text(3)?,original_reason:q.column_text(4)?,
    });}
    Ok(rows)
}

/// Root only calls this for an attributable native User foreground/open/input
/// event. A heartbeat, model output, routine check, or polling result is not
/// an eligible source. The source locator must identify that original event.
pub(crate) fn record_user_presence(db:&mut VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer,
    source_id:&str,kind:UserPresenceKind,source_operation_id:&str,source_epoch:&str,
    source_cursor:&str,occurred_at_ms:i64,observed_at_ms:i64)->Result<bool,SeatError> {
    transact(db,|db|record_user_presence_in_transaction(db,issuer,source_id,kind,
        source_operation_id,source_epoch,source_cursor,occurred_at_ms,observed_at_ms))
}

/// E remains the writer. H may compose this only after inserting the exact
/// authenticated User request in its existing transaction, before stdin write.
pub(crate) fn record_user_presence_in_transaction(db:&VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer,
    source_id:&str,kind:UserPresenceKind,source_operation_id:&str,source_epoch:&str,
    source_cursor:&str,occurred_at_ms:i64,observed_at_ms:i64)->Result<bool,SeatError> {
    for (value,name) in [(source_id,"source_id"),(source_operation_id,"source_operation_id"),
        (source_epoch,"source_epoch"),(source_cursor,"source_cursor")] {original(value,name)?;}
    positive(occurred_at_ms,"occurred_at_ms")?;positive(observed_at_ms,"observed_at_ms")?;
    if observed_at_ms<occurred_at_ms {return Err(SeatError::Invalid("presence_clock"));}
        check_current_owner(db,issuer)?;current_secretary(db)?;
        let q=Statement::prepare(db.as_ptr(),"SELECT kind,source_operation_id,source_epoch,source_cursor,CAST(occurred_at_ms AS TEXT),CAST(observed_at_ms AS TEXT) FROM main.gogoke_v37_seat_secretary_presence WHERE source_id=?1")?;
        q.bind_text(1,source_id)?;
        if q.step_row()? {
            let same=q.column_text(0)?==kind.sql()&&q.column_text(1)?==source_operation_id&&
                q.column_text(2)?==source_epoch&&q.column_text(3)?==source_cursor&&
                q.column_text(4)?==occurred_at_ms.to_string()&&q.column_text(5)?==observed_at_ms.to_string();
            if q.step_row()? {return Err(SeatError::SchemaDrift);}
            return if same {Ok(true)} else {Err(SeatError::Conflict)};
        }
        let q=Statement::prepare(db.as_ptr(),"INSERT INTO main.gogoke_v37_seat_secretary_presence(source_id,kind,source_operation_id,source_epoch,source_cursor,occurred_at_ms,observed_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
        for (index,value) in [source_id,kind.sql(),source_operation_id,source_epoch,source_cursor].iter().enumerate() {q.bind_text((index+1) as i32,value)?;}
        q.bind_i64(6,occurred_at_ms)?;q.bind_i64(7,observed_at_ms)?;q.step_done()?;Ok(false)
}

/// Explicit User policy; absence of this row is UNKNOWN, not a default.
pub(crate) fn configure_absence_policy(db:&mut VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer,
    expected_revision:Option<i64>,max_absent_ms:i64,source_id:&str)->Result<i64,SeatError> {
    positive(max_absent_ms,"max_absent_ms")?;original(source_id,"source_id")?;
    if expected_revision.is_some_and(|revision|revision<=0) {return Err(SeatError::Invalid("expected_revision"));}
    transact(db,|db| {
        check_current_owner(db,issuer)?;current_secretary(db)?;
        let q=Statement::prepare(db.as_ptr(),"SELECT CAST(revision AS TEXT) FROM main.gogoke_v37_seat_secretary_absence_policy WHERE singleton=1")?;
        let current=if q.step_row()? {Some(q.column_text(0)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?)} else {None};
        if q.step_row()? {return Err(SeatError::SchemaDrift);}
        if current!=expected_revision {return Err(SeatError::Conflict);}
        let next=current.unwrap_or(0).checked_add(1).ok_or(SeatError::Conflict)?;
        if current.is_some() {
            let q=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_seat_secretary_absence_policy SET revision=?1,max_absent_ms=?2,source_id=?3 WHERE singleton=1")?;
            q.bind_i64(1,next)?;q.bind_i64(2,max_absent_ms)?;q.bind_text(3,source_id)?;q.step_done()?;
        } else {
            let q=Statement::prepare(db.as_ptr(),"INSERT INTO main.gogoke_v37_seat_secretary_absence_policy(singleton,revision,max_absent_ms,source_id) VALUES(1,1,?1,?2)")?;
            q.bind_i64(1,max_absent_ms)?;q.bind_text(2,source_id)?;q.step_done()?;
        }
        Ok(next)
    })
}

/// Compose inside the existing coordinator's BEGIN IMMEDIATE transaction,
/// followed by H prepare/journal in that same transaction. The result alone
/// is never a launch authorization. An unknown occurrence is never retried.
pub(crate) fn take_due_secretary_routine_in_transaction(db:&VerifiedDatabaseConnection<'_>,
    issuer:&OwnerIssuer,routine_id:&str,expected_revision:i64,now_ms:i64)
    ->Result<SecretaryRoutineDecision,SeatError> {
    check_current_owner(db,issuer)?;
    if !valid_id(routine_id)||expected_revision<=0||now_ms<=0 {return Err(SeatError::Invalid("due_request"));}
    let seat=current_secretary(db)?;
    match super::secretary::read_secretary_configuration_in_transaction(db,issuer)? {
        super::secretary::SecretaryConfiguration::Designated {
            instance_id:Some(_),model:Some(_),effort:Some(_),permission:Some(_),..
        }=>{},
        _=>return Ok(SecretaryRoutineDecision::MissingFacts),
    }
    let row=routine(db,routine_id)?.ok_or(SeatError::Denied)?;
    if row.seat_id!=seat.seat_id||row.incarnation!=seat.incarnation {return Err(SeatError::Denied);}
    if row.revision!=expected_revision {return Err(SeatError::Conflict);}
    if has_unresolved_occurrence(db,routine_id)? {return Err(SeatError::Denied);}
    if row.state!="ACTIVE"||row.next_due_ms==0||row.next_due_ms>now_ms {
        return Ok(SecretaryRoutineDecision::NotDue);
    }
    let policy=Statement::prepare(db.as_ptr(),"SELECT CAST(max_absent_ms AS TEXT) FROM main.gogoke_v37_seat_secretary_absence_policy WHERE singleton=1")?;
    if !policy.step_row()? {return Ok(SecretaryRoutineDecision::MissingFacts);}
    let max_absent=policy.column_text(0)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
    if policy.step_row()?||max_absent<=0 {return Err(SeatError::SchemaDrift);}
    let presence=Statement::prepare(db.as_ptr(),"SELECT CAST(occurred_at_ms AS TEXT),CAST(observed_at_ms AS TEXT) FROM main.gogoke_v37_seat_secretary_presence ORDER BY occurred_at_ms DESC,observed_at_ms DESC LIMIT 1")?;
    if !presence.step_row()? {return Ok(SecretaryRoutineDecision::MissingFacts);}
    let occurred=presence.column_text(0)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
    let observed=presence.column_text(1)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
    if presence.step_row()?||occurred<=0||observed<occurred {return Err(SeatError::SchemaDrift);}
    if now_ms<observed {return Ok(SecretaryRoutineDecision::MissingFacts);}
    let elapsed=now_ms.checked_sub(occurred).ok_or(SeatError::Unknown)?;
    let revision=row.revision.checked_add(1).ok_or(SeatError::Conflict)?;
    if elapsed>=max_absent {
        let q=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_seat_secretary_routines SET state='ABSENCE_PAUSED',revision=?1 WHERE routine_id=?2 AND revision=?3")?;
        q.bind_i64(1,revision)?;q.bind_text(2,routine_id)?;q.bind_i64(3,row.revision)?;q.step_done()?;
        if routine(db,routine_id)?.ok_or(SeatError::SchemaDrift)?.revision!=revision {return Err(SeatError::Conflict);}
        return Ok(SecretaryRoutineDecision::PausedForAbsence {revision,elapsed_ms:elapsed});
    }
    // A deterministic occurrence key and UNIQUE(routine_id,due_ms) prevent
    // a restart or competing coordinator from claiming this time twice.
    let digest=fingerprint(&["secretary-occurrence",routine_id,&row.next_due_ms.to_string()],b"");
    let hex=digest.strip_prefix("sha256:").ok_or(SeatError::SchemaDrift)?;
    let occurrence_id=format!("occ-{hex}");
    if !valid_id(&occurrence_id) {return Err(SeatError::SchemaDrift);}
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO main.gogoke_v37_seat_secretary_occurrences(occurrence_id,routine_id,due_ms,state,h_receipt_id,original_reason) VALUES(?1,?2,?3,'UNKNOWN','','')")?;
    q.bind_text(1,&occurrence_id)?;q.bind_text(2,routine_id)?;q.bind_i64(3,row.next_due_ms)?;q.step_done()?;
    let q=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_seat_secretary_routines SET state='WAITING_NEXT',revision=?1,last_occurrence_id=?2,last_result='UNKNOWN',last_reason='' WHERE routine_id=?3 AND revision=?4")?;
    q.bind_i64(1,revision)?;q.bind_text(2,&occurrence_id)?;q.bind_text(3,routine_id)?;
    q.bind_i64(4,row.revision)?;q.step_done()?;
    if routine(db,routine_id)?.ok_or(SeatError::SchemaDrift)?.revision!=revision {return Err(SeatError::Conflict);}
    Ok(SecretaryRoutineDecision::Reserved {occurrence_id,revision})
}

/// Root calls only after verifying the exact original H journal entry in
/// this same transaction. `h_receipt_id` is a locator, not proof by itself.
/// UNKNOWN remains non-replayable and cannot schedule a further occurrence.
pub(crate) fn record_secretary_occurrence_outcome_in_transaction(
    db:&VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer,routine_id:&str,
    occurrence_id:&str,expected_revision:i64,outcome:SecretaryOccurrenceOutcome,
    h_receipt_id:&str,original_reason:&str,next_due_ms:Option<i64>,now_ms:i64,
)->Result<SecretaryRoutine,SeatError> {
    check_current_owner(db,issuer)?;
    if !valid_id(routine_id)||!valid_id(occurrence_id)||expected_revision<=0||now_ms<=0 {
        return Err(SeatError::Invalid("occurrence_outcome"));
    }
    if original_reason.len()>crate::ipc::MAX_FRAME_BYTES {return Err(SeatError::Invalid("original_reason"));}
    if outcome!=SecretaryOccurrenceOutcome::Delivered&&original_reason.is_empty() {
        return Err(SeatError::Invalid("original_reason"));
    }
    if outcome!=SecretaryOccurrenceOutcome::Unknown {
        original(h_receipt_id,"h_receipt_id")?;
    }
    if outcome==SecretaryOccurrenceOutcome::Unknown&&next_due_ms.is_some() {
        return Err(SeatError::Invalid("next_due_ms"));
    }
    if next_due_ms.is_some_and(|next|next<=now_ms) {
        return Err(SeatError::Invalid("next_due_ms"));
    }
    let seat=current_secretary(db)?;
    let before=routine(db,routine_id)?.ok_or(SeatError::Denied)?;
    if before.seat_id!=seat.seat_id||before.incarnation!=seat.incarnation {return Err(SeatError::Denied);}
    if before.revision!=expected_revision {return Err(SeatError::Conflict);}
    if before.last_occurrence_id!=occurrence_id {return Err(SeatError::Denied);}
    if before.state!="WAITING_NEXT"&&before.state!="PAUSED"&&before.state!="DELETED" {
        return Err(SeatError::Denied);
    }
    let q=Statement::prepare(db.as_ptr(),"SELECT state FROM main.gogoke_v37_seat_secretary_occurrences WHERE occurrence_id=?1 AND routine_id=?2")?;
    q.bind_text(1,occurrence_id)?;q.bind_text(2,routine_id)?;
    if !q.step_row()?||q.column_text(0)?!="UNKNOWN" {return Err(SeatError::Denied);}
    if q.step_row()? {return Err(SeatError::SchemaDrift);}
    let q=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_seat_secretary_occurrences SET state=?1,h_receipt_id=?2,original_reason=?3 WHERE occurrence_id=?4 AND routine_id=?5 AND state='UNKNOWN'")?;
    q.bind_text(1,outcome.sql())?;q.bind_text(2,h_receipt_id)?;
    q.bind_text(3,original_reason)?;q.bind_text(4,occurrence_id)?;q.bind_text(5,routine_id)?;q.step_done()?;
    let revision=before.revision.checked_add(1).ok_or(SeatError::Conflict)?;
    let state=if before.state=="WAITING_NEXT"&&outcome!=SecretaryOccurrenceOutcome::Unknown&&next_due_ms.is_some() {
        "ACTIVE"
    } else {before.state.as_str()};
    let due=if before.state=="WAITING_NEXT"&&outcome!=SecretaryOccurrenceOutcome::Unknown {
        next_due_ms.unwrap_or(0)
    } else {before.next_due_ms};
    let q=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_seat_secretary_routines SET state=?1,next_due_ms=?2,revision=?3,last_result=?4,last_reason=?5 WHERE routine_id=?6 AND revision=?7")?;
    q.bind_text(1,state)?;q.bind_i64(2,due)?;q.bind_i64(3,revision)?;
    q.bind_text(4,outcome.sql())?;q.bind_text(5,original_reason)?;
    q.bind_text(6,routine_id)?;q.bind_i64(7,before.revision)?;q.step_done()?;
    let after=routine(db,routine_id)?.ok_or(SeatError::SchemaDrift)?;
    if after.revision!=revision {return Err(SeatError::Conflict);}
    Ok(after)
}
