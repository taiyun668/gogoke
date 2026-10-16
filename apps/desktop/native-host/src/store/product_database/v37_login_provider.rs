//! Fixed non-Codex CLI login under the existing User action and process custody.
//! Account state comes from fixed CLI evidence under the original custody.

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

fn strip_csi_colors(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            result.push(ch);
            continue;
        }
        if chars.next()? != '[' {
            return None;
        }
        let mut length = 0;
        loop {
            let next = chars.next()?;
            if next == 'm' {
                break;
            }
            if !next.is_ascii_digit() && next != ';' {
                return None;
            }
            length += 1;
            if length > 32 {
                return None;
            }
        }
    }
    Some(result)
}

fn classify_opencode_credential_list(stdout: &[u8], exit: Option<u32>) -> NativeAccountState {
    if exit != Some(0) {
        return NativeAccountState::Unknown;
    }
    let Some(text) = strip_csi_colors(stdout) else {
        return NativeAccountState::Unknown;
    };
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect();
    // The fixed 1.18.32 formatter prints each local credential through
    // prompts.log.info as its info symbol plus model display name and type.
    // Clack's pinned info symbols are exactly "●" or "•". Its xAI catalog
    // entry is named "xAI". Classify only the exact one-entry inventory;
    // additional credentials, duplicates, other types, or extra output stay
    // UNKNOWN.
    if lines.len() == 3
        && lines[0].starts_with("T  Credentials ")
        && lines[1] == "|"
        && lines[2] == "—  0 credentials"
    {
        NativeAccountState::LoggedOut
    } else if lines.len() == 4
        && lines[0].starts_with("T  Credentials ")
        && lines[1] == "|"
        && matches!(lines[2], "●  xAI oauth" | "•  xAI oauth")
        && lines[3] == "—  1 credentials"
    {
        NativeAccountState::CredentialPresent
    } else {
        NativeAccountState::Unknown
    }
}

/// The pinned OpenCode xAI OAuth callback returns tokens to the CLI, which
/// persists them before printing this complete, LF-terminated spinner stop
/// line. The caller must separately prove the original prepared xAI process
/// stopped at zero without cancellation or capture failure. A fragment or
/// generic exit zero never establishes account state.
pub(super) fn opencode_login_success_frame(frame: &[u8]) -> bool {
    [
        b"o  Login successful\n".as_slice(),
        "◇  Login successful\n".as_bytes(),
        b"\x1b[32mo\x1b[39m  Login successful\n".as_slice(),
        "\x1b[32m◇\x1b[39m  Login successful\n".as_bytes(),
    ]
    .iter()
    .any(|suffix| frame.ends_with(suffix))
}

fn partial_frame_end(error: &ProcessCustodyError) -> bool {
    match error {
        ProcessCustodyError::ProtocolPipe(source) => matches!(
            source.kind(),
            std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
        ),
        ProcessCustodyError::ProtocolEvidence { cause, .. } => partial_frame_end(cause),
        _ => false,
    }
}

impl<'root> ProductDatabase<'root> {
    pub(super) fn cleanup_confirmed_provider_login_runtime(
        &mut self,
        instance_id: &str,
        driver: &str,
        expected_home: &RootIdentity,
        runtime: &Path,
        runtime_identity: &RootIdentity,
    ) -> Result<()> {
        let home = instance::provider_login::resolve_registered_login_home(
            &mut self.connection,
            self.root,
            &self.owner,
            instance_id,
            driver,
        )
        .map_err(|error| {
            OrchestrationError::V37StoreFailure(format!(
                "provider login cleanup registered home: {error:?}"
            ))
        })?;
        if &home.identity != expected_home || runtime.parent() != Some(home.path.as_path()) {
            return Err(OrchestrationError::AccessDenied);
        }
        // The original login Job and writers are durably stopped before this
        // path runs. Remove only the Windows-generated cache junction from F's
        // unchanged registered home; the target is never opened or traversed.
        login_cache::remove_generated_cache_junction(self.root, &home)?;
        remove_owned_runtime(runtime, runtime_identity)
    }

    /// Add the current unfinished bytes only to this response snapshot. The
    /// reader still owns them and a later complete LF frame replaces them.
    pub(super) fn provider_display_output(&self, active: &ActiveOwnerLogin) -> String {
        let mut output = active.output.clone();
        match self
            .process_custodian
            .persistent_stdout_fragment(&active.prepared.ticket)
        {
            Ok(fragment) if output.len().saturating_add(fragment.len()) <= 65_536 => {
                output.push_str(&String::from_utf8_lossy(&fragment));
            }
            Ok(_) => output.push_str("\nprovider stdout display limit"),
            Err(error) => output.push_str(&format!("\nprovider stdout fragment: {error:?}")),
        }
        output
    }

    /// Called after the exact Job/writer stop proof while custody is retained.
    /// Complete frames are consumed once; the retained no-LF tail is cloned
    /// once for final private display. Neither is account-state evidence.
    fn collect_provider_stopped_stdout(
        &self,
        prepared: &PreparedCustody,
        mut output: Vec<u8>,
    ) -> Result<(Vec<u8>, bool)> {
        let mut finished = false;
        let mut opencode_completion = false;
        for _ in 0..1024 {
            match self
                .process_custodian
                .poll_persistent_child_frame(&prepared.ticket)
            {
                Ok(Some(frame)) => {
                    if frame.custody() != prepared {
                        return Err(OrchestrationError::AccessDenied);
                    }
                    if output.len().saturating_add(frame.bytes().len()) > 65_536 {
                        return Err(OrchestrationError::Invalid("provider stdout limit"));
                    }
                    opencode_completion |= opencode_login_success_frame(frame.bytes());
                    output.extend_from_slice(frame.bytes());
                }
                Ok(None) => {
                    finished = true;
                    break;
                }
                Err(error) if partial_frame_end(&error) => {
                    finished = true;
                    break;
                }
                Err(error) => return Err(OrchestrationError::Process(error)),
            }
        }
        if !finished {
            return Err(OrchestrationError::Invalid("provider stdout frame limit"));
        }
        let fragment = self
            .process_custodian
            .persistent_stdout_fragment(&prepared.ticket)?;
        if output.len().saturating_add(fragment.len()) > 65_536 {
            return Err(OrchestrationError::Invalid("provider stdout limit"));
        }
        output.extend_from_slice(&fragment);
        Ok((output, opencode_completion))
    }

    pub(super) fn append_provider_final_stdout(&self, active: &mut ActiveOwnerLogin) -> Result<()> {
        let (bytes, completion) = self.collect_provider_stopped_stdout(&active.prepared, Vec::new())?;
        if active.output.len().saturating_add(bytes.len()) > 65_536 {
            return Err(OrchestrationError::Invalid("owner login output limit"));
        }
        if active.provider.as_ref().is_some_and(|provider| provider.driver_id == "opencode") {
            active.provider_completion_frame |= completion;
        }
        active.output.push_str(&String::from_utf8_lossy(&bytes));
        Ok(())
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
            credential_custody:None,
            login: provider.login.clone(),
            account_read: provider.login.clone(), // unused for this CLI
            runtime_home: runtime,
            runtime_identity,
            registered_driver: provider.driver_id.clone(),
            registered_home_identity: provider.home.identity.clone(),
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
        original_completion: bool,
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
            || fresh.browser != provider.browser
        {
            return Err(OrchestrationError::AccessDenied);
        }
        let state = if original_completion && fresh.driver_id == "opencode" {
            NativeAccountState::CredentialPresent
        } else { match &fresh.status {
            StatusObservation::Unknown(_) => NativeAccountState::Unknown,
            StatusObservation::Documented(status) => {
                let (bytes, exit, stderr) = self.observe_provider_status(command, &fresh, &status.request)?;
                let state = classify_status(&bytes, exit, status.logged_in_exit, status.logged_out_exit);
                if state == NativeAccountState::Unknown
                    && (stderr.len() > 0 || (exit != u32::try_from(status.logged_in_exit).ok()
                        && exit != u32::try_from(status.logged_out_exit).ok())) {
                    return Err(OrchestrationError::V37StoreFailure(format!(
                        "provider status CLI exit={exit:?}; STDERR_TAIL: {stderr}")));
                }
                state
            }
            StatusObservation::OpenCodeCredentialList(request) => {
                let (bytes, exit, stderr) = self.observe_provider_status(command, &fresh, request)?;
                let state = classify_opencode_credential_list(&bytes, exit);
                if state == NativeAccountState::Unknown && (!stderr.is_empty() || exit != Some(0)) {
                    return Err(OrchestrationError::V37StoreFailure(format!(
                        "provider status CLI exit={exit:?}; STDERR_TAIL: {stderr}")));
                }
                state
            }
            StatusObservation::GrokModelsAuthenticationHeading(request) => {
                let (bytes,exit,stderr)=self.observe_provider_status(command,&fresh,request)?;
                use instance::provider_login::GrokModelsAccountState;
                let state=match instance::provider_login::classify_grok_models_status(&bytes,exit) {
                    GrokModelsAccountState::CredentialPresent=>NativeAccountState::CredentialPresent,
                    GrokModelsAccountState::LoggedOut=>NativeAccountState::LoggedOut,
                    GrokModelsAccountState::Unknown=>NativeAccountState::Unknown,
                };
                if state==NativeAccountState::Unknown && (!stderr.is_empty() || exit!=Some(0)) {
                    return Err(OrchestrationError::V37StoreFailure(format!(
                        "provider status CLI exit={exit:?}; STDERR_TAIL: {stderr}")));
                }
                state
            }
        }};
        let source = if original_completion && fresh.driver_id == "opencode" {
            "owner-login-opencode-xai-cli-completion"
        } else { "owner-login-provider-status" };
        self.record_provider_state(command, &fresh, state, source)
    }

    fn observe_provider_status(
        &mut self,
        command: &OwnerLoginCommand,
        provider: &PreparedProviderLogin,
        request: &PrepareRequest,
    ) -> Result<(Vec<u8>, Option<u32>, String)> {
        let operation_id = status_operation_id(command);
        let prepared = match self.process_custodian.prepare(request) {
            Ok(prepared) => prepared,
            Err(error) => return Err(OrchestrationError::Process(error)),
        };
        let pending = |proof: Option<NativeStopProof>, abort_prepared| PendingAccountCustody {
            operation_id: Some(operation_id.clone()),
            prepared: Some(prepared.clone()),
            runtime_home: None,
            runtime_identity: None,
            registered_driver: None,
            registered_home_identity: None,
            proof,
            durable_revision: None,
            abort_prepared,
            released: false,
            frame: None,
            backend_source: None,
            credential_custody:None,
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
                    continuation: None,
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
                    continuation: None,
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
                continuation: None,
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
            Err(error) if partial_frame_end(&error) => Ok(Vec::new()),
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
                    continuation: None,
                }));
                return Err(OrchestrationError::Process(error));
            }
        };
        let stdout = match output {
            Ok(prefix) => self.collect_provider_stopped_stdout(&prepared, prefix).map(|(bytes, _)| bytes),
            Err(error) => Err(OrchestrationError::V37StoreFailure(error)),
        };
        let process = self.process_custodian.active(&prepared.ticket)
            .ok_or(OrchestrationError::AccessDenied)?;
        let stderr_drain = if proof.writer_fence_verified && proof.active_job_processes == Some(0) {
            process.drain_stderr_after_writers_stopped()
        } else { Err("provider status stderr writers not fenced".into()) };
        let stderr = process.stderr_tail();
        let status_diagnostic = format!(
            "CLI exit={:?}; STDERR_TAIL: {stderr}; stderr drain={:?}; wait={exited:?}; stdout capture={:?}",
            proof.exit_code, stderr_drain.as_ref().err(), stdout.as_ref().err());
        let revision =
            match authority::mark_process_stopped(&mut self.connection, &operation_id, &proof) {
                Ok(revision) => revision,
                Err(error) => {
                    let unknown = authority::mark_process_unknown(
                        &mut self.connection,
                        &operation_id,
                        &prepared,
                    );
                    let cause = OrchestrationError::V37StoreFailure(format!(
                        "provider status stop record: {error:?}; unknown record: {unknown:?}; {status_diagnostic}"));
                    self.owner_login =
                        Some(OwnerLoginSession::PendingAccount(PendingAccountRead {
                            instance_id: command.instance_id.clone(),
                            request_id: command.request_id.clone(),
                            expected_revision: command.expected_revision,
                            output: format!("{cause:?}"),
                            latest_error: None,
                            custody: pending(Some(proof), false),
                            continuation: None,
                        }));
                    return Err(cause);
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
            let cause = OrchestrationError::V37StoreFailure(format!(
                "provider status stop confirmation: {error:?}; {status_diagnostic}"));
            let mut custody = pending(Some(proof), false);
            custody.durable_revision = Some(revision);
            self.owner_login = Some(OwnerLoginSession::PendingAccount(PendingAccountRead {
                instance_id: command.instance_id.clone(),
                request_id: command.request_id.clone(),
                expected_revision: command.expected_revision,
                output: format!("{cause:?}"),
                latest_error: None,
                custody,
                continuation: None,
            }));
            return Err(cause);
        }
        // Status may run for an already logged-in instance without a new
        // login. Bind cleanup to its original F home only after this status
        // child's own Job/writer stop is durably confirmed.
        let cache_cleanup = (|| -> Result<()> {
            if provider.instance_id != command.instance_id {
                return Err(OrchestrationError::AccessDenied);
            }
            let home = instance::provider_login::resolve_registered_login_home(
                &mut self.connection, self.root, &self.owner,
                &provider.instance_id, &provider.driver_id,
            ).map_err(|error| OrchestrationError::V37StoreFailure(format!(
                "provider status cleanup registered home: {error:?}")))?;
            if home.path != provider.home.path || home.identity != provider.home.identity {
                return Err(OrchestrationError::AccessDenied);
            }
            login_cache::remove_generated_cache_junction(self.root, &home)
        })();
        if let Err(error) = cache_cleanup {
            return Err(OrchestrationError::V37StoreFailure(format!(
                "provider status cache cleanup: {error:?}; {status_diagnostic}")));
        }
        let bytes = stdout.map_err(|error| OrchestrationError::V37StoreFailure(format!(
            "provider status stdout: {error:?}; CLI exit={:?}; STDERR_TAIL: {stderr}",
            proof.exit_code)))?;
        if let Err(error) = stderr_drain {
            return Err(OrchestrationError::V37StoreFailure(format!(
                "provider status stderr drain: {error}; CLI exit={:?}; STDERR_TAIL: {stderr}",
                proof.exit_code)));
        }
        if !exited.map_err(|error| OrchestrationError::V37StoreFailure(format!(
            "{error}; CLI exit={:?}; STDERR_TAIL: {stderr}", proof.exit_code)))? {
            return Ok((bytes, None, stderr));
        }
        Ok((bytes, proof.exit_code, stderr))
    }

    fn record_provider_state(
        &mut self,
        command: &OwnerLoginCommand,
        provider: &PreparedProviderLogin,
        state: NativeAccountState,
        source: &str,
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
                "{source}:{}:{}", command.instance_id, command.request_id
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
    fn opencode_completion_requires_the_fixed_complete_spinner_line() {
        for line in [
            b"o  Login successful\n".as_slice(),
            "◇  Login successful\n".as_bytes(),
            b"\x1b[32mo\x1b[39m  Login successful\n".as_slice(),
            "\x1b[32m◇\x1b[39m  Login successful\n".as_bytes(),
        ] {
            assert!(opencode_login_success_frame(line));
        }
        for line in [
            b"o  Login successful".as_slice(),
            b"o  Login failed\n".as_slice(),
            b"Login successful\n".as_slice(),
            b"\x1b[32mo\x1b[39m  Login successful\r\n".as_slice(),
        ] {
            assert!(!opencode_login_success_frame(line));
        }
    }

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

    #[test]
    fn fixed_opencode_empty_inventory_is_logout_without_reading_auth_file() {
        let original = b"\x1b[90mT\x1b[39m  Credentials \x1b[90m~\\.local\\share\\opencode\\auth.json\n\x1b[90m|\x1b[39m\n\x1b[90m\xe2\x80\x94\x1b[39m  0 credentials\n\n";
        assert_eq!(
            classify_opencode_credential_list(original, Some(0)),
            NativeAccountState::LoggedOut
        );
        assert_eq!(
            classify_opencode_credential_list(original, Some(1)),
            NativeAccountState::Unknown
        );
        assert_eq!(
            classify_opencode_credential_list(b"OpenAI oauth\n1 credentials\n", Some(0)),
            NativeAccountState::Unknown
        );
        assert_eq!(
            classify_opencode_credential_list(
                b"T  Credentials isolated\n|\n|  xAI oauth\n\xe2\x80\x94  1 credentials\n",
                Some(0)
            ),
            NativeAccountState::Unknown
        );
        for output in [
            &"T  Credentials isolated\n|\n•  xAI oauth\n—  1 credentials\n".as_bytes()[..],
            &"T  Credentials isolated\n|\n●  xAI oauth\n—  1 credentials\n".as_bytes()[..],
            &"T  Credentials isolated\n|\n\x1b[34m•\x1b[39m  xAI oauth\n—  1 credentials\n".as_bytes()[..],
        ] {
            assert_eq!(
                classify_opencode_credential_list(output, Some(0)),
                NativeAccountState::CredentialPresent
            );
        }
        assert_eq!(
            classify_opencode_credential_list(
                b"T  Credentials isolated\n|\n|  OpenAI oauth\n\xe2\x80\x94  0 credentials\n",
                Some(0)
            ),
            NativeAccountState::Unknown
        );
        for output in [
            &b"T  Credentials isolated\n|\n|  xAI api\n\xe2\x80\x94  1 credentials\n"[..],
            &b"T  Credentials isolated\n|\n|  xAI oauth\n|  xAI oauth\n\xe2\x80\x94  2 credentials\n"[..],
            &b"T  Credentials isolated\n|\n|  xAI oauth\n\xe2\x80\x94  2 credentials\n"[..],
        ] {
            assert_eq!(classify_opencode_credential_list(output, Some(0)), NativeAccountState::Unknown);
        }
    }
}
