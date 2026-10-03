//! Fixed non-Codex CLI login under the existing User action and process custody.
//! Account state comes only from an independent command of that exact CLI.

use super::*;
use crate::store::instance::provider_login::{
    LoginPreparation, PreparedProviderLogin, StatusObservation,
};

fn preparation_error(error: impl std::fmt::Debug) -> OrchestrationError {
    OrchestrationError::V37StoreFailure(format!("registered provider login preparation: {error:?}"))
}

fn status_operation_id(command: &OwnerLoginCommand) -> String {
    let digest = crate::store::digest::sha256_hex(
        format!("{}\nstatus", owner_login_operation_id(command)).as_bytes(),
    );
    format!("owner-login-status-{}", &digest[..40])
}

/// The status command can print other account fields. Read only loggedIn and
/// require the documented exit code to agree; discard the original JSON.
fn classify_status(
    stdout: &[u8],
    exit: Option<u32>,
    logged_in: i32,
    logged_out: i32,
) -> NativeAccountState {
    let Ok(text) = std::str::from_utf8(stdout) else {
        return NativeAccountState::Unknown;
    };
    let Ok(Json::Object(mut fields)) = Parser::parse(text.trim()) else {
        return NativeAccountState::Unknown;
    };
    match fields.remove(&JsonString::from_str("loggedIn")) {
        Some(Json::Bool(true)) if exit == u32::try_from(logged_in).ok() => {
            NativeAccountState::CredentialPresent
        }
        Some(Json::Bool(false)) if exit == u32::try_from(logged_out).ok() => {
            NativeAccountState::LoggedOut
        }
        _ => NativeAccountState::Unknown,
    }
}

impl<'root> ProductDatabase<'root> {
    pub(super) fn begin_registered_provider_login(
        &mut self,
        command: &OwnerLoginCommand,
    ) -> Result<Vec<u8>> {
        let provider = match instance::provider_login::prepare_registered_provider_login(
            &mut self.connection,
            self.root,
            &self.owner,
            &command.instance_id,
        )
        .map_err(preparation_error)
        .map_err(|error| self.settle_owner_login_preflight_error(command, error))?
        {
            LoginPreparation::Ready(prepared) => prepared,
            LoginPreparation::Unsupported { driver_id, reason } => {
                let error = OrchestrationError::V37StoreFailure(format!(
                    "registered provider login unsupported: driver={driver_id}; {reason}"
                ));
                return Err(self.settle_owner_login_preflight_error(command, error));
            }
        };
        if provider.instance_id != command.instance_id
            || provider.login.binding.generation != command.expected_revision.to_string()
        {
            return Err(
                self.settle_owner_login_preflight_error(command, OrchestrationError::AccessDenied)
            );
        }
        // Keep the existing login cleanup path pointed at a new, host-owned
        // child. It must never be allowed to recursively remove F's home.
        let (runtime, runtime_identity) = runtime_home(&provider.home.path)
            .map_err(|error| self.settle_owner_login_preflight_error(command, error))?;
        let launch = PreparedOwnerLogin {
            login: provider.login.clone(),
            account_read: provider.login.clone(), // unused for this CLI
            runtime_home: runtime,
            runtime_identity,
        };
        let reply = self.start_owner_device_login(command, launch, |custodian, prepared| {
            custodian.activate(prepared)
        })?;
        if let Some(OwnerLoginSession::Active(active)) = &mut self.owner_login {
            active.rpc = None;
            active.provider = Some(provider);
        } else {
            return Err(OrchestrationError::OperationConflict);
        }
        Ok(reply)
    }

    pub(super) fn provider_login_account_state(
        &mut self,
        command: &OwnerLoginCommand,
        provider: PreparedProviderLogin,
    ) -> Result<String> {
        // Login may have spent minutes in a browser. Resolve F and the catalog
        // again before starting a second CLI process in that home.
        let fresh = match instance::provider_login::prepare_registered_provider_login(
            &mut self.connection,
            self.root,
            &self.owner,
            &command.instance_id,
        )
        .map_err(preparation_error)?
        {
            LoginPreparation::Ready(prepared) => prepared,
            LoginPreparation::Unsupported { .. } => return Err(OrchestrationError::AccessDenied),
        };
        if fresh.home.identity != provider.home.identity
            || fresh.driver_id != provider.driver_id
            || fresh.version != provider.version
            || fresh.program_digest != provider.program_digest
            || fresh.login.binding.generation != provider.login.binding.generation
        {
            return Err(OrchestrationError::AccessDenied);
        }
        let state = match &fresh.status {
            StatusObservation::Unknown(_) => NativeAccountState::Unknown,
            StatusObservation::Documented(status) => {
                self.observe_provider_status(command, status)?
            }
        };
        self.record_provider_state(command, &fresh, state)
    }

    fn observe_provider_status(
        &mut self,
        command: &OwnerLoginCommand,
        status: &instance::provider_login::PreparedStatusObservation,
    ) -> Result<NativeAccountState> {
        let operation_id = status_operation_id(command);
        let prepared = match self.process_custodian.prepare(&status.request) {
            Ok(prepared) => prepared,
            Err(error) => return Err(OrchestrationError::Process(error)),
        };
        let pending = |proof: Option<NativeStopProof>, abort_prepared| PendingAccountCustody {
            operation_id: Some(operation_id.clone()),
            prepared: Some(prepared.clone()),
            runtime_home: None,
            runtime_identity: None,
            proof,
            durable_revision: None,
            abort_prepared,
            frame: None,
            request: None,
        };
        if let Err(error) =
            authority::record_prepared_process(&mut self.connection, &operation_id, &prepared)
        {
            let abort = self.process_custodian.abort_prepared(&prepared);
            if abort.is_err() {
                self.owner_login = Some(OwnerLoginSession::PendingAccount(PendingAccountRead {
                    instance_id: command.instance_id.clone(),
                    request_id: command.request_id.clone(),
                    expected_revision: command.expected_revision,
                    output: format!("provider status prepare record: {error:?}; abort: {abort:?}"),
                    latest_error: None,
                    custody: pending(None, true),
                }));
            }
            return Err(OrchestrationError::V37StoreFailure(format!(
                "provider status prepare record: {error:?}; abort: {abort:?}"
            )));
        }
        if let Err(error) = self.process_custodian.activate(&prepared) {
            let released = activation_was_aborted(
                &error,
                self.process_custodian.is_tombstoned(&prepared.ticket),
            );
            let unknown =
                authority::mark_process_unknown(&mut self.connection, &operation_id, &prepared);
            if !released {
                self.owner_login = Some(OwnerLoginSession::PendingAccount(PendingAccountRead {
                    instance_id: command.instance_id.clone(),
                    request_id: command.request_id.clone(),
                    expected_revision: command.expected_revision,
                    output: format!(
                        "provider status activate: {error:?}; unknown record: {unknown:?}"
                    ),
                    latest_error: None,
                    custody: pending(None, true),
                }));
            }
            return Err(OrchestrationError::V37StoreFailure(format!(
                "provider status activate: {error:?}; unknown record: {unknown:?}"
            )));
        }
        if let Err(error) =
            authority::mark_process_active(&mut self.connection, &operation_id, &prepared)
        {
            let stop =
                self.process_custodian
                    .stop(&prepared.ticket, StopBudgets::production(), || Ok(()));
            let unknown =
                authority::mark_process_unknown(&mut self.connection, &operation_id, &prepared);
            self.owner_login = Some(OwnerLoginSession::PendingAccount(PendingAccountRead {
                instance_id: command.instance_id.clone(), request_id: command.request_id.clone(),
                expected_revision: command.expected_revision, output: format!(
                    "provider status active record: {error:?}; stop: {stop:?}; unknown record: {unknown:?}"),
                latest_error: None, custody: pending(stop.ok(), false),
            }));
            return Err(OrchestrationError::V37StoreFailure(format!(
                "provider status active record: {error:?}; unknown record: {unknown:?}"
            )));
        }
        let output = self
            .process_custodian
            .read_persistent_child_frame(&prepared.ticket, Duration::from_secs(15));
        let output = match output {
            Ok(frame) if frame.custody() == &prepared && frame.bytes().len() <= 65_536 => {
                Ok(frame.bytes().to_vec())
            }
            Ok(_) => Err("provider status output custody or size mismatch".to_owned()),
            Err(error) => Err(format!("provider status output: {error:?}")),
        };
        let exited = self
            .process_custodian
            .active(&prepared.ticket)
            .ok_or(OrchestrationError::AccessDenied)?
            .wait(Duration::from_secs(15))
            .map_err(|error| format!("provider status child wait: {error}"));
        let stop = self
            .process_custodian
            .stop(&prepared.ticket, StopBudgets::production(), || Ok(()));
        let proof = match stop {
            Ok(proof) => proof,
            Err(error) => {
                let unknown =
                    authority::mark_process_unknown(&mut self.connection, &operation_id, &prepared);
                self.owner_login = Some(OwnerLoginSession::PendingAccount(PendingAccountRead {
                    instance_id: command.instance_id.clone(), request_id: command.request_id.clone(),
                    expected_revision: command.expected_revision, output: format!(
                        "provider status stop: {error:?}; output: {:?}; wait: {exited:?}; unknown record: {unknown:?}", output.as_ref().err()),
                    latest_error: None, custody: pending(None, false),
                }));
                return Err(OrchestrationError::Process(error));
            }
        };
        let revision =
            match authority::mark_process_stopped(&mut self.connection, &operation_id, &proof) {
                Ok(revision) => revision,
                Err(error) => {
                    let unknown = authority::mark_process_unknown(
                        &mut self.connection,
                        &operation_id,
                        &prepared,
                    );
                    self.owner_login =
                        Some(OwnerLoginSession::PendingAccount(PendingAccountRead {
                            instance_id: command.instance_id.clone(),
                            request_id: command.request_id.clone(),
                            expected_revision: command.expected_revision,
                            output: format!(
                        "provider status stop record: {error:?}; unknown record: {unknown:?}"),
                            latest_error: None,
                            custody: pending(Some(proof), false),
                        }));
                    return Err(error);
                }
            };
        if let Err(error) = self
            .process_custodian
            .confirm_stop_durable(&DurableStopConfirmation {
                ticket: prepared.ticket.clone(),
                custodian_nonce: prepared.custodian_nonce.clone(),
                identity: prepared.identity.clone(),
                proof_hash: proof.proof_hash(),
                durable_revision: revision,
            })
        {
            let mut custody = pending(Some(proof), false);
            custody.durable_revision = Some(revision);
            self.owner_login = Some(OwnerLoginSession::PendingAccount(PendingAccountRead {
                instance_id: command.instance_id.clone(),
                request_id: command.request_id.clone(),
                expected_revision: command.expected_revision,
                output: format!("provider status stop confirmation: {error:?}"),
                latest_error: None,
                custody,
            }));
            return Err(error.into());
        }
        let bytes = output.map_err(OrchestrationError::V37StoreFailure)?;
        if !exited.map_err(OrchestrationError::V37StoreFailure)? {
            return Ok(NativeAccountState::Unknown);
        }
        Ok(classify_status(
            &bytes,
            proof.exit_code,
            status.logged_in_exit,
            status.logged_out_exit,
        ))
    }

    fn record_provider_state(
        &mut self,
        command: &OwnerLoginCommand,
        provider: &PreparedProviderLogin,
        state: NativeAccountState,
    ) -> Result<String> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        let current = self.user_instance_revision(&command.instance_id)?;
        if current != command.expected_revision
            || provider.instance_id != command.instance_id
            || provider.login.binding.generation != current.to_string()
        {
            return Err(OrchestrationError::OperationConflict);
        }
        let row = self
            .read_registered_instance(&command.instance_id)?
            .ok_or(OrchestrationError::AccessDenied)?;
        if row.driver_id != provider.driver_id
            || row.version != provider.version
            || row.program_digest != provider.program_digest
        {
            return Err(OrchestrationError::AccessDenied);
        }
        let request = V37Request {
            raw_bytes: format!(
                "owner-login-provider-status:{}:{}",
                command.instance_id, command.request_id
            )
            .into_bytes(),
            family: "K-INSTANCE".into(),
            operation: "login-state".into(),
            request_id: format!("{}-account-read", command.request_id),
            target_id: command.instance_id.clone(),
            domain_id: "global".into(),
            expected_revision: current,
            payload: BTreeMap::new(),
        };
        if let Some(receipt) = self.prior_login_state_request(&request)? {
            return owner_login_state_from_receipt(&receipt);
        }
        if row.login_state == state.public_state()
            && (state == NativeAccountState::Unknown
                || self.current_login_observation(
                    &command.instance_id,
                    current,
                    &row.login_state,
                )?)
        {
            return owner_login_state_from_receipt(
                &self.record_unchanged_login_state(&request, state)?,
            );
        }
        let observation = ObservationRequest {
            request_id: &request.request_id,
            request_bytes: &request.raw_bytes,
            instance_id: &command.instance_id,
            expected_revision: i64::try_from(current).map_err(|error| {
                OrchestrationError::V37StoreFailure(format!("provider revision overflow: {error}"))
            })?,
            observation: state.durable(),
        };
        match instance::record_observation(&mut self.connection, self.root, &observation) {
            Ok(RegistrationDisposition::Applied | RegistrationDisposition::Replayed) => {
                Ok(state.public_state().into())
            }
            Ok(_) => Err(OrchestrationError::OperationConflict),
            Err(error) => Err(OrchestrationError::V37StoreFailure(format!(
                "provider status observation: {error:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_needs_matching_fixed_exit_and_boolean() {
        assert_eq!(
            classify_status(br#"{"loggedIn":true,"account":"private"}"#, Some(0), 0, 1),
            NativeAccountState::CredentialPresent
        );
        assert_eq!(
            classify_status(br#"{"loggedIn":false,"authMethod":"none"}"#, Some(1), 0, 1),
            NativeAccountState::LoggedOut
        );
        for (json, exit) in [
            (&br#"{"loggedIn":true}"#[..], Some(1)),
            (&br#"{"loggedIn":false}"#[..], Some(0)),
            (&br#"{"loggedIn":"true"}"#[..], Some(0)),
            (&br#"{"account":"private"}"#[..], Some(0)),
            (&br#"{"loggedIn":true}"#[..], None),
        ] {
            assert_eq!(
                classify_status(json, exit, 0, 1),
                NativeAccountState::Unknown
            );
        }
    }
}
