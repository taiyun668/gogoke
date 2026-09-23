//! Closed typed operation decoder. Not a SQL transport and not an IPC session.

use std::collections::{BTreeMap, BTreeSet};

const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

const ADMITTED: &[&str] = &[
    "CommitOrchestration",
    "CommitDomainRecord",
    "CommitContextVersion",
    "ReserveAction",
    "BeginActionCommitment",
    "RecordRejectedCommand",
    "ReconcileOperation",
    "ReadEvents",
    "ReadAggregateRange",
    "ReadSnapshot",
    "GetReceipt",
    "AuthenticateService",
    "ReadProductIdentity",
    "AdmitControllerCaller",
    "PrepareR2TestDelegation",
    "PrepareR2TestPackage",
    "PrepareR2TestLineage",
    "PrepareR2TestTask",
    "PrepareR2TestRecipe",
    "RunControlledFixtureProbe",
    "RunControlledFixtureAction",
    "PublishDecisionSnapshot",
    "CommitDecision",
    "ReadDecisionReplay",
    "PublishContextAssemblySnapshot",
    "ReadContextAssemblyBasis",
    "ListContextAssemblySources",
    "ReadGranteeContextSet",
    "CommitContextManifest",
    "ReadContextManifest",
    "CommitTaskContextRequirements",
    "ReadTaskContextRequirements",
    "ReadCurrentDelegationGrant",
    "AppendExecutionRecipe",
    "ReadCurrentExecutionRecipe",
    "ReadExecutionRecipeRevision",
    "AppendObjectiveOutcome",
    "ReadObjectiveOutcome",
    "AppendEvaluation",
    "ReadEvaluation",
    "AppendDreamRun",
    "ReadDreamRun",
    "AppendDreamProposal",
    "ReadDreamProposal",
    "ApplySessionLineageCommand",
    "ReadSessionLineage",
    "ReadExposureReceipt",
    "Shutdown",
];

const FORBIDDEN: &[&str] = &[
    "execute",
    "execSql",
    "prepareSql",
    "begin",
    "commit",
    "rollback",
    "savepoint",
    "attach",
    "openDatabase",
    "rawTablePatch",
    "arbitraryKeyValue",
    "sql",
    "query",
];

#[derive(Debug, Eq, PartialEq)]
pub enum ProtocolError {
    Oversize,
    NonCanonical,
    DuplicateKey,
    UnknownOperation(String),
    ForbiddenField(&'static str),
    MissingOperation,
}

#[derive(Debug, Eq, PartialEq)]
pub struct DecodedOperation {
    pub name: &'static str,
    pub principal: Option<String>,
    pub domain_id: Option<String>,
    pub host_epoch: Option<String>,
    pub schema_fingerprint: Option<String>,
}

fn admitted(name: &str) -> Option<&'static str> {
    ADMITTED.iter().copied().find(|item| *item == name)
}

fn forbidden_field(key: &str) -> Option<&'static str> {
    FORBIDDEN
        .iter()
        .copied()
        .find(|item| item.eq_ignore_ascii_case(key))
}

/// Decode one request frame. Duplicate keys and non-canonical JSON fail closed.
pub fn decode_operation_frame(bytes: &[u8]) -> Result<DecodedOperation, ProtocolError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::Oversize);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ProtocolError::NonCanonical)?;
    let mut parser = FrameParser { text, offset: 0 };
    parser.skip_ws();
    if parser.next_byte() != Some(b'{') {
        return Err(ProtocolError::NonCanonical);
    }
    parser.offset += 1;
    parser.skip_ws();
    let mut operation = None;
    let mut principal = None;
    let mut domain_id = None;
    let mut host_epoch = None;
    let mut schema_fingerprint = None;
    let mut seen = std::collections::BTreeSet::new();
    if parser.next_byte() != Some(b'}') {
        loop {
            parser.skip_ws();
            let key = parser.parse_string()?;
            if !seen.insert(key.clone()) {
                return Err(ProtocolError::DuplicateKey);
            }
            if let Some(field) = forbidden_field(&key) {
                return Err(ProtocolError::ForbiddenField(field));
            }
            parser.skip_ws();
            if parser.next_byte() != Some(b':') {
                return Err(ProtocolError::NonCanonical);
            }
            parser.offset += 1;
            parser.skip_ws();
            if key == "operation" {
                operation = Some(parser.parse_string()?);
            } else if key == "principal" {
                principal = Some(parser.parse_string()?);
            } else if key == "domainId" {
                domain_id = Some(parser.parse_string()?);
            } else if key == "hostEpoch" {
                host_epoch = Some(parser.parse_string()?);
            } else if key == "schemaFingerprint" {
                schema_fingerprint = Some(parser.parse_string()?);
            } else {
                parser.skip_value()?;
            }
            parser.skip_ws();
            match parser.next_byte() {
                Some(b',') => parser.offset += 1,
                Some(b'}') => {
                    parser.offset += 1;
                    break;
                }
                _ => return Err(ProtocolError::NonCanonical),
            }
        }
    }
    parser.skip_ws();
    if parser.offset != parser.text.len() {
        return Err(ProtocolError::NonCanonical);
    }
    let name = operation.ok_or(ProtocolError::MissingOperation)?;
    let admitted = admitted(&name).ok_or(ProtocolError::UnknownOperation(name))?;
    Ok(DecodedOperation {
        name: admitted,
        principal,
        domain_id,
        host_epoch,
        schema_fingerprint,
    })
}

pub(crate) fn decode_flat_string_object(
    bytes: &[u8],
) -> Result<BTreeMap<String, String>, ProtocolError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::Oversize);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ProtocolError::NonCanonical)?;
    let mut parser = FrameParser { text, offset: 0 };
    parser.skip_ws();
    if parser.next_byte() != Some(b'{') {
        return Err(ProtocolError::NonCanonical);
    }
    parser.offset += 1;
    parser.skip_ws();
    let mut fields = BTreeMap::new();
    let mut seen = BTreeSet::new();
    if parser.next_byte() == Some(b'}') {
        parser.offset += 1;
    } else {
        loop {
            parser.skip_ws();
            let key = parser.parse_string()?;
            if !seen.insert(key.clone()) {
                return Err(ProtocolError::DuplicateKey);
            }
            if let Some(field) = forbidden_field(&key) {
                return Err(ProtocolError::ForbiddenField(field));
            }
            parser.skip_ws();
            if parser.next_byte() != Some(b':') {
                return Err(ProtocolError::NonCanonical);
            }
            parser.offset += 1;
            parser.skip_ws();
            let value = parser.parse_string()?;
            fields.insert(key, value);
            parser.skip_ws();
            match parser.next_byte() {
                Some(b',') => parser.offset += 1,
                Some(b'}') => {
                    parser.offset += 1;
                    break;
                }
                _ => return Err(ProtocolError::NonCanonical),
            }
        }
    }
    parser.skip_ws();
    if parser.offset != parser.text.len() {
        return Err(ProtocolError::NonCanonical);
    }
    Ok(fields)
}

struct FrameParser<'a> {
    text: &'a str,
    offset: usize,
}

impl<'a> FrameParser<'a> {
    fn next_byte(&self) -> Option<u8> {
        self.text.as_bytes().get(self.offset).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.next_byte(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.offset += 1;
        }
    }

    fn parse_string(&mut self) -> Result<String, ProtocolError> {
        if self.next_byte() != Some(b'"') {
            return Err(ProtocolError::NonCanonical);
        }
        self.offset += 1;
        let mut output = String::new();
        let bytes = self.text.as_bytes();
        while let Some(&byte) = bytes.get(self.offset) {
            match byte {
                b'"' => {
                    self.offset += 1;
                    return Ok(output);
                }
                b'\\' => {
                    self.offset += 1;
                    match bytes.get(self.offset) {
                        Some(b'"') => output.push('"'),
                        Some(b'\\') => output.push('\\'),
                        Some(b'/') => output.push('/'),
                        Some(b'b') => output.push('\u{0008}'),
                        Some(b'f') => output.push('\u{000c}'),
                        Some(b'n') => output.push('\n'),
                        Some(b'r') => output.push('\r'),
                        Some(b't') => output.push('\t'),
                        _ => return Err(ProtocolError::NonCanonical),
                    }
                    self.offset += 1;
                }
                b if b < 0x20 => return Err(ProtocolError::NonCanonical),
                _ => {
                    let ch = self.text[self.offset..]
                        .chars()
                        .next()
                        .ok_or(ProtocolError::NonCanonical)?;
                    output.push(ch);
                    self.offset += ch.len_utf8();
                }
            }
        }
        Err(ProtocolError::NonCanonical)
    }

    fn skip_value(&mut self) -> Result<(), ProtocolError> {
        match self.next_byte() {
            Some(b'"') => {
                self.parse_string()?;
                Ok(())
            }
            Some(b'{') => self.skip_container(b'{', b'}'),
            Some(b'[') => self.skip_container(b'[', b']'),
            Some(b't') => self.skip_literal("true"),
            Some(b'f') => self.skip_literal("false"),
            Some(b'n') => self.skip_literal("null"),
            Some(b'-') | Some(b'0'..=b'9') => {
                while matches!(
                    self.next_byte(),
                    Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                ) {
                    self.offset += 1;
                }
                Ok(())
            }
            _ => Err(ProtocolError::NonCanonical),
        }
    }

    fn skip_literal(&mut self, literal: &str) -> Result<(), ProtocolError> {
        if self.text[self.offset..].starts_with(literal) {
            self.offset += literal.len();
            Ok(())
        } else {
            Err(ProtocolError::NonCanonical)
        }
    }

    fn skip_container(&mut self, open: u8, close: u8) -> Result<(), ProtocolError> {
        if self.next_byte() != Some(open) {
            return Err(ProtocolError::NonCanonical);
        }
        self.offset += 1;
        let mut depth = 1u32;
        let mut in_string = false;
        let mut escape = false;
        while let Some(byte) = self.next_byte() {
            self.offset += 1;
            if in_string {
                if escape {
                    escape = false;
                } else if byte == b'\\' {
                    escape = true;
                } else if byte == b'"' {
                    in_string = false;
                }
                continue;
            }
            match byte {
                b'"' => in_string = true,
                b if b == open => depth += 1,
                b if b == close => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        Err(ProtocolError::NonCanonical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_closed_operations_and_rejects_sql_and_unknown() {
        let decoded = decode_operation_frame(
            br#"{"domainId":"d","hostEpoch":"1","operation":"GetReceipt","principal":"user","schemaFingerprint":"abc"}"#,
        )
        .expect("ok");
        assert_eq!(decoded.name, "GetReceipt");
        assert_eq!(decoded.principal.as_deref(), Some("user"));
        assert_eq!(decoded.domain_id.as_deref(), Some("d"));
        assert_eq!(decoded.host_epoch.as_deref(), Some("1"));
        assert_eq!(decoded.schema_fingerprint.as_deref(), Some("abc"));
        assert!(matches!(
            decode_operation_frame(br#"{"operation":"execute","sql":"SELECT 1"}"#),
            Err(ProtocolError::ForbiddenField("sql"))
                | Err(ProtocolError::UnknownOperation(_))
                | Err(ProtocolError::ForbiddenField("execute"))
        ));
        assert!(matches!(
            decode_operation_frame(br#"{"operation":"CommitSomethingElse"}"#),
            Err(ProtocolError::UnknownOperation(_))
        ));
        assert!(matches!(
            decode_operation_frame(&vec![b'x'; MAX_FRAME_BYTES + 1]),
            Err(ProtocolError::Oversize)
        ));
    }

    #[test]
    fn commit_orchestration_frame_is_admitted() {
        let frame = br#"{"accessAdmitted":true,"commandId":"cmd-project","commandType":"project.create","events":[{"eventId":"ev-p","occurredAt":"2026-09-20T00:00:00Z","projectId":"proj-1","title":"one","type":"project.created","workspaceRoot":"C:/tmp/one"}],"operation":"CommitOrchestration"}"#;
        let decoded = decode_operation_frame(frame).expect("decode");
        assert_eq!(decoded.name, "CommitOrchestration");
    }

    #[test]
    fn commit_context_version_is_a_closed_admitted_operation() {
        let decoded = decode_operation_frame(
            br#"{"domainId":"domain-one","operation":"CommitContextVersion","operationId":"operation-one"}"#,
        )
        .expect("decode");
        assert_eq!(decoded.name, "CommitContextVersion");
        assert_eq!(decoded.domain_id.as_deref(), Some("domain-one"));
    }

    #[test]
    fn flat_authority_fields_reject_nested_override_and_numeric_coercion() {
        assert!(matches!(
            decode_flat_string_object(
                br#"{"contextId":"outer","extra":{"contextId":"inner"},"operation":"CommitContextVersion"}"#,
            ),
            Err(ProtocolError::NonCanonical)
        ));
        assert!(matches!(
            decode_flat_string_object(br#"{"operation":"CommitContextVersion","version":1.5}"#,),
            Err(ProtocolError::NonCanonical)
        ));
        let decoded = decode_flat_string_object(
            br#"{"contextId":"outer","operation":"CommitContextVersion","version":"1"}"#,
        )
        .unwrap();
        assert_eq!(decoded.get("contextId").map(String::as_str), Some("outer"));
    }

    #[test]
    fn action_operations_are_closed_and_untrusted_outcome_ingress_is_not_admitted() {
        for operation in ["ReserveAction", "BeginActionCommitment"] {
            let frame = format!("{{\"operation\":\"{operation}\"}}");
            assert_eq!(
                decode_operation_frame(frame.as_bytes()).unwrap().name,
                operation
            );
        }
        assert!(matches!(
            decode_operation_frame(br#"{"operation":"RecordActionOutcome"}"#),
            Err(ProtocolError::UnknownOperation(value)) if value == "RecordActionOutcome"
        ));
    }

    #[test]
    fn owner_outcome_append_is_not_a_service_protocol_operation() {
        assert!(matches!(
            decode_operation_frame(br#"{"operation":"AppendOwnerOutcome"}"#),
            Err(ProtocolError::UnknownOperation(value)) if value == "AppendOwnerOutcome"
        ));
    }

    #[test]
    fn context_authority_service_operations_are_closed_and_admitted() {
        for operation in [
            "PublishContextAssemblySnapshot",
            "ReadContextAssemblyBasis",
            "ListContextAssemblySources",
            "ReadGranteeContextSet",
            "CommitContextManifest",
            "ReadContextManifest",
            "CommitTaskContextRequirements",
            "ReadTaskContextRequirements",
        ] {
            let frame = format!("{{\"operation\":\"{operation}\"}}");
            assert_eq!(
                decode_operation_frame(frame.as_bytes()).unwrap().name,
                operation
            );
        }
        assert_eq!(
            decode_operation_frame(
                br#"{"grantRef":"grant-one","operation":"ReadCurrentDelegationGrant"}"#
            )
            .unwrap()
            .name,
            "ReadCurrentDelegationGrant"
        );
    }

    #[test]
    fn duplicate_operation_key_fails_closed() {
        assert!(matches!(
            decode_operation_frame(br#"{"operation":"GetReceipt","operation":"GetReceipt"}"#),
            Err(ProtocolError::DuplicateKey) | Err(ProtocolError::NonCanonical)
        ));
    }
}
