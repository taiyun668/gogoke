//! Seat continuity: the copied template supplies questions, the seat card
//! holds current work, and H's observed health chooses an exact session action.

use super::*;
use std::collections::BTreeSet;

pub(super) const CARDS: &str = "CREATE TABLE gogoke_v37_seat_cards(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,card_json TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),PRIMARY KEY(domain_id,seat_id),FOREIGN KEY(domain_id,seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT";
pub(super) const ANSWERS: &str = "CREATE TABLE gogoke_v37_seat_takeover_answers(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,question_id TEXT NOT NULL,instance_id TEXT NOT NULL,answer TEXT NOT NULL,basis TEXT NOT NULL CHECK(basis IN ('CITED','UNKNOWN')),source_ref TEXT NOT NULL,how_to_find TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),PRIMARY KEY(domain_id,seat_id,question_id),FOREIGN KEY(domain_id,seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT";
pub(super) const OPERATIONS: &str = "CREATE TABLE gogoke_v37_seat_continuity_operations(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,operation TEXT NOT NULL,fingerprint TEXT NOT NULL,seat_id TEXT NOT NULL,revision INTEGER NOT NULL,state TEXT NOT NULL,PRIMARY KEY(domain_id,request_id)) STRICT";
pub(super) const HEALTH: &str = "CREATE TABLE gogoke_v37_seat_health(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,event_id TEXT NOT NULL,generation INTEGER NOT NULL,signal TEXT NOT NULL CHECK(signal IN ('HEALTHY','CONTEXT_COMPACT','CONTEXT_RENEW','REPEATED_FAILURE','CRASH','STALLED','UNKNOWN')),source_event_id TEXT NOT NULL,action TEXT NOT NULL CHECK(action IN ('NONE','COMPACT','RENEW','ESCALATE')),state TEXT NOT NULL CHECK(state IN ('OBSERVED','REQUESTED','RECEIPTED','UNKNOWN')),session_request_id TEXT,receipt_id TEXT,PRIMARY KEY(domain_id,event_id)) STRICT";

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct TakeoverQuestion {pub(crate) id:String,pub(crate) prompt:String}

fn required_string(fields:&std::collections::BTreeMap<JsonString,Json>,key:&str)
    ->Result<String,SeatError> {
    let Some(Json::String(value))=fields.get(&JsonString::from_str(key)) else {
        return Err(SeatError::Invalid("takeover question"));
    };
    let text=value.to_well_formed_string().ok_or(SeatError::Invalid("takeover question"))?;
    if text.is_empty()||text.len()>4096 {return Err(SeatError::Invalid("takeover question"));}
    Ok(text)
}

pub(super) fn validate_takeover_template(settings:&Json)->Result<(),SeatError> {
    let Json::Object(fields)=settings else {return Err(SeatError::Invalid("template settings"));};
    let Some(value)=fields.get(&JsonString::from_str("takeoverQuestions")) else {return Ok(());};
    parse_questions(value).map(|_|())
}

fn parse_questions(value:&Json)->Result<Vec<TakeoverQuestion>,SeatError> {
    let Json::Array(items)=value else {return Err(SeatError::Invalid("takeoverQuestions"));};
    if items.is_empty()||items.len()>32 {return Err(SeatError::Invalid("takeoverQuestions"));}
    let mut seen=BTreeSet::new();let mut result=Vec::new();
    for item in items {
        let Json::Object(fields)=item else {return Err(SeatError::Invalid("takeoverQuestions"));};
        if fields.len()!=2 {return Err(SeatError::Invalid("takeoverQuestions"));}
        let id=required_string(fields,"id")?;
        let prompt=required_string(fields,"prompt")?;
        if !valid_id(&id)||!seen.insert(id.clone()) {return Err(SeatError::Invalid("takeoverQuestions"));}
        result.push(TakeoverQuestion {id,prompt});
    }
    Ok(result)
}

pub(crate) fn takeover_questions(seat:&Seat)->Result<Vec<TakeoverQuestion>,SeatError> {
    if seat.state==State::Reclaimed {return Err(SeatError::Denied);}
    let settings=seat.settings_json.as_deref().ok_or(SeatError::Denied)?;
    let Json::Object(fields)=Parser::parse(settings)? else {return Err(SeatError::SchemaDrift);};
    let questions=fields.get(&JsonString::from_str("takeoverQuestions"))
        .ok_or(SeatError::Denied)?;
    parse_questions(questions)
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) enum AnswerBasis {Cited {source_ref:String},Unknown {how_to_find:String}}

fn prior_operation(db:&VerifiedDatabaseConnection<'_>,domain:&str,request:&str,
    operation:&str,fp:&str)->Result<Option<i64>,SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT operation,fingerprint,revision FROM main.gogoke_v37_seat_continuity_operations WHERE domain_id=?1 AND request_id=?2")?;
    q.bind_text(1,domain)?;q.bind_text(2,request)?;
    if !q.step_row()? {return Ok(None);}
    if q.column_text(0)?!=operation||q.column_text(1)?!=fp {return Err(SeatError::Conflict);}
    let revision=q.column_text(2)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
    if q.step_row()? {return Err(SeatError::SchemaDrift);}
    Ok(Some(revision))
}
fn record_operation(db:&VerifiedDatabaseConnection<'_>,domain:&str,request:&str,
    operation:&str,fp:&str,seat:&str,revision:i64,state:&str)->Result<(),SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_seat_continuity_operations(domain_id,request_id,operation,fingerprint,seat_id,revision,state) VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
    q.bind_text(1,domain)?;q.bind_text(2,request)?;q.bind_text(3,operation)?;
    q.bind_text(4,fp)?;q.bind_text(5,seat)?;q.bind_i64(6,revision)?;
    q.bind_text(7,state)?;q.step_done()?;
    Ok(())
}

/// A takeover answer belongs to the copied question set and current binding.
/// The lead may answer its own project only through H's live native caller.
pub(crate) fn answer_takeover(db:&mut VerifiedDatabaseConnection<'_>,
    caller:&policy::NativeSeatCall,question_id:&str,answer:&str,basis:AnswerBasis,
    expected_answer_revision:i64,request_id:&str,original_raw:&[u8])->Result<i64,SeatError> {
    answer_takeover_inner(db,caller,question_id,answer,basis,None,
        expected_answer_revision,request_id,original_raw,|_|Ok(()))
}

pub(crate) fn answer_takeover_at_seat_revision(db:&mut VerifiedDatabaseConnection<'_>,
    caller:&policy::NativeSeatCall,question_id:&str,answer:&str,basis:AnswerBasis,
    expected_seat_revision:i64,expected_answer_revision:i64,request_id:&str,
    original_raw:&[u8])->Result<i64,SeatError> {
    answer_takeover_inner(db,caller,question_id,answer,basis,Some(expected_seat_revision),
        expected_answer_revision,request_id,original_raw,|_|Ok(()))
}

/// Root supplies C's written-source check, executed inside this existing E
/// transaction before the answer or its replay receipt can be changed.
pub(crate) fn answer_takeover_from_written_source(db:&mut VerifiedDatabaseConnection<'_>,
    caller:&policy::NativeSeatCall,question_id:&str,answer:&str,basis:AnswerBasis,
    expected_seat_revision:i64,expected_answer_revision:i64,request_id:&str,
    original_raw:&[u8],source:impl FnOnce(&VerifiedDatabaseConnection<'_>)->Result<(),SeatError>)
    ->Result<i64,SeatError> {
    answer_takeover_inner(db,caller,question_id,answer,basis,Some(expected_seat_revision),
        expected_answer_revision,request_id,original_raw,source)
}

fn answer_takeover_inner(db:&mut VerifiedDatabaseConnection<'_>,
    caller:&policy::NativeSeatCall,question_id:&str,answer:&str,basis:AnswerBasis,
    expected_seat_revision:Option<i64>,expected_answer_revision:i64,
    request_id:&str,original_raw:&[u8],
    authorize_source:impl FnOnce(&VerifiedDatabaseConnection<'_>)->Result<(),SeatError>)->Result<i64,SeatError> {
    validate(caller.domain_id(),question_id,request_id,original_raw)?;
    if answer.is_empty()||answer.len()>8192||expected_answer_revision<0 {
        return Err(SeatError::Invalid("takeover answer"));
    }
    let (label,source,how)=match &basis {
        AnswerBasis::Cited{source_ref} if !source_ref.is_empty()&&source_ref.len()<=4096 =>
            ("CITED",source_ref.as_str(),""),
        AnswerBasis::Unknown{how_to_find} if !how_to_find.is_empty()&&how_to_find.len()<=4096 =>
            ("UNKNOWN","",how_to_find.as_str()),
        _=>return Err(SeatError::Invalid("answer basis")),
    };
    let fp=fingerprint(&["takeover-answer",caller.seat_id(),question_id,answer,label,source,how,
        &expected_answer_revision.to_string()],original_raw);
    transact(db,|db| {
        let seat=policy::current_caller(db,caller)?;
        authorize_source(db)?;
        if seat.layer!=Layer::User||seat.instance_id.is_empty() {return Err(SeatError::Denied);}
        if expected_seat_revision.is_some_and(|expected|expected!=seat.revision) {
            return Err(SeatError::Conflict);
        }
        if let Some(old)=prior_operation(db,caller.domain_id(),request_id,"takeover-answer",&fp)? {
            return Ok(old);
        }
        if !takeover_questions(&seat)?.iter().any(|item|item.id==question_id) {
            return Err(SeatError::Denied);
        }
        let previous=Statement::prepare(db.as_ptr(),
            "SELECT revision FROM main.gogoke_v37_seat_takeover_answers WHERE domain_id=?1 AND seat_id=?2 AND question_id=?3")?;
        previous.bind_text(1,caller.domain_id())?;previous.bind_text(2,caller.seat_id())?;
        previous.bind_text(3,question_id)?;
        let before=if previous.step_row()? {
            let old=previous.column_text(0)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
            if previous.step_row()? {return Err(SeatError::SchemaDrift);}
            old
        }else{0};
        if before!=expected_answer_revision {return Err(SeatError::Conflict);}
        let revision=before.checked_add(1).ok_or(SeatError::Conflict)?;
        let q=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_takeover_answers(domain_id,seat_id,question_id,instance_id,answer,basis,source_ref,how_to_find,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(domain_id,seat_id,question_id) DO UPDATE SET instance_id=excluded.instance_id,answer=excluded.answer,basis=excluded.basis,source_ref=excluded.source_ref,how_to_find=excluded.how_to_find,revision=excluded.revision")?;
        for (index,value) in [caller.domain_id(),caller.seat_id(),question_id,seat.instance_id.as_str(),
            answer,label,source,how].iter().enumerate() {q.bind_text((index+1) as i32,value)?;}
        q.bind_i64(9,revision)?;q.step_done()?;
        record_operation(db,caller.domain_id(),request_id,"takeover-answer",&fp,
            caller.seat_id(),revision,label)?;
        Ok(revision)
    })
}

pub(crate) fn takeover_ready(db:&VerifiedDatabaseConnection<'_>,seat:&Seat)->Result<bool,SeatError> {
    if seat.layer!=Layer::User||seat.state==State::Reclaimed||seat.instance_id.is_empty() {
        return Ok(false);
    }
    let questions=takeover_questions(seat)?;
    for item in questions {
        let q=Statement::prepare(db.as_ptr(),
            "SELECT instance_id,answer,basis,source_ref,how_to_find FROM main.gogoke_v37_seat_takeover_answers WHERE domain_id=?1 AND seat_id=?2 AND question_id=?3")?;
        q.bind_text(1,&seat.domain_id)?;q.bind_text(2,&seat.seat_id)?;q.bind_text(3,&item.id)?;
        if !q.step_row()? {return Ok(false);}
        let instance=q.column_text(0)?;let answer=q.column_text(1)?;
        let basis=q.column_text(2)?;let source=q.column_text(3)?;let how=q.column_text(4)?;
        if q.step_row()? {return Err(SeatError::SchemaDrift);}
        if instance!=seat.instance_id||answer.is_empty()||
            !((basis=="CITED"&&!source.is_empty()&&how.is_empty())||
              (basis=="UNKNOWN"&&source.is_empty()&&!how.is_empty())) {return Ok(false);}
    }
    Ok(true)
}

pub(crate) fn update_state_card(db:&mut VerifiedDatabaseConnection<'_>,
    caller:&policy::NativeSeatCall,card_json:&[u8],expected_revision:i64,
    request_id:&str,original_raw:&[u8])->Result<i64,SeatError> {
    validate(caller.domain_id(),caller.seat_id(),request_id,original_raw)?;
    if card_json.is_empty()||card_json.len()>crate::ipc::MAX_FRAME_BYTES||expected_revision<0 {
        return Err(SeatError::Invalid("state card"));
    }
    super::super::atomic::require_canonical_json(card_json,"seat.state_card")?;
    let card=std::str::from_utf8(card_json).map_err(|_|SeatError::Invalid("state card"))?;
    let Json::Object(fields)=Parser::parse(card)? else {return Err(SeatError::Invalid("state card"));};
    if fields.len()!=4 || ["goal","constraints","unfinishedInstructions","pendingQuestions"].iter()
        .any(|name|!matches!(fields.get(&JsonString::from_str(name)),Some(Json::String(_)))) {
        return Err(SeatError::Invalid("state card"));
    }
    let fp=fingerprint(&["state-card",caller.seat_id(),card,&expected_revision.to_string()],original_raw);
    transact(db,|db| {
        policy::current_caller(db,caller)?;
        if let Some(old)=prior_operation(db,caller.domain_id(),request_id,"state-card",&fp)? {
            return Ok(old);
        }
        let q=Statement::prepare(db.as_ptr(),
            "SELECT revision FROM main.gogoke_v37_seat_cards WHERE domain_id=?1 AND seat_id=?2")?;
        q.bind_text(1,caller.domain_id())?;q.bind_text(2,caller.seat_id())?;
        let before=if q.step_row()? {
            let value=q.column_text(0)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
            if q.step_row()? {return Err(SeatError::SchemaDrift);}
            value
        }else{0};
        if before!=expected_revision {return Err(SeatError::Conflict);}
        let next=before.checked_add(1).ok_or(SeatError::Conflict)?;
        let write=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_cards(domain_id,seat_id,card_json,revision) VALUES(?1,?2,?3,?4) ON CONFLICT(domain_id,seat_id) DO UPDATE SET card_json=excluded.card_json,revision=excluded.revision")?;
        write.bind_text(1,caller.domain_id())?;write.bind_text(2,caller.seat_id())?;
        write.bind_text(3,card)?;write.bind_i64(4,next)?;write.step_done()?;
        record_operation(db,caller.domain_id(),request_id,"state-card",&fp,caller.seat_id(),next,"UPDATED")?;
        Ok(next)
    })
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct StateCard {
    pub(crate) card_json:Option<String>,pub(crate) revision:i64,
    pub(crate) takeover_ready:bool,
    pub(crate) takeover_answers:Vec<TakeoverAnswer>,
}
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct TakeoverAnswer {
    pub(crate) question_id:String,pub(crate) answer:String,pub(crate) basis:String,
    pub(crate) source_ref:String,pub(crate) how_to_find:String,
}
pub(crate) fn read_state_card(db:&VerifiedDatabaseConnection<'_>,seat:&Seat)
    ->Result<StateCard,SeatError> {
    let current=read(db,&seat.domain_id,&seat.seat_id)?.ok_or(SeatError::Denied)?;
    if current.incarnation!=seat.incarnation||current.state==State::Reclaimed {return Err(SeatError::Denied);}
    let q=Statement::prepare(db.as_ptr(),
        "SELECT card_json,revision FROM main.gogoke_v37_seat_cards WHERE domain_id=?1 AND seat_id=?2")?;
    q.bind_text(1,&seat.domain_id)?;q.bind_text(2,&seat.seat_id)?;
    let (card_json,revision)=if q.step_row()? {
        let card=q.column_text(0)?;let rev=q.column_text(1)?.parse::<i64>()
            .map_err(|_|SeatError::SchemaDrift)?;
        if q.step_row()? {return Err(SeatError::SchemaDrift);}
        (Some(card),rev)
    }else{(None,0)};
    let ready=if current.layer==Layer::User {match takeover_ready(db,&current) {
        Ok(value)=>value,Err(SeatError::Denied)=>false,Err(error)=>return Err(error),
    }}else{true};
    let answers=Statement::prepare(db.as_ptr(),
        "SELECT question_id,answer,basis,source_ref,how_to_find FROM main.gogoke_v37_seat_takeover_answers WHERE domain_id=?1 AND seat_id=?2 AND instance_id=?3 ORDER BY question_id")?;
    answers.bind_text(1,&seat.domain_id)?;answers.bind_text(2,&seat.seat_id)?;
    answers.bind_text(3,&current.instance_id)?;
    let mut takeover_answers=Vec::new();
    while answers.step_row()? {
        takeover_answers.push(TakeoverAnswer {question_id:answers.column_text(0)?,
            answer:answers.column_text(1)?,basis:answers.column_text(2)?,
            source_ref:answers.column_text(3)?,how_to_find:answers.column_text(4)?});
    }
    Ok(StateCard {card_json,revision,takeover_ready:ready,takeover_answers})
}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub(crate) enum HealthSignal {Healthy,ContextCompact,ContextRenew,RepeatedFailure,Crash,Stalled,Unknown}
impl HealthSignal {
    fn sql(self)->&'static str {match self {Self::Healthy=>"HEALTHY",Self::ContextCompact=>"CONTEXT_COMPACT",
        Self::ContextRenew=>"CONTEXT_RENEW",Self::RepeatedFailure=>"REPEATED_FAILURE",
        Self::Crash=>"CRASH",Self::Stalled=>"STALLED",Self::Unknown=>"UNKNOWN"}}
    fn action(self)->&'static str {match self {Self::ContextCompact=>"COMPACT",
        Self::ContextRenew|Self::RepeatedFailure|Self::Crash=>"RENEW",Self::Stalled=>"ESCALATE",
        Self::Healthy|Self::Unknown=>"NONE"}}
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct HealthObservation {pub(crate) event_id:String,pub(crate) action:String,
    pub(crate) state:String,pub(crate) generation:i64}

/// Input must be an original H/adapter health event, not model prose. E only
/// records the requested action; H owns actual K-SESSION compact/renew.
pub(crate) fn observe_health(db:&mut VerifiedDatabaseConnection<'_>,
    caller:&policy::NativeSeatCall,event_id:&str,source_event_id:&str,
    signal:HealthSignal)->Result<HealthObservation,SeatError> {
    if !valid_id(event_id)||!valid_id(source_event_id) {return Err(SeatError::Invalid("health event"));}
    transact(db,|db| {
        let seat=policy::current_caller(db,caller)?;
        let q=Statement::prepare(db.as_ptr(),
            "SELECT generation,signal,source_event_id,action,state FROM main.gogoke_v37_seat_health WHERE domain_id=?1 AND event_id=?2")?;
        q.bind_text(1,caller.domain_id())?;q.bind_text(2,event_id)?;
        if q.step_row()? {
            let generation=q.column_text(0)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
            if generation!=seat.generation||q.column_text(1)?!=signal.sql()||
                q.column_text(2)?!=source_event_id {return Err(SeatError::Conflict);}
            let action=q.column_text(3)?;let state=q.column_text(4)?;
            if q.step_row()? {return Err(SeatError::SchemaDrift);}
            return Ok(HealthObservation {event_id:event_id.into(),action,state,generation});
        }
        let write=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_health(domain_id,seat_id,event_id,generation,signal,source_event_id,action,state) VALUES(?1,?2,?3,?4,?5,?6,?7,'OBSERVED')")?;
        write.bind_text(1,caller.domain_id())?;write.bind_text(2,caller.seat_id())?;
        write.bind_text(3,event_id)?;write.bind_i64(4,seat.generation)?;
        write.bind_text(5,signal.sql())?;write.bind_text(6,source_event_id)?;
        write.bind_text(7,signal.action())?;write.step_done()?;
        Ok(HealthObservation {event_id:event_id.into(),action:signal.action().into(),
            state:"OBSERVED".into(),generation:seat.generation})
    })
}

pub(super) fn require_stalled_health(db:&VerifiedDatabaseConnection<'_>,domain:&str,
    seat:&str,event_id:&str)->Result<(),SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT signal,action,state FROM main.gogoke_v37_seat_health WHERE domain_id=?1 AND seat_id=?2 AND event_id=?3")?;
    q.bind_text(1,domain)?;q.bind_text(2,seat)?;q.bind_text(3,event_id)?;
    if !q.step_row()?||q.column_text(0)?!="STALLED"||q.column_text(1)?!="ESCALATE"||
        q.column_text(2)?!="OBSERVED"||q.step_row()? {return Err(SeatError::Denied);}
    Ok(())
}

pub(crate) fn mark_health_requested(db:&mut VerifiedDatabaseConnection<'_>,
    caller:&policy::NativeSeatCall,event_id:&str,session_request_id:&str)->Result<bool,SeatError> {
    if !valid_id(event_id)||!valid_id(session_request_id) {return Err(SeatError::Invalid("health request"));}
    transact(db,|db| {
        let seat=policy::current_caller(db,caller)?;
        let prior=Statement::prepare(db.as_ptr(),
            "SELECT generation,action,state,COALESCE(session_request_id,'') FROM main.gogoke_v37_seat_health WHERE domain_id=?1 AND seat_id=?2 AND event_id=?3")?;
        prior.bind_text(1,caller.domain_id())?;prior.bind_text(2,caller.seat_id())?;
        prior.bind_text(3,event_id)?;
        if !prior.step_row()? {return Err(SeatError::Denied);}
        let generation=prior.column_text(0)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
        let action=prior.column_text(1)?;let state=prior.column_text(2)?;
        let prior_request=prior.column_text(3)?;
        if prior.step_row()?||generation!=seat.generation||!matches!(action.as_str(),"COMPACT"|"RENEW") {
            return Err(SeatError::Denied);
        }
        if state=="REQUESTED" && prior_request==session_request_id {return Ok(false);}
        if state!="OBSERVED" || !prior_request.is_empty() {return Err(SeatError::Unknown);}
        let q=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_health SET state='REQUESTED',session_request_id=?1 WHERE domain_id=?2 AND seat_id=?3 AND event_id=?4 AND generation=?5 AND action IN ('COMPACT','RENEW') AND state='OBSERVED'")?;
        q.bind_text(1,session_request_id)?;q.bind_text(2,caller.domain_id())?;
        q.bind_text(3,caller.seat_id())?;q.bind_text(4,event_id)?;
        q.bind_i64(5,seat.generation)?;q.step_done()?;
        let verify=Statement::prepare(db.as_ptr(),
            "SELECT state,session_request_id FROM main.gogoke_v37_seat_health WHERE domain_id=?1 AND event_id=?2")?;
        verify.bind_text(1,caller.domain_id())?;verify.bind_text(2,event_id)?;
        if !verify.step_row()?||verify.column_text(0)?!="REQUESTED"||
            verify.column_text(1)?!=session_request_id||verify.step_row()? {return Err(SeatError::Conflict);}
        Ok(true)
    })
}

/// Root/H must verify this receipt against the original K-SESSION request.
pub(crate) fn settle_health_receipt(db:&mut VerifiedDatabaseConnection<'_>,domain:&str,
    event_id:&str,session_request_id:&str,receipt_id:&str)->Result<(),SeatError> {
    if [domain,event_id,session_request_id,receipt_id].iter().any(|value|!valid_id(value)) {
        return Err(SeatError::Invalid("health receipt"));
    }
    transact(db,|db| {
        let q=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_health SET state='RECEIPTED',receipt_id=?1 WHERE domain_id=?2 AND event_id=?3 AND session_request_id=?4 AND state='REQUESTED'")?;
        q.bind_text(1,receipt_id)?;q.bind_text(2,domain)?;q.bind_text(3,event_id)?;
        q.bind_text(4,session_request_id)?;q.step_done()?;
        let verify=Statement::prepare(db.as_ptr(),
            "SELECT state,receipt_id FROM main.gogoke_v37_seat_health WHERE domain_id=?1 AND event_id=?2")?;
        verify.bind_text(1,domain)?;verify.bind_text(2,event_id)?;
        if !verify.step_row()?||verify.column_text(0)?!="RECEIPTED"||
            verify.column_text(1)?!=receipt_id||verify.step_row()? {return Err(SeatError::Conflict);}
        Ok(())
    })
}
