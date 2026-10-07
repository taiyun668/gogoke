//! Host-backed seat-page facts. Presentation metadata never changes seat identity.
use super::*;

const NAMES: &str = "CREATE TABLE gogoke_v37_seat_display_names(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL,display_name TEXT NOT NULL CHECK(length(display_name)>0),PRIMARY KEY(domain_id,seat_id,incarnation),FOREIGN KEY(domain_id,seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT";
const LEAD: &str = "CREATE TABLE gogoke_v37_seat_project_lead(domain_id TEXT PRIMARY KEY,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL,FOREIGN KEY(domain_id,seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT";
pub(super) const SCHEMA: [(&str,&str);2] = [
    ("gogoke_v37_seat_display_names",NAMES),("gogoke_v37_seat_project_lead",LEAD),
];
pub(super) fn create_tables(db:&mut VerifiedDatabaseConnection<'_>)->Result<(),SeatError> {
    db.execute(NAMES)?; db.execute(LEAD)?; Ok(())
}

/// Root supplies only an issuer minted at the native Owner boundary.
pub(crate) fn rename_seat(db:&mut VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer,
    domain:&str,seat_id:&str,incarnation:&str,display_name:&str)->Result<(),SeatError> {
    if !valid_id(domain)||!valid_id(seat_id)||!valid_id(incarnation)||
        display_name.trim()!=display_name||display_name.is_empty()||
        display_name.chars().count()>80||display_name.chars().any(char::is_control) {
        return Err(SeatError::Invalid("display_name"));
    }
    transact(db,|db| {
        check_current_owner(db,issuer)?;
        let seat=read(db,domain,seat_id)?.ok_or(SeatError::Denied)?;
        if seat.incarnation!=incarnation||seat.state==State::Reclaimed {return Err(SeatError::Denied);}
        let q=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_display_names(domain_id,seat_id,incarnation,display_name) VALUES(?1,?2,?3,?4) ON CONFLICT(domain_id,seat_id,incarnation) DO UPDATE SET display_name=excluded.display_name")?;
        q.bind_text(1,domain)?;q.bind_text(2,seat_id)?;q.bind_text(3,incarnation)?;
        q.bind_text(4,display_name)?;q.step_done()?;Ok(())
    })
}

/// There is no inferred lead. The exact live USER-layer incarnation is designated by Owner.
pub(crate) fn designate_project_lead(db:&mut VerifiedDatabaseConnection<'_>,
    issuer:&OwnerIssuer,domain:&str,seat_id:&str,incarnation:&str)->Result<(),SeatError> {
    if !valid_id(domain)||!valid_id(seat_id)||!valid_id(incarnation) {
        return Err(SeatError::Invalid("lead address"));
    }
    transact(db,|db| {
        check_current_owner(db,issuer)?;
        let seat=read(db,domain,seat_id)?.ok_or(SeatError::Denied)?;
        if seat.incarnation!=incarnation||seat.layer!=Layer::User||
            seat.state==State::Reclaimed {return Err(SeatError::Denied);}
        let q=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_project_lead(domain_id,seat_id,incarnation) VALUES(?1,?2,?3) ON CONFLICT(domain_id) DO UPDATE SET seat_id=excluded.seat_id,incarnation=excluded.incarnation")?;
        q.bind_text(1,domain)?;q.bind_text(2,seat_id)?;q.bind_text(3,incarnation)?;
        q.step_done()?;Ok(())
    })
}

#[derive(Debug)]
pub(crate) struct SeatActionFacts {
    pub(crate) tune:bool,pub(crate) change_instance:bool,pub(crate) remove:bool,
    /// Native precondition preventing an action; rechecked in the real mutation.
    pub(crate) locked_reason:Option<&'static str>,
}
#[derive(Debug)]
pub(crate) struct PageSeatFacts {
    pub(crate) seat:Seat,
    pub(crate) display_name:Option<String>,
    pub(crate) is_project_lead:bool,
    pub(crate) state_card:Option<StateCard>,
    pub(crate) orchestration_scope:Option<OrchestrationScope>,
    pub(crate) allowed:SeatActionFacts,
}
#[derive(Debug)]
pub(crate) struct SeatPageFacts {
    pub(crate) seats:Vec<PageSeatFacts>,
    pub(crate) lead_designated:bool,
}
#[derive(Debug)]
pub(crate) struct TemplateChoice {
    pub(crate) template_id:String,pub(crate) settings_json:String,pub(crate) revision:i64,
}

pub(crate) fn list_templates(db:&VerifiedDatabaseConnection<'_>,domain:&str)
    ->Result<Vec<TemplateChoice>,SeatError> {
    if !valid_id(domain) {return Err(SeatError::Invalid("domain_id"));}
    let q=Statement::prepare(db.as_ptr(),
        "SELECT template_id,settings_json,revision FROM main.gogoke_v37_seat_templates WHERE domain_id=?1 ORDER BY template_id")?;
    q.bind_text(1,domain)?;let mut out=Vec::new();
    while q.step_row()? {out.push(TemplateChoice {template_id:q.column_text(0)?,
        settings_json:q.column_text(1)?,revision:q.column_text(2)?.parse()
            .map_err(|_|SeatError::SchemaDrift)?});}
    Ok(out)
}

fn one_text(db:&VerifiedDatabaseConnection<'_>,sql:&str,domain:&str,seat:&str,inc:&str)
    ->Result<Option<String>,SeatError> {
    let q=Statement::prepare(db.as_ptr(),sql)?;
    q.bind_text(1,domain)?;q.bind_text(2,seat)?;q.bind_text(3,inc)?;
    if !q.step_row()? {return Ok(None);}
    let value=q.column_text(0)?;
    if q.step_row()? {return Err(SeatError::SchemaDrift);}
    Ok(Some(value))
}
fn exists(db:&VerifiedDatabaseConnection<'_>,sql:&str,domain:&str,seat:&str,inc:&str)
    ->Result<bool,SeatError> {Ok(one_text(db,sql,domain,seat,inc)?.is_some())}
fn has_table(db:&VerifiedDatabaseConnection<'_>,name:&str)->Result<bool,SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.sqlite_schema WHERE type='table' AND name=?1")?;
    q.bind_text(1,name)?;Ok(q.step_row()?)
}
pub(super) fn is_designated_lead(db:&VerifiedDatabaseConnection<'_>,seat:&Seat)
    ->Result<bool,SeatError> {
    exists(db,
        "SELECT incarnation FROM main.gogoke_v37_seat_project_lead WHERE domain_id=?1 AND seat_id=?2 AND incarnation=?3",
        &seat.domain_id,&seat.seat_id,&seat.incarnation)
}

/// An IDLE row is insufficient when H has a live or unresolved claim/episode.
fn unresolved_h(db:&VerifiedDatabaseConnection<'_>,seat:&Seat)->Result<bool,SeatError> {
    if !has_table(db,"gogoke_v37_h_claim")?||
        !has_table(db,"gogoke_v37_h_seat_binding")? {return Ok(false);}
    exists(db,
        "SELECT h.state FROM main.gogoke_v37_h_claim h JOIN main.gogoke_v37_h_seat_binding b ON b.domain_id=h.domain_id AND b.session_id=h.session_id AND b.generation=h.generation WHERE b.domain_id=?1 AND b.seat_id=?2 AND b.seat_incarnation=?3 AND h.state IN ('RESERVED','COMMITTED','UNKNOWN') LIMIT 1",
        &seat.domain_id,&seat.seat_id,&seat.incarnation)
}
fn unresolved_episode(db:&VerifiedDatabaseConnection<'_>,seat:&Seat)->Result<bool,SeatError> {
    if !has_table(db,"gogoke_v37_h_process_episode")? {return Ok(false);}
    exists(db,
        "SELECT phase FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1 AND seat_id=?2 AND seat_incarnation=?3 AND phase IN ('INTENT','PREPARED','ACTIVE','UNKNOWN') LIMIT 1",
        &seat.domain_id,&seat.seat_id,&seat.incarnation)
}
fn unresolved_instance(db:&VerifiedDatabaseConnection<'_>,seat:&Seat)->Result<bool,SeatError> {
    if seat.instance_id.is_empty() {return Ok(false);}
    if !has_table(db,"gogoke_v37_instance_operations")? {return Ok(false);}
    let q=Statement::prepare(db.as_ptr(),
        "SELECT phase FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase IN ('PREPARING','UNKNOWN') LIMIT 1")?;
    q.bind_text(1,&seat.instance_id)?;Ok(q.step_row()?)
}
pub(super) fn ensure_mutable(db:&VerifiedDatabaseConnection<'_>,seat:&Seat)
    ->Result<(),SeatError> {
    if unresolved_h(db,seat)?||unresolved_episode(db,seat)?||
        unresolved_instance(db,seat)? {return Err(SeatError::Busy);}
    Ok(())
}
fn actions(db:&VerifiedDatabaseConnection<'_>,seat:&Seat,lead:bool)
    ->Result<SeatActionFacts,SeatError> {
    let reason=if seat.state==State::Reclaimed {Some("RECLAIMED")}
        else if seat.state==State::Busy {Some("BUSY_REQUIRES_H_STOP_READBACK")}
        else if !has_table(db,"gogoke_v37_h_claim")?||
            !has_table(db,"gogoke_v37_h_seat_binding")?||
            !has_table(db,"gogoke_v37_h_process_episode")? {
            Some("H_FACTS_UNAVAILABLE")
        }
        else if unresolved_h(db,seat)?||unresolved_episode(db,seat)? {
            Some("H_PENDING_OR_UNRESOLVED")
        } else if !has_table(db,"gogoke_v37_instance_operations")? {
            Some("INSTANCE_FACTS_UNAVAILABLE")
        } else if unresolved_instance(db,seat)? {Some("INSTANCE_MUTATION_PENDING")}
        else if seat.settings_json.is_none() {Some("SETTINGS_UNAVAILABLE")}
        else {None};
    let idle=reason.is_none();
    Ok(SeatActionFacts {tune:idle,change_instance:idle&&!seat.instance_id.is_empty(),
        remove:idle&&!lead,locked_reason:if lead&&idle {Some("PROJECT_LEAD")} else {reason}})
}

/// Consistent page snapshot should be called inside Root's read transaction.
/// Busy removal remains disabled: only Root's real H stop/readback may later enable it.
pub(crate) fn list_page_facts(db:&VerifiedDatabaseConnection<'_>,domain:&str)
    ->Result<SeatPageFacts,SeatError> {
    if !valid_id(domain) {return Err(SeatError::Invalid("domain_id"));}
    let leadq=Statement::prepare(db.as_ptr(),
        "SELECT seat_id,incarnation FROM main.gogoke_v37_seat_project_lead WHERE domain_id=?1")?;
    leadq.bind_text(1,domain)?;
    let designation=if leadq.step_row()? {Some((leadq.column_text(0)?,leadq.column_text(1)?))}
        else {None};
    if leadq.step_row()? {return Err(SeatError::SchemaDrift);}
    let q=Statement::prepare(db.as_ptr(),
        "SELECT seat_id FROM main.gogoke_v37_seats WHERE domain_id=?1 ORDER BY seat_id")?;
    q.bind_text(1,domain)?;
    let mut seats=Vec::new();let mut lead_designated=false;
    while q.step_row()? {
        let seat_id=q.column_text(0)?;
        let seat=read(db,domain,&seat_id)?.ok_or(SeatError::SchemaDrift)?;
        let lead=designation.as_ref().is_some_and(|(id,inc)|
            id==&seat.seat_id&&inc==&seat.incarnation&&
            seat.layer==Layer::User&&seat.state!=State::Reclaimed);
        lead_designated|=lead;
        let display_name=one_text(db,
            "SELECT display_name FROM main.gogoke_v37_seat_display_names WHERE domain_id=?1 AND seat_id=?2 AND incarnation=?3",
            domain,&seat.seat_id,&seat.incarnation)?;
        let state_card=if seat.state==State::Reclaimed {None}
            else {Some(continuity::read_state_card(db,&seat)?)};
        let orchestration_scope=if lead {match orchestration::orchestration_scope(&seat) {
            Ok(scope)=>Some(scope),Err(SeatError::Denied)=>None,Err(error)=>return Err(error),
        }} else {None};
        let allowed=actions(db,&seat,lead)?;
        seats.push(PageSeatFacts {seat,display_name,is_project_lead:lead,
            state_card,orchestration_scope,allowed});
    }
    Ok(SeatPageFacts {seats,lead_designated})
}
