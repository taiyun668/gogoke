//! Owner's E configuration fact for the sole global secretary seat.
//! This designation is not an A session purpose, an H admission proof, or a
//! grant to read global/project ledgers, dispatch work, or run a model.

use super::*;

pub(super) const DESIGNATION: &str = "CREATE TABLE gogoke_v37_seat_secretary(singleton INTEGER PRIMARY KEY CHECK(singleton=1),domain_id TEXT NOT NULL CHECK(domain_id='global'),seat_id TEXT NOT NULL,incarnation TEXT NOT NULL,request_id TEXT NOT NULL,fingerprint TEXT NOT NULL,FOREIGN KEY(domain_id,seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT";

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct SecretaryDesignation {
    pub(crate) seat_id:String,
    pub(crate) incarnation:String,
    pub(crate) replayed:bool,
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) enum SecretaryConfiguration {
    Unset,
    Revoked,
    /// Four optional fields are the original E settings, never inferred from
    /// an instance pin or vendor default. Root must separately obtain H/A
    /// purpose and binding before it can claim this seat is runnable.
    Designated {
        seat_id:String, incarnation:String, generation:i64, revision:i64,
        instance_id:Option<String>, model:Option<String>, effort:Option<String>,
        permission:Option<PermissionTier>, state:State,
    },
}

pub(super) fn designation(db:&VerifiedDatabaseConnection<'_>)
    ->Result<Option<(String,String,String,String)>,SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT seat_id,incarnation,request_id,fingerprint FROM main.gogoke_v37_seat_secretary WHERE singleton=1")?;
    if !q.step_row()? {return Ok(None);}
    let result=(q.column_text(0)?,q.column_text(1)?,q.column_text(2)?,q.column_text(3)?);
    if q.step_row()? {return Err(SeatError::SchemaDrift);}
    Ok(Some(result))
}

pub(super) fn is_designated_current(db:&VerifiedDatabaseConnection<'_>,seat:&Seat)
    ->Result<bool,SeatError> {
    Ok(seat.domain_id=="global" && designation(db)?
        .is_some_and(|row|row.0==seat.seat_id && row.1==seat.incarnation))
}

pub(super) fn require_enabled_instance(db:&VerifiedDatabaseConnection<'_>,instance_id:&str)
    ->Result<(),SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_instances i JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id WHERE i.instance_id=?1 AND i.install_state='INSTALLED' AND i.login_state='LOGGED_IN' AND p.enabled=1 AND p.tombstoned=0")?;
    q.bind_text(1,instance_id)?;
    if !q.step_row()? {return Err(SeatError::Denied);}
    if q.step_row()? {return Err(SeatError::SchemaDrift);}
    Ok(())
}

/// Designate an already created E USER/LONG global seat. Its identity is the
/// persisted singleton row, not a caller supplied `secretary` role flag or a
/// guessed seat name. The row cannot be replaced by a later request.
pub(crate) fn designate_secretary(db:&mut VerifiedDatabaseConnection<'_>,
    issuer:&OwnerIssuer,seat_id:&str,incarnation:&str,request_id:&str,
    original_raw:&[u8])->Result<SecretaryDesignation,SeatError> {
    validate("global",seat_id,request_id,original_raw)?;
    if !valid_id(incarnation) {return Err(SeatError::Invalid("incarnation"));}
    let fp=fingerprint(&["designate-secretary","global",seat_id,incarnation],original_raw);
    transact(db,|db| {
        check_current_owner(db,issuer)?;
        if let Some((old_seat,old_inc,old_request,old_fp))=designation(db)? {
            if old_request!=request_id||old_fp!=fp||old_seat!=seat_id||old_inc!=incarnation {
                return Err(SeatError::Conflict);
            }
            let current=read(db,"global",seat_id)?.ok_or(SeatError::SchemaDrift)?;
            if current.incarnation!=incarnation||current.state==State::Reclaimed {
                return Err(SeatError::Denied);
            }
            return Ok(SecretaryDesignation {seat_id:seat_id.into(),
                incarnation:incarnation.into(),replayed:true});
        }
        let seat=read(db,"global",seat_id)?.ok_or(SeatError::Denied)?;
        if seat.incarnation!=incarnation||seat.layer!=Layer::User||seat.kind!=Kind::Long||
            seat.parent_seat_id.is_some()||seat.state==State::Reclaimed {
            return Err(SeatError::Denied);
        }
        if seat.state==State::Busy {return Err(SeatError::Busy);}
        page_facts::ensure_mutable(db,&seat)?;
        // A pre-existing project-style template may contain arbitrary model
        // intent. Start from an actually unconfigured seat; only the dedicated
        // F-checked write below may establish these four selections.
        if !seat.instance_id.is_empty() {return Err(SeatError::Denied);}
        let settings=seat.settings_json.as_deref().ok_or(SeatError::SchemaDrift)?;
        let Json::Object(fields)=Parser::parse(settings)? else {return Err(SeatError::SchemaDrift);};
        for key in ["model","effort","reasoningEffort","permissionTier"] {
            if fields.contains_key(&JsonString::from_str(key)) {return Err(SeatError::Denied);}
        }
        let q=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_secretary(singleton,domain_id,seat_id,incarnation,request_id,fingerprint) VALUES(1,'global',?1,?2,?3,?4)")?;
        q.bind_text(1,seat_id)?;q.bind_text(2,incarnation)?;
        q.bind_text(3,request_id)?;q.bind_text(4,&fp)?;q.step_done()?;
        Ok(SecretaryDesignation {seat_id:seat_id.into(),incarnation:incarnation.into(),replayed:false})
    })
}

/// Root calls this inside its authenticated Owner read transaction. This is
/// an E settings snapshot only; it does not make the secretary operational.
pub(crate) fn read_secretary_configuration_in_transaction(
    db:&VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer)
    ->Result<SecretaryConfiguration,SeatError> {
    check_current_owner(db,issuer)?;
    let Some((seat_id,incarnation,_,_))=designation(db)? else {
        return Ok(SecretaryConfiguration::Unset);
    };
    let seat=read(db,"global",&seat_id)?.ok_or(SeatError::SchemaDrift)?;
    if seat.incarnation!=incarnation||seat.layer!=Layer::User||seat.kind!=Kind::Long {
        return Err(SeatError::SchemaDrift);
    }
    if seat.state==State::Reclaimed {return Ok(SecretaryConfiguration::Revoked);}
    let settings=seat.settings_json.as_deref().ok_or(SeatError::SchemaDrift)?;
    let Json::Object(fields)=Parser::parse(settings)? else {return Err(SeatError::SchemaDrift);};
    let optional_string=|name:&str|->Result<Option<String>,SeatError> {
        match fields.get(&JsonString::from_str(name)) {
            None=>Ok(None),
            Some(Json::String(value))=>value.to_well_formed_string()
                .filter(|value|!value.is_empty()).map(Some).ok_or(SeatError::SchemaDrift),
            _=>Err(SeatError::SchemaDrift),
        }
    };
    let permission=fields.get(&JsonString::from_str("permissionTier"))
        .map(PermissionTier::from_json).transpose()?;
    Ok(SecretaryConfiguration::Designated {seat_id,incarnation,
        generation:seat.generation,revision:seat.revision,
        instance_id:if seat.instance_id.is_empty(){None}else{Some(seat.instance_id)},
        model:optional_string("model")?,effort:optional_string("effort")?,
        permission,state:seat.state})
}

/// H's current USER admission fact. The singleton and its incarnation are
/// re-read at every launch verification; a display name or wire flag has no
/// authority. All four selections must be present before a model can start.
pub(crate) fn require_secretary_session(db:&VerifiedDatabaseConnection<'_>,
    issuer:&OwnerIssuer,seat_id:&str,incarnation:&str)->Result<Seat,SeatError> {
    let SecretaryConfiguration::Designated {seat_id:current,incarnation:current_inc,
        instance_id:Some(instance),model:Some(_),effort:Some(_),permission:Some(_),
        state,..}=read_secretary_configuration_in_transaction(db,issuer)? else {
        return Err(SeatError::Denied);
    };
    if current!=seat_id || current_inc!=incarnation ||
        !matches!(state,State::Idle|State::Busy) {return Err(SeatError::Denied);}
    require_enabled_instance(db,&instance)?;
    let seat=read(db,"global",seat_id)?.ok_or(SeatError::SchemaDrift)?;
    if seat.incarnation!=incarnation || seat.instance_id!=instance {
        return Err(SeatError::Denied);
    }
    Ok(seat)
}

/// One atomic change of the four selections. Existing E operation receipts
/// supply original-byte deduplication, CAS, busy/unreleased refusal, and a
/// current-target replay check. F's source-bound model evidence is rechecked
/// by configure_instance inside that same transaction.
pub(crate) fn configure_secretary(db:&mut VerifiedDatabaseConnection<'_>,
    issuer:&OwnerIssuer,expected_generation:i64,expected_revision:i64,
    request_id:&str,original_raw:&[u8],instance_id:&str,model:&str,
    effort:&str,permission_json:&str)->Result<SeatReceipt,SeatError> {
    let (seat_id,incarnation,_,_)=transact(db,|db| {
        check_current_owner(db,issuer)?;
        designation(db)?.ok_or(SeatError::Unknown)
    })?;
    let seat=read(db,"global",&seat_id)?.ok_or(SeatError::SchemaDrift)?;
    if seat.incarnation!=incarnation||seat.layer!=Layer::User||seat.kind!=Kind::Long {
        return Err(SeatError::SchemaDrift);
    }
    configure_instance(db,NativeOrigin::User(issuer),SeatChange {
        domain_id:"global",seat_id:&seat_id,expected_generation,expected_revision,
        request_id,request_bytes:original_raw,
    },instance_id,model,effort,permission_json)
}
