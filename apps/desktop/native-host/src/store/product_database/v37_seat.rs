//! User-side E.1/F.1 ingress on the already verified product database.
//! The parent must verify UserOriginProof before invoking either method.
use super::*;
use crate::store::atomic::Parser;
use crate::store::seat::{self, CreateSeat, Kind, NativeOrigin, Seat, SeatChange, SeatError, SeatReceipt, State, StoreTemplate};

fn key(name: &str) -> JsonString { JsonString::from_str(name) }

pub(super) fn configuration_depth_ok(frame: &[u8]) -> bool {
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for &byte in frame {
        if quoted {
            if escaped { escaped = false; }
            else if byte == b'\\' { escaped = true; }
            else if byte == b'"' { quoted = false; }
        } else if byte == b'"' { quoted = true; }
        else if byte == b'{' || byte == b'[' {
            depth += 1;
            if depth > 128 { return false; }
        } else if byte == b'}' || byte == b']' { depth = depth.saturating_sub(1); }
    }
    true
}

/// Parent dispatch hint only. A frame with another schema stays on the
/// operations decoder; this does not authenticate the frame or caller.
pub(super) fn is_user_v37_configuration_frame(frame: &[u8]) -> bool {
    if frame.is_empty() || frame.len() > crate::ipc::MAX_FRAME_BYTES
        || !configuration_depth_ok(frame) { return false; }
    let Ok(text) = std::str::from_utf8(frame) else { return false; };
    let Ok(Json::Object(fields)) = Parser::parse(text) else { return false; };
    matches!(fields.get(&key("schema")), Some(Json::String(schema))
        if schema.to_well_formed_string().as_deref() == Some("gogoke.37.owner-configuration.v1"))
}

fn string_field(payload: &BTreeMap<JsonString, Json>, name: &'static str) -> Result<String> {
    match payload.get(&key(name)) {
        Some(Json::String(value)) => value.to_well_formed_string()
            .filter(|value| !value.is_empty() && !value.contains('\0'))
            .ok_or(OrchestrationError::Invalid(name)),
        _ => Err(OrchestrationError::Invalid(name)),
    }
}

fn exact_payload(request: &V37Request, fields: &[&str]) -> bool {
    request.payload.len() == fields.len()
        && fields.iter().all(|field| request.payload.contains_key(&key(field)))
}

fn seat_revision(seat: &Seat) -> Result<u64> {
    u64::try_from(seat.revision).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("seat revision: {error}")))
}

fn seat_result(seat: &Seat) -> Result<BTreeMap<JsonString, Json>> {
    let mut result = BTreeMap::from([
        (key("layer"), Json::String(JsonString::from_str(match seat.layer {
            seat::Layer::User => "USER", seat::Layer::Lead => "LEAD",
        }))),
        (key("kind"), Json::String(JsonString::from_str(match seat.kind {
            Kind::Long => "LONG", Kind::Short => "SHORT",
        }))),
        (key("state"), Json::String(JsonString::from_str(match seat.state {
            State::Idle => "IDLE", State::Busy => "BUSY", State::Reclaimed => "RECLAIMED",
        }))),
        (key("generation"), Json::String(JsonString::from_str(&seat.generation.to_string()))),
        (key("takeoverReady"), Json::Bool(false)),
        (key("takeoverAnswers"), Json::Null),
    ]);
    result.insert(key("instanceId"), if seat.instance_id.is_empty() { Json::Null }
        else { Json::String(JsonString::from_str(&seat.instance_id)) });
    result.insert(key("templateId"), match &seat.template_id {
        Some(id) => Json::String(JsonString::from_str(id)), None => Json::Null,
    });
    result.insert(key("settings"), match &seat.settings_json {
        Some(json) => Parser::parse(json).map_err(OrchestrationError::Atomic)?, None => Json::Null,
    });
    Ok(result)
}

fn receipt(request: &V37Request, status: V37Status, previous: u64, revision: u64,
    result: BTreeMap<JsonString, Json>) -> Vec<u8> {
    encode_receipt(request, status, previous, revision, result)
}

fn current(product: &mut ProductDatabase<'_>, request: &V37Request) -> Result<Option<Seat>> {
    product.connection.execute("BEGIN IMMEDIATE")
        .map_err(OrchestrationError::CommitUnknownWithCause)?;
    let found: Result<Option<Seat>> = (|| {
        authority::check_owner_in_current_transaction(&product.connection, &product.owner)?;
        seat::get(&product.connection, &request.domain_id, &request.target_id)
            .map_err(OrchestrationError::from)
    })();
    match found {
        Ok(found) => {
            product.connection.execute("COMMIT")
                .map_err(OrchestrationError::CommitUnknownWithCause)?;
            Ok(found)
        }
        Err(error) => {
            product.connection.execute("ROLLBACK")
                .map_err(OrchestrationError::CommitUnknownWithCause)?;
            Err(error)
        }
    }
}

fn status_for(error: &SeatError, request: &V37Request, present: Option<&Seat>) -> V37Status {
    match error {
        SeatError::Invalid(_) => V37Status::Denied,
        SeatError::Denied if present.is_some_and(|seat| seat.state == State::Reclaimed) => V37Status::Conflict,
        SeatError::Denied => V37Status::Denied,
        SeatError::Busy | SeatError::Conflict => {
            if present.is_some_and(|seat| u64::try_from(seat.revision).ok() != Some(request.expected_revision))
                && request.operation != "create-from-template" { V37Status::Stale }
            else { V37Status::Conflict }
        }
        SeatError::Unknown => V37Status::Conflict,
        SeatError::Store(_) | SeatError::Open(_) | SeatError::CommitUnknown(_)
        | SeatError::RollbackUnknown(_) | SeatError::HostResourceObservation(_)
        | SeatError::SchemaDrift => V37Status::Unknown,
    }
}

impl<'root> ProductDatabase<'root> {
    /// Parent integration: dispatch K-SEAT only after the UserOriginProof check.
    /// The native Owner issuer, never request JSON, establishes the user layer.
    pub(super) fn dispatch_user_seat(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        if request.family != "K-SEAT" { return Err(OrchestrationError::Invalid("family")); }
        if request.operation == "takeover-answers" {
            return Ok(receipt(request, V37Status::Unsupported,
                request.expected_revision, request.expected_revision, BTreeMap::new()));
        }
        let prior = current(self, request)?;
        let prior_revision = prior.as_ref().map(seat_revision).transpose()?.unwrap_or(0);
        if request.operation == "state-card" {
            if !exact_payload(request, &[]) {
                return Ok(receipt(request, V37Status::Denied, prior_revision, prior_revision, BTreeMap::new()));
            }
            // A current Owner check precedes every scoped read. ProductDatabase's
            // single connection is the seat store's connection, not a duplicate.
            return Ok(match prior {
                None => receipt(request, V37Status::Conflict, 0, 0, BTreeMap::new()),
                Some(_) if request.expected_revision != prior_revision =>
                    receipt(request, V37Status::Stale, prior_revision, prior_revision, BTreeMap::new()),
                Some(seat) => receipt(request, V37Status::Applied,
                    prior_revision, prior_revision, seat_result(&seat)?),
            });
        }
        let native = NativeOrigin::user(&self.owner);
        let outcome: std::result::Result<SeatReceipt, SeatError> = match request.operation.as_str() {
            "create-from-template" => {
                if request.expected_revision != 0 {
                    return Ok(receipt(request, V37Status::Stale, prior_revision, prior_revision, BTreeMap::new()));
                }
                // NativeOrigin::user currently creates USER seats. A LEAD seat
                // needs an explicit Owner API in seat::mod; a wire layer is not
                // an authority constructor.
                if !exact_payload(request, &["layer", "templateId"])
                    || string_field(&request.payload, "layer").ok().as_deref() != Some("USER") {
                    return Ok(receipt(request, V37Status::Unsupported, 0, 0, BTreeMap::new()));
                }
                let template_id = match string_field(&request.payload, "templateId") {
                    Ok(value) => value,
                    Err(_) => return Ok(receipt(request, V37Status::Denied, 0, 0, BTreeMap::new())),
                };
                seat::create(&mut self.connection, native, CreateSeat {
                    domain_id: &request.domain_id, seat_id: &request.target_id,
                    template_id: &template_id, instance_id: None, kind: Kind::Long,
                    request_id: &request.request_id, request_bytes: &request.raw_bytes,
                })
            }
            "tune" | "bind-instance" | "change-instance" | "reclaim" | "short-to-long" => {
                let expected_fields: &[&str] = if matches!(request.operation.as_str(), "bind-instance" | "change-instance") {
                    &["instanceId"]
                } else if request.operation == "tune" {
                    &["setting", "value"]
                } else { &[] };
                if !exact_payload(request, expected_fields) {
                    return Ok(receipt(request, V37Status::Denied, prior_revision, prior_revision, BTreeMap::new()));
                }
                let expected_revision = match i64::try_from(request.expected_revision) {
                    Ok(value) if value > 0 => value,
                    _ => return Ok(receipt(request, V37Status::Denied, prior_revision, prior_revision, BTreeMap::new())),
                };
                let change = SeatChange { domain_id: &request.domain_id, seat_id: &request.target_id,
                    expected_generation: prior.as_ref().map(|seat| seat.generation).unwrap_or(1),
                    expected_revision, request_id: &request.request_id, request_bytes: &request.raw_bytes };
                match request.operation.as_str() {
                    "tune" => {
                        let setting = match string_field(&request.payload, "setting") {
                            Ok(value) => value,
                            Err(_) => return Ok(receipt(request, V37Status::Denied, prior_revision, prior_revision, BTreeMap::new())),
                        };
                        let value = request.payload.get(&key("value")).expect("exact payload").canonical();
                        seat::tune(&mut self.connection, native, change, &setting, &value)
                    }
                    "bind-instance" => {
                        let instance_id = match string_field(&request.payload, "instanceId") {
                            Ok(value) => value,
                            Err(_) => return Ok(receipt(request, V37Status::Denied, prior_revision, prior_revision, BTreeMap::new())),
                        };
                        seat::bind_instance(&mut self.connection, native, change, &instance_id)
                    }
                    "change-instance" => {
                        let instance_id = match string_field(&request.payload, "instanceId") {
                            Ok(value) => value,
                            Err(_) => return Ok(receipt(request, V37Status::Denied, prior_revision, prior_revision, BTreeMap::new())),
                        };
                        seat::change_instance(&mut self.connection, native, change, &instance_id)
                    }
                    "reclaim" => seat::reclaim(&mut self.connection, native, change),
                    _ => seat::promote(&mut self.connection, native, change),
                }
            }
            _ => return Ok(receipt(request, V37Status::Unsupported,
                request.expected_revision, request.expected_revision, BTreeMap::new())),
        };
        match outcome {
            Ok(value) => {
                let revision = seat_revision(&value.seat)?;
                let previous = revision.saturating_sub(1);
                Ok(receipt(request, if value.replayed { V37Status::Replayed } else { V37Status::Applied },
                    previous, revision, seat_result(&value.seat)?))
            }
            Err(error) => {
                let after = current(self, request)?;
                let revision = after.as_ref().map(seat_revision).transpose()?.unwrap_or(0);
                let status = status_for(&error, request, after.as_ref());
                let mut result = BTreeMap::new();
                if status == V37Status::Unknown {
                    result.insert(key("reason"), Json::String(JsonString::from_str(&format!("native seat store: {error:?}"))));
                }
                Ok(receipt(request, status, revision, revision, result))
            }
        }
    }

    /// Separate Owner configuration plane. Parent must verify UserOriginProof
    /// before calling; service and seat ingress must never route here.
    pub(super) fn configure_user_v37(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        if frame.is_empty() || frame.len() > crate::ipc::MAX_FRAME_BYTES {
            return Err(OrchestrationError::Invalid("configuration frame"));
        }
        if !configuration_depth_ok(frame) {
            return Err(OrchestrationError::Invalid("configuration depth"));
        }
        let text = std::str::from_utf8(frame).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("configuration UTF-8: {error}")))?;
        let Json::Object(fields) = Parser::parse(text).map_err(OrchestrationError::Atomic)? else {
            return Err(OrchestrationError::Invalid("configuration object"));
        };
        if string_field(&fields, "schema")?.as_str() != "gogoke.37.owner-configuration.v1" {
            return Err(OrchestrationError::Invalid("configuration schema"));
        }
        let command = string_field(&fields, "command")?;
        let cap = |name: &'static str| -> Result<i64> {
            match fields.get(&key(name)) {
                Some(Json::Number(value)) => value.parse::<i64>().ok().filter(|value| *value > 0)
                    .ok_or(OrchestrationError::Invalid(name)),
                _ => Err(OrchestrationError::Invalid(name)),
            }
        };
        match command.as_str() {
            "project-parallel-cap" if fields.len() == 4 => {
                let domain = string_field(&fields, "domainId")?;
                seat::set_project_parallel_cap(&mut self.connection, &self.owner, &domain, cap("value")?)?;
            }
            "instance-concurrency-cap" if fields.len() == 4 => {
                let instance = string_field(&fields, "instanceId")?;
                instance::set_instance_concurrency_cap(&mut self.connection, &self.owner, &instance, cap("value")?)?;
            }
            "seat-template" if fields.len() == 5 => {
                let domain = string_field(&fields, "domainId")?;
                let template = string_field(&fields, "templateId")?;
                let settings = match fields.get(&key("settings")) {
                    Some(value @ Json::Object(_)) => value.canonical(),
                    _ => return Err(OrchestrationError::Invalid("settings")),
                };
                seat::store_template(&mut self.connection, NativeOrigin::user(&self.owner), StoreTemplate {
                    domain_id: &domain, template_id: &template, settings_json: settings.as_bytes(),
                })?;
            }
            "worktree-source" if fields.len() == 5 => {
                let repository = string_field(&fields, "repositoryId")?;
                let source = string_field(&fields, "sourcePath")?;
                let program = string_field(&fields, "gitPath")?;
                let pin = super::super::worktree::GitProgramPin::observe(
                    &mut self.connection, &self.owner, self.root,
                    Path::new(&program), &mut self.process_custodian)
                    .map_err(|error| OrchestrationError::V37StoreFailure(format!("native Git pin: {error:?}")))?;
                super::super::worktree::register_source(&mut self.connection, self.root,
                    &self.owner, &pin, &mut self.process_custodian,
                    super::super::worktree::SourceRegistration {
                        repository_id: &repository, source_path: Path::new(&source),
                    }).map_err(|error| OrchestrationError::V37StoreFailure(format!("native worktree source: {error:?}")))?;
            }
            _ => return Err(OrchestrationError::Invalid("configuration command or fields")),
        }
        Ok(Json::Object(BTreeMap::from([
            (key("schema"), Json::String(JsonString::from_str("gogoke.37.owner-configuration.v1"))),
            (key("command"), Json::String(JsonString::from_str(&command))),
            (key("status"), Json::String(JsonString::from_str("APPLIED"))),
        ])).canonical().into_bytes())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::store::session_transport::decode_receipt;
    use crate::store::same_open::route_b_test_guard;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn request(operation: &str, request_id: &str, target_id: &str,
        expected: u64, payload: &str) -> V37Request {
        let frame = format!(
            "{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SEAT\",\"operation\":\"{operation}\",\"requestId\":\"{request_id}\",\"targetId\":\"{target_id}\",\"domainId\":\"projectA\",\"expectedRevision\":\"{expected}\",\"payload\":{payload}}}"
        );
        decode_request(frame.as_bytes()).unwrap()
    }

    fn status(product: &mut ProductDatabase<'_>, request: &V37Request) -> V37Status {
        decode_receipt(&product.dispatch_user_seat(request).unwrap()).unwrap().status
    }

    fn fixture(run: impl FnOnce(&mut ProductDatabase<'_>)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-v37-user-seat-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let database = path.join("state.sqlite");
        let mut product = ProductDatabase::open(&root, &database).unwrap();
        run(&mut product);
        product.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).unwrap();
        std::fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
        if let Err(error) = std::fs::remove_dir(&path) {
            eprintln!("owned fixture retained: {error}");
        }
    }

    #[test]
    fn owner_configuration_and_user_seat_share_the_verified_product_store() {
        fixture(|product| {
            assert!(is_user_v37_configuration_frame(br#"{"schema":"gogoke.37.owner-configuration.v1","command":"project-parallel-cap","domainId":"projectA","value":2}"#));
            assert!(!is_user_v37_configuration_frame(br#"{"schema":"gogoke.37.operations.v1","family":"K-SEAT"}"#));
            let config = |product: &mut ProductDatabase<'_>, frame: &str| {
                product.configure_user_v37(frame.as_bytes()).unwrap()
            };
            config(product, r#"{"schema":"gogoke.37.owner-configuration.v1","command":"project-parallel-cap","domainId":"projectA","value":2}"#);
            assert_eq!(seat::read_project_parallel_cap(&product.connection, "projectA").unwrap(), 2);
            assert!(product.configure_user_v37(br#"{"schema":"gogoke.37.owner-configuration.v1","command":"project-parallel-cap","domainId":"projectA","value":0}"#).is_err());
            assert!(product.configure_user_v37(br#"{"schema":"gogoke.37.owner-configuration.v1","command":"project-parallel-cap","domainId":"projectA","value":2,"sql":"DROP TABLE"}"#).is_err());
            config(product, r#"{"schema":"gogoke.37.owner-configuration.v1","command":"seat-template","domainId":"projectA","templateId":"templateA","settings":{"instruction":"default"}}"#);
            let insert = Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','homeA','identityA','sha256:test','1','INSTALLED','LOGGED_IN',1)").unwrap();
            insert.step_done().unwrap();
            config(product, r#"{"schema":"gogoke.37.owner-configuration.v1","command":"instance-concurrency-cap","instanceId":"instanceA","value":2}"#);
            assert_eq!(instance::read_instance_concurrency_cap(&product.connection, "instanceA").unwrap(), 2);
            let create = request("create-from-template", "createA", "seatA", 0,
                r#"{"layer":"USER","templateId":"templateA"}"#);
            assert_eq!(status(product, &create), V37Status::Applied);
            assert_eq!(status(product, &create), V37Status::Replayed);
            let card = request("state-card", "cardA", "seatA", 1, "{}");
            let card_receipt = product.dispatch_user_seat(&card).unwrap();
            assert_eq!(decode_receipt(&card_receipt).unwrap().status, V37Status::Applied);
            let card_text = std::str::from_utf8(&card_receipt).unwrap();
            assert!(card_text.contains("\"instruction\":\"default\""));
            assert_eq!(status(product, &request("tune", "tuneA", "seatA", 1,
                r#"{"setting":"instruction","value":"changed"}"#)), V37Status::Applied);
            assert_eq!(status(product, &card), V37Status::Stale);
            assert_eq!(status(product, &request("bind-instance", "bindA", "seatA", 2,
                r#"{"instanceId":"instanceA"}"#)), V37Status::Applied);
            assert_eq!(status(product, &request("change-instance", "changeBusy", "seatA", 1,
                r#"{"instanceId":"instanceA"}"#)), V37Status::Stale);
            assert_eq!(status(product, &request("reclaim", "reclaimA", "seatA", 3, "{}")), V37Status::Applied);
            assert_eq!(status(product, &request("short-to-long", "latePromote", "seatA", 4, "{}")), V37Status::Conflict);
            assert_eq!(status(product, &request("takeover-answers", "takeoverA", "seatA", 4, "{}")), V37Status::Unsupported);
        });
    }
}
