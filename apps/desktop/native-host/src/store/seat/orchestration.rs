//! E.2 lead scope and copied-template rendering. A template is a source of
//! instructions, never a filesystem or process permission grant.

use super::*;
use std::collections::BTreeSet;

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct OrchestrationScope {
    pub(crate) instance_ids:BTreeSet<String>,
    pub(crate) models:BTreeSet<String>,
    pub(crate) reasoning_efforts:BTreeSet<String>,
    pub(crate) max_permission_tier:PermissionTier,
    /// None is an old four-field record, never an inferred Owner bound.
    pub(crate) max_concurrent:Option<i64>,
}

fn string_field(fields:&std::collections::BTreeMap<JsonString,Json>,key:&str)
    ->Result<String,SeatError> {
    let Some(Json::String(value))=fields.get(&JsonString::from_str(key)) else {
        return Err(SeatError::Denied);
    };
    let value=value.to_well_formed_string().ok_or(SeatError::Denied)?;
    if value.is_empty()||value.len()>256 {return Err(SeatError::Denied);}
    Ok(value)
}

fn effort_field(fields:&std::collections::BTreeMap<JsonString,Json>)
    ->Result<Option<String>,SeatError> {
    let canonical=fields.get(&JsonString::from_str("effort"));
    let legacy=fields.get(&JsonString::from_str("reasoningEffort"));
    let current=canonical.map(|_|string_field(fields,"effort")).transpose()?;
    let old=legacy.map(|_|string_field(fields,"reasoningEffort")).transpose()?;
    if current.is_some()&&old.is_some()&&current!=old {return Err(SeatError::Denied);}
    Ok(current.or(old))
}

/// E.2's historical reasoningEffort input is accepted only when unambiguous.
/// The copied template and all later host use carry M1's actual `effort` key.
pub(super) fn normalized_effort_json(raw:&str)->Result<String,SeatError> {
    let Json::Object(mut fields)=Parser::parse(raw)? else {return Err(SeatError::Denied);};
    let effort=effort_field(&fields)?;
    fields.remove(&JsonString::from_str("reasoningEffort"));
    if let Some(value)=effort {
        fields.insert(JsonString::from_str("effort"),Json::String(JsonString::from_str(&value)));
    }
    Ok(Json::Object(fields).canonical())
}

pub(crate) fn seat_effort(seat:&Seat)->Result<String,SeatError> {
    let settings=seat.settings_json.as_deref().ok_or(SeatError::Denied)?;
    let Json::Object(fields)=Parser::parse(settings)? else {return Err(SeatError::SchemaDrift);};
    effort_field(&fields)?.ok_or(SeatError::Denied)
}

fn names(fields:&std::collections::BTreeMap<JsonString,Json>,key:&str)
    ->Result<BTreeSet<String>,SeatError> {
    let Some(Json::Array(items))=fields.get(&JsonString::from_str(key)) else {
        return Err(SeatError::Denied);
    };
    if items.is_empty()||items.len()>64 {return Err(SeatError::Denied);}
    let mut names=BTreeSet::new();
    for item in items {
        let Json::String(value)=item else {return Err(SeatError::Denied);};
        let value=value.to_well_formed_string().ok_or(SeatError::Denied)?;
        if value.is_empty()||value.len()>256||!names.insert(value) {return Err(SeatError::Denied);}
    }
    Ok(names)
}

fn tier_rank(tier:PermissionTier)->u8 {match tier {PermissionTier::ReadOnly=>0,
    PermissionTier::NoNetwork=>1,PermissionTier::IsolatedWrite=>2,
    PermissionTier::NetworkedWrite=>3}}

pub(super) fn validate_template_scope(settings:&Json)->Result<(),SeatError> {
    let Json::Object(fields)=settings else {return Err(SeatError::Invalid("template settings"));};
    effort_field(fields).map_err(|_|SeatError::Invalid("effort"))?;
    let Some(scope)=fields.get(&JsonString::from_str("orchestrationScope")) else {return Ok(());};
    parse_scope(scope).map(|_|()).map_err(|_|SeatError::Invalid("orchestrationScope"))
}

fn parse_scope(value:&Json)->Result<OrchestrationScope,SeatError> {
    let Json::Object(fields)=value else {return Err(SeatError::Denied);};
    if fields.len()!=4 && fields.len()!=5 {return Err(SeatError::Denied);}
    let instances=names(fields,"instanceIds")?;
    if instances.iter().any(|value|!valid_id(value)) {return Err(SeatError::Denied);}
    let models=names(fields,"models")?;
    let efforts=names(fields,"reasoningEfforts")?;
    let tier=fields.get(&JsonString::from_str("maxPermissionTier"))
        .ok_or(SeatError::Denied).and_then(PermissionTier::from_json)?;
    let max_concurrent=match fields.get(&JsonString::from_str("maxConcurrent")) {
        Some(Json::Number(value))=>Some(value.parse::<i64>().ok()
            .filter(|value|*value>0).ok_or(SeatError::Denied)?),
        None if fields.len()==4=>None,
        _=>return Err(SeatError::Denied),
    };
    Ok(OrchestrationScope {instance_ids:instances,models,
        reasoning_efforts:efforts,max_permission_tier:tier,max_concurrent})
}

/// A new User-set bound must name its own seat cap. The project-wide Owner
/// cap is a different fact and must never be copied into this field.
pub(crate) fn validate_new_scope(value:&Json)->Result<OrchestrationScope,SeatError> {
    let scope=parse_scope(value)?;
    if scope.max_concurrent.is_none() {return Err(SeatError::Denied);}
    Ok(scope)
}

pub(crate) fn orchestration_scope(seat:&Seat)->Result<OrchestrationScope,SeatError> {
    if seat.layer!=Layer::User||seat.state==State::Reclaimed {return Err(SeatError::Denied);}
    let settings=seat.settings_json.as_deref().ok_or(SeatError::Denied)?;
    let Json::Object(fields)=Parser::parse(settings)? else {return Err(SeatError::SchemaDrift);};
    let scope=fields.get(&JsonString::from_str("orchestrationScope")).ok_or(SeatError::Denied)?;
    parse_scope(scope)
}

pub(super) fn child_within_scope(parent:&Seat,child_settings:&str,
    child_instance_id:&str)->Result<(),SeatError> {
    let scope=orchestration_scope(parent)?;
    if scope.max_concurrent.is_none() {return Err(SeatError::Denied);}
    if !scope.instance_ids.contains(child_instance_id) {return Err(SeatError::Denied);}
    let Json::Object(fields)=Parser::parse(child_settings)? else {return Err(SeatError::Denied);};
    let model=string_field(&fields,"model")?;
    let effort=effort_field(&fields)?.ok_or(SeatError::Denied)?;
    let tier=fields.get(&JsonString::from_str("permissionTier"))
        .ok_or(SeatError::Denied).and_then(PermissionTier::from_json)?;
    if !scope.models.contains(&model)||!scope.reasoning_efforts.contains(&effort)||
        tier_rank(tier)>tier_rank(scope.max_permission_tier) {return Err(SeatError::Denied);}
    Ok(())
}

/// Root/H invokes this after live lead authentication and takeover readiness,
/// before a child reservation or any new work is admitted.
pub(crate) fn authorize_child_dispatch(db:&VerifiedDatabaseConnection<'_>,
    caller:&policy::NativeSeatCall,child:&Seat)->Result<(),SeatError> {
    let current=current_child_dispatch_context(db,caller,child)?;
    if current.state!=State::Idle {return Err(SeatError::Denied);}
    Ok(())
}

/// Root/H can reuse this common check only inside a separate exact matching
/// reservation check. BUSY alone never authorizes an open or commit.
pub(crate) fn current_child_dispatch_context(db:&VerifiedDatabaseConnection<'_>,
    caller:&policy::NativeSeatCall,child:&Seat)->Result<Seat,SeatError> {
    let parent=policy::current_caller(db,caller)?;
    if parent.layer!=Layer::User||child.layer!=Layer::Lead||
        child.domain_id!=parent.domain_id||child.parent_seat_id.as_deref()!=Some(&parent.seat_id)||
        child.state==State::Reclaimed||!continuity::takeover_ready(db,&parent)? {
        return Err(SeatError::Denied);
    }
    let current=read(db,&child.domain_id,&child.seat_id)?.ok_or(SeatError::Denied)?;
    if current!=*child {return Err(SeatError::Conflict);}
    policy::authorize_current_call(db,caller,&child.domain_id,&child.seat_id,
        policy::CallAction::Dispatch)?;
    child_within_scope(&parent,child.settings_json.as_deref().ok_or(SeatError::Denied)?,
        &child.instance_id)?;
    Ok(current)
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct RenderedInstruction {
    pub(crate) file_name:&'static str,
    pub(crate) contents:String,
    pub(crate) template_id:String,
}

/// The caller must prove a collision-free, project-scoped discovery location
/// before writing these bytes. This renderer does not edit the repository or
/// assert that the running vendor loaded them.
pub(crate) fn render_codex_instruction(seat:&Seat)->Result<RenderedInstruction,SeatError> {
    let _effort=seat_effort(seat)?;
    let settings=seat.settings_json.as_deref().ok_or(SeatError::Denied)?;
    let Json::Object(fields)=Parser::parse(settings)? else {return Err(SeatError::SchemaDrift);};
    let instruction=string_field(&fields,"instruction")?;
    if instruction.len()>65_536||instruction.contains('\0') {return Err(SeatError::Denied);}
    let template_id=seat.template_id.as_deref().ok_or(SeatError::Denied)?;
    if !valid_id(template_id) {return Err(SeatError::Denied);}
    Ok(RenderedInstruction {file_name:"AGENTS.md",contents:format!("{instruction}\n"),
        template_id:template_id.into()})
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD:&str=r#"{"instanceIds":["instanceA"],"models":["modelA"],"reasoningEfforts":["high"],"maxPermissionTier":"READ_ONLY"}"#;

    fn scope(raw:&str)->Result<OrchestrationScope,SeatError> {
        parse_scope(&Parser::parse(raw).unwrap())
    }

    #[test]
    fn old_scope_remains_readable_but_is_not_a_new_user_bound() {
        let old=scope(OLD).unwrap();
        assert_eq!(old.max_concurrent,None);
        assert!(matches!(validate_new_scope(&Parser::parse(OLD).unwrap()),
            Err(SeatError::Denied)));
    }

    #[test]
    fn new_scope_requires_exact_positive_i64_concurrency_fact() {
        let with=|value:&str| OLD.replace("\"maxPermissionTier\"",
            &format!("\"maxConcurrent\":{value},\"maxPermissionTier\""));
        let valid=Parser::parse(&with("3")).unwrap();
        assert_eq!(validate_new_scope(&valid).unwrap().max_concurrent,Some(3));
        for value in ["0","-1","1.5","\"3\"","null","9223372036854775808"] {
            assert!(matches!(scope(&with(value)),Err(SeatError::Denied)),
                "invalid maxConcurrent accepted: {value}");
        }
        assert!(matches!(scope(&OLD.replace("\"maxPermissionTier\"",
            "\"unexpected\":1,\"maxPermissionTier\"")),Err(SeatError::Denied)));
    }

    #[test]
    fn old_scope_cannot_authorize_a_model_child() {
        let mut parent=Seat {domain_id:"projectA".into(),seat_id:"lead".into(),
            incarnation:"incarnationA".into(),layer:Layer::User,parent_seat_id:None,
            kind:Kind::Long,instance_id:"instanceA".into(),template_id:None,
            settings_json:Some(format!(r#"{{"orchestrationScope":{OLD}}}"#)),
            state:State::Idle,generation:1,revision:1};
        let child=r#"{"model":"modelA","effort":"high","permissionTier":"READ_ONLY"}"#;
        assert!(matches!(child_within_scope(&parent,child,"instanceA"),Err(SeatError::Denied)));
        parent.settings_json=Some(format!(r#"{{"orchestrationScope":{}}}"#,
            OLD.replace("\"maxPermissionTier\"","\"maxConcurrent\":2,\"maxPermissionTier\"")));
        assert!(child_within_scope(&parent,child,"instanceA").is_ok());
    }
}
