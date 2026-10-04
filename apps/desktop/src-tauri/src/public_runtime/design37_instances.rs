//! Owner instance page. The host owns request identities and drives login even
//! when no WebView is mounted. Only the vendor CLI touches its credentials.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use tauri_plugin_opener::OpenerExt;

const AUTHORIZATION_URL: &str = "https://auth.openai.com/codex/device";
type ReplyFuture<'a> = Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;

trait LoginEnvironment: Send + Sync + 'static {
    fn request(&self, frame: String) -> ReplyFuture<'_>;
    fn open_authorization(&self, url: &str) -> Result<(), String>;
}
struct InstalledEnvironment(tauri::AppHandle);
impl LoginEnvironment for InstalledEnvironment {
    fn request(&self, frame: String) -> ReplyFuture<'_> {
        Box::pin(super::product_entry::gogoke_design37_user_operation(self.0.clone(), frame))
    }
    fn open_authorization(&self, url: &str) -> Result<(), String> {
        self.0.opener().open_url(url, None::<&str>)
            .map_err(|error| format!("GOGOKE_INSTANCE_AUTHORIZATION_OPEN_FAILED:{error}"))
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginView {
    request_id: String,
    expected_revision: u64,
    state: String,
    output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    browser_state: String,
    started_at: u64,
    settled: bool,
}
#[derive(Clone)]
struct LoginRecord { view: LoginView, driver_id: String, cancel_requested: bool, settled: bool }
type Sessions = Arc<Mutex<BTreeMap<String, LoginRecord>>>;
fn sessions() -> &'static Sessions {
    static SESSIONS: OnceLock<Sessions> = OnceLock::new();
    SESSIONS.get_or_init(|| Arc::new(Mutex::new(BTreeMap::new())))
}
fn lock_sessions(sessions: &Sessions) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, LoginRecord>>, String> {
    sessions.lock().map_err(|error| format!("GOGOKE_INSTANCE_SESSIONS_LOCK_FAILED:{error}"))
}
fn same_record<'a>(map: &'a mut BTreeMap<String, LoginRecord>, id: &str, request: &str) -> Result<&'a mut LoginRecord, String> {
    let record = map.get_mut(id).ok_or("GOGOKE_INSTANCE_LOGIN_RECORD_MISSING")?;
    if record.view.request_id != request { return Err("GOGOKE_INSTANCE_LOGIN_IDENTITY_CHANGED".into()); }
    Ok(record)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeInstance {
    instance_id: String, driver_id: String, version: String, revision: String,
    install_state: String, login_state: String,
    #[serde(default)]
    new_version: Option<String>,
    #[serde(default)]
    runtime_issues: Vec<RuntimeIssue>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeIssue {
    seat_id: String, session_id: String, generation: String, reason: String,
    source_epoch: String, source_cursor: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeInstances { schema: String, instances: Vec<NativeInstance> }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstanceView {
    instance_id: String, driver_id: String, version: String, revision: String, state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_version: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    runtime_issues: Vec<RuntimeIssue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    login: Option<LoginView>,
}
#[derive(Serialize)]
pub(crate) struct InstancePage { schema: &'static str, instances: Vec<InstanceView> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginReply { schema: String, instance_id: String, request_id: String, state: String, output: String, settled: bool }

fn valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (1..=64).contains(&bytes.len()) && bytes[0].is_ascii_alphabetic()
        && bytes[1..].iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
}
fn login_frame(id: &str, view: &LoginView, action: &str) -> String {
    serde_json::json!({"schema":"gogoke.37.owner-login.v1", "action":action,
        "instanceId":id,"requestId":view.request_id,"expectedRevision":view.expected_revision}).to_string()
}
fn decode_login(raw: &str, id: &str, request: &str) -> Result<LoginReply, String> {
    let reply: LoginReply = serde_json::from_str(raw)
        .map_err(|error| format!("GOGOKE_INSTANCE_LOGIN_REPLY_DECODE_FAILED:{error}"))?;
    if reply.schema != "gogoke.37.owner-login.v1" || reply.instance_id != id || reply.request_id != request
        || !["PENDING","LOGGED_IN","LOGGED_OUT","UNKNOWN"].contains(&reply.state.as_str()) {
        return Err("GOGOKE_INSTANCE_LOGIN_REPLY_IDENTITY_MISMATCH".into());
    }
    Ok(reply)
}

/// Rendering removes terminal controls only. The original CLI failure remains
/// in the Owner-private error field; no progress or device code enters logs.
fn display_text(raw: &str) -> String {
    let mut output = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.next() {
                Some('[') => { for c in chars.by_ref() { if ('@'..='~').contains(&c) { break; } } }
                Some(']') => { let mut escaped = false; for c in chars.by_ref() {
                    if c == '\u{7}' || (escaped && c == '\\') { break; } escaped = c == '\u{1b}';
                } }
                _ => {}
            }
        } else if !ch.is_control() || ch == '\n' || ch == '\t' { output.push(ch); }
    }
    output
}
fn device_code(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|word| {
        let (a,b) = word.split_once('-')?;
        if (3..=8).contains(&a.len()) && (3..=8).contains(&b.len())
            && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
            Some(word.to_owned())
        } else { None }
    })
}
// The fixed CLI prints its ordinary OAuth URL as a complete stderr line.
// Follow Room's login pattern: use the URL from this login process. A partial
// line must never open a syntactically valid but incomplete OAuth request.
fn authorization_url(text: &str, driver_id: &str) -> Option<String> {
    // Browser ownership follows the original registered fixed CLI, never a
    // URL in generic output. Claude/Grok own their browser launch. OpenCode's
    // builtin OpenAI browser flow prints its URL but does not open a browser.
    if !matches!(driver_id,"codex"|"opencode") { return None; }
    text.split_inclusive('\n').filter(|line| line.ends_with('\n')).find_map(|line| {
        let candidate = if driver_id=="opencode" {
            line.trim().trim_start_matches(|ch:char|matches!(ch,'│'|' '| '\t'))
                .strip_prefix("Go to: ")?.trim()
        } else { line.trim() };
        let url = reqwest::Url::parse(candidate).ok()?;
        if url.scheme() != "https" || url.host_str() != Some("auth.openai.com")
            || !url.username().is_empty() || url.password().is_some()
            || url.port().is_some_and(|port| port != 443) || url.fragment().is_some()
        {
            return None;
        }
        if url.path() == "/oauth/authorize" || (driver_id=="codex" && candidate == AUTHORIZATION_URL) {
            Some(candidate.to_owned())
        } else {
            None
        }
    })
}
fn apply_reply(sessions: &Sessions, id: &str, request: &str, action: &str, reply: LoginReply) -> Result<bool, String> {
    let mut map = lock_sessions(sessions)?;
    let record = same_record(&mut map, id, request)?;
    record.view.output = display_text(&reply.output);
    if record.view.authorization_url.is_none() {
        record.view.authorization_url = authorization_url(&record.view.output,&record.driver_id);
    }
    record.view.device_code = if record.view.authorization_url.as_deref()==Some(AUTHORIZATION_URL) {
        device_code(&record.view.output)
    } else { None };
    record.settled = reply.settled;
    record.view.settled = reply.settled;
    record.view.state = if action == "cancel" && reply.settled && reply.state == "LOGGED_OUT" {
        "CANCELLED".into()
    } else { reply.state };
    if record.settled && matches!(record.view.state.as_str(), "LOGGED_IN" | "CANCELLED") {
        record.view.error = None;
    }
    if record.view.state == "UNKNOWN" {
        record.view.state = "ERROR".into();
        if record.view.error.is_none() {
            record.view.error = Some(if let Some(pos) = record.view.output.find("owner login process exited:") {
                record.view.output[pos..].to_owned()
            } else { "CLI 已结束，但登录状态无法确认。".into() });
        }
    }
    Ok(!record.settled)
}
fn save_error(sessions: &Sessions, id: &str, request: &str, error: String) -> Result<(), String> {
    let mut map = lock_sessions(sessions)?;
    let record = same_record(&mut map, id, request)?;
    record.view.state = "ERROR".into();
    if record.view.error.is_none() { record.view.error = Some(error); }
    Ok(())
}

fn reserve_login(sessions: &Sessions, id: &str, revision: u64, driver_id: &str) -> Result<Option<LoginView>, String> {
    let mut map = lock_sessions(sessions)?;
    if let Some(record) = map.get(id) {
        if record.view.state == "PENDING" { return Ok(None); }
        if !record.settled { return Err("GOGOKE_INSTANCE_ORIGINAL_LOGIN_UNCONFIRMED".into()); }
    }
    if map.values().any(|r| r.view.state == "PENDING" || !r.settled) {
        return Err("GOGOKE_INSTANCE_OTHER_LOGIN_ACTIVE".into());
    }
    let view = LoginView { request_id:format!("owner_{}", uuid::Uuid::new_v4().simple()),
        expected_revision:revision, state:"PENDING".into(), output:String::new(),
        authorization_url:None, device_code:None, error:None, browser_state:"NOT_REQUESTED".into(),
        started_at:std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!("GOGOKE_INSTANCE_CLOCK_FAILED:{error}"))?.as_millis() as u64,
        settled:false };
    map.insert(id.to_owned(), LoginRecord { view:view.clone(), driver_id:driver_id.to_owned(), cancel_requested:false, settled:false });
    Ok(Some(view))
}

async fn drive<E: LoginEnvironment>(environment: Arc<E>, sessions: Sessions, id: String, original: LoginView) -> Result<(), String> {
    let mut action = "begin";
    loop {
        if action != "begin" && same_record(&mut *lock_sessions(&sessions)?, &id, &original.request_id)?.cancel_requested {
            action = "cancel";
        }
        let result = environment.request(login_frame(&id, &original, action)).await
            .and_then(|raw| decode_login(&raw, &id, &original.request_id));
        let reply = match result {
            Ok(reply) => reply,
            Err(error) => {
                save_error(&sessions, &id, &original.request_id, error)?;
                // Keep observing custody of this original request. Never replay
                // begin or replace its identity after a transport/operation error.
                action = "status";
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                continue;
            }
        };
        let pending = apply_reply(&sessions, &id, &original.request_id, action, reply)?;
        let should_open = {
            let mut map = lock_sessions(&sessions)?;
            let record = same_record(&mut map, &id, &original.request_id)?;
            if pending && matches!(record.driver_id.as_str(),"codex"|"opencode") && record.view.authorization_url.is_some()
                && record.view.browser_state == "NOT_REQUESTED" && !record.cancel_requested {
                record.view.browser_state = "OPENED".into(); record.view.authorization_url.clone()
            } else { None }
        };
        if let Some(url) = should_open {
            if let Err(error) = environment.open_authorization(&url) {
                let mut map = lock_sessions(&sessions)?;
                let record = same_record(&mut map, &id, &original.request_id)?;
                record.view.browser_state = "FAILED".into(); record.view.error = Some(error);
            }
        }
        if !pending { return Ok(()); }
        action = "status";
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

async fn page<E: LoginEnvironment>(environment: &E, sessions: &Sessions) -> Result<InstancePage, String> {
    let raw = environment.request(r#"{"schema":"gogoke.37.instance-list.v1"}"#.into()).await?;
    let native: NativeInstances = serde_json::from_str(&raw)
        .map_err(|error| format!("GOGOKE_INSTANCE_LIST_DECODE_FAILED:{error}"))?;
    if native.schema != "gogoke.37.instance-list.v1" || native.instances.len() > 1024 { return Err("GOGOKE_INSTANCE_LIST_SCHEMA_INVALID".into()); }
    let map = lock_sessions(sessions)?;
    let mut instances = Vec::with_capacity(native.instances.len());
    for item in native.instances {
        if !valid_id(&item.instance_id) || item.revision.parse::<u64>().ok().filter(|r| *r > 0).map(|r| r.to_string()).as_deref() != Some(item.revision.as_str()) {
            return Err("GOGOKE_INSTANCE_LIST_IDENTITY_INVALID".into());
        }
        if item.runtime_issues.iter().any(|issue| [
            &issue.seat_id, &issue.session_id, &issue.generation, &issue.reason,
            &issue.source_epoch, &issue.source_cursor,
        ].iter().any(|field| field.trim().is_empty())) {
            return Err("GOGOKE_INSTANCE_RUNTIME_ISSUE_INVALID".into());
        }
        let login = map.get(&item.instance_id).map(|r| r.view.clone());
        let state = match (item.install_state.as_str(), item.login_state.as_str(), login.as_ref().map(|l| l.state.as_str())) {
            ("MISSING",_,_) => "NOT_INSTALLED",
            (_,_,Some("ERROR" | "UNKNOWN")) => "ERROR",
            ("INSTALLED",_,Some("PENDING" | "CANCELLED")) => "NOT_LOGGED_IN",
            ("INSTALLED","LOGGED_IN",_) => "LOGGED_IN",
            ("INSTALLED","LOGGED_OUT",_) => "NOT_LOGGED_IN",
            _ => "ERROR",
        };
        instances.push(InstanceView { instance_id:item.instance_id, driver_id:item.driver_id,
            version:item.version, revision:item.revision, state:state.into(),
            new_version:item.new_version, runtime_issues:item.runtime_issues, login });
    }
    Ok(InstancePage { schema:"gogoke.37.instance-page.v1", instances })
}

#[tauri::command]
pub(crate) async fn gogoke_design37_instances(app: tauri::AppHandle) -> Result<InstancePage, String> {
    super::product_entry::ensure_design37_user_host(&app).await?;
    page(&InstalledEnvironment(app), sessions()).await
}
#[tauri::command]
pub(crate) async fn gogoke_design37_instance_register(app: tauri::AppHandle, instance_id: String) -> Result<InstancePage, String> {
    if !valid_id(&instance_id) { return Err("GOGOKE_INSTANCE_ID_INVALID".into()); }
    super::product_entry::gogoke_design37_register_codex_instance(app.clone(),
        super::product_entry::Design37RegisterCodexRequest::for_instance(instance_id)).await?.require_applied()?;
    gogoke_design37_instances(app).await
}
#[tauri::command]
pub(crate) async fn gogoke_design37_instance_login(app: tauri::AppHandle, instance_id: String) -> Result<InstancePage, String> {
    if !valid_id(&instance_id) { return Err("GOGOKE_INSTANCE_ID_INVALID".into()); }
    let current = gogoke_design37_instances(app.clone()).await?;
    let item = current.instances.iter().find(|i| i.instance_id == instance_id).ok_or("GOGOKE_INSTANCE_NOT_REGISTERED")?;
    let revision = item.revision.parse().map_err(|error| format!("GOGOKE_INSTANCE_REVISION_INVALID:{error}"))?;
    let Some(view) = reserve_login(sessions(), &instance_id, revision, &item.driver_id)? else { return Ok(current) };
    let environment = Arc::new(InstalledEnvironment(app.clone()));
    let records = Arc::clone(sessions());
    tauri::async_runtime::spawn(async move {
        if let Err(error) = drive(environment, Arc::clone(&records), instance_id.clone(), view.clone()).await {
            if let Err(store_error) = save_error(&records, &instance_id, &view.request_id, error) {
                eprintln!("GOGOKE_INSTANCE_LOGIN_RESULT_STORE_FAILED:{store_error}");
            }
        }
    });
    gogoke_design37_instances(app).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct FakeEnvironment {
        replies: Mutex<VecDeque<Result<(&'static str, &'static str, bool), String>>>,
        actions: Mutex<Vec<serde_json::Value>>,
        opened: AtomicUsize,
        opened_urls: Mutex<Vec<String>>,
        logged_in: AtomicBool,
    }
    impl LoginEnvironment for FakeEnvironment {
        fn request(&self, frame: String) -> ReplyFuture<'_> {
            Box::pin(async move {
                let request: serde_json::Value = serde_json::from_str(&frame).unwrap();
                if request["schema"] == "gogoke.37.instance-list.v1" {
                    return Ok(serde_json::json!({"schema":"gogoke.37.instance-list.v1","instances":[{
                        "instanceId":"instanceA","driverId":"codex","version":"0.149.0","revision":"2",
                        "installState":"INSTALLED","loginState":if self.logged_in.load(Ordering::SeqCst) {"LOGGED_IN"} else {"LOGGED_OUT"}
                    }]}).to_string());
                }
                self.actions.lock().unwrap().push(request.clone());
                let (state, output, settled) = self.replies.lock().unwrap().pop_front().expect("unexpected new request")?;
                if state == "LOGGED_IN" { self.logged_in.store(true, Ordering::SeqCst); }
                Ok(serde_json::json!({"schema":"gogoke.37.owner-login.v1","instanceId":"instanceA",
                    "requestId":request["requestId"],"state":state,"output":output,"settled":settled}).to_string())
            })
        }
        fn open_authorization(&self, url: &str) -> Result<(), String> {
            self.opened_urls.lock().unwrap().push(url.to_owned());
            self.opened.fetch_add(1, Ordering::SeqCst); Ok(())
        }
    }
    fn fake(replies: Vec<Result<(&'static str, &'static str, bool), String>>) -> Arc<FakeEnvironment> {
        Arc::new(FakeEnvironment { replies:Mutex::new(replies.into()), actions:Mutex::new(Vec::new()),
            opened:AtomicUsize::new(0), opened_urls:Mutex::new(Vec::new()), logged_in:AtomicBool::new(false) })
    }
    fn runtime() -> tokio::runtime::Runtime { tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap() }
    fn broker() -> Sessions { Arc::new(Mutex::new(BTreeMap::new())) }

    #[test]
    fn host_drives_login_without_ui_then_same_session_result_survives_new_reads() {
        runtime().block_on(async {
            let sessions = broker();
            let original = reserve_login(&sessions,"instanceA",2,"codex").unwrap().unwrap();
            assert!(reserve_login(&sessions,"instanceA",2,"codex").unwrap().is_none());
            let url = "https://auth.openai.com/oauth/authorize?client_id=test-client&state=test-state&code_challenge=test-challenge&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback";
            let environment = fake(vec![Ok(("PENDING", "https://auth.openai.com/oauth/authorize?client_id=test-client&state=test-state&code_challenge=test-challenge&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback\n", false)),
                Ok(("LOGGED_IN", "Successfully logged in", true))]);
            // Production driver receives no frontend object or poll operation.
            drive(Arc::clone(&environment), Arc::clone(&sessions), "instanceA".into(), original.clone()).await.unwrap();
            for _ in 0..2 {
                let reopened = page(environment.as_ref(), &sessions).await.unwrap();
                assert_eq!(reopened.instances[0].state, "LOGGED_IN");
                assert_eq!(reopened.instances[0].login.as_ref().unwrap().request_id, original.request_id);
            }
            assert_eq!(environment.opened.load(Ordering::SeqCst), 1);
            assert_eq!(environment.opened_urls.lock().unwrap().as_slice(), &[url.to_owned()]);
            let actions = environment.actions.lock().unwrap();
            assert_eq!(actions.len(), 2);
            assert_eq!(actions[0]["action"], "begin"); assert_eq!(actions[1]["action"], "status");
            assert_eq!(actions[0]["requestId"], actions[1]["requestId"]);
        });
    }
    #[test]
    fn original_failure_is_kept_and_settled_readback_does_not_restart_login() {
        runtime().block_on(async {
            let sessions = broker(); let original = reserve_login(&sessions,"instanceA",2,"codex").unwrap().unwrap();
            let reason = "original CLI exit2; STDERR_TAIL: unrecognized argument";
            let environment = fake(vec![Err(reason.into()), Ok(("UNKNOWN",reason,true))]);
            drive(Arc::clone(&environment),Arc::clone(&sessions),"instanceA".into(),original.clone()).await.unwrap();
            for _ in 0..2 {
                let view = page(environment.as_ref(),&sessions).await.unwrap();
                assert_eq!(view.instances[0].state,"ERROR");
                assert_eq!(view.instances[0].login.as_ref().unwrap().error.as_deref(),Some(reason));
                assert_eq!(view.instances[0].login.as_ref().unwrap().request_id,original.request_id);
            }
            assert_eq!(environment.actions.lock().unwrap().len(),2);
            assert!(reserve_login(&sessions,"instanceA",2,"codex").unwrap().is_some());
        });
    }
    #[test]
    fn uncertain_original_failure_keeps_observing_the_same_request_until_settled() {
        runtime().block_on(async {
            let sessions=broker(); let original=reserve_login(&sessions,"instanceA",2,"codex").unwrap().unwrap();
            save_error(&sessions,"instanceA",&original.request_id,"pipe closed".into()).unwrap();
            assert!(reserve_login(&sessions,"instanceA",2,"codex").err().unwrap().contains("UNCONFIRMED"));
            let environment=fake(vec![Err("pipe closed".into()),Err("original status unavailable".into()),Ok(("UNKNOWN","original cause",true))]);
            drive(Arc::clone(&environment),Arc::clone(&sessions),"instanceA".into(),original.clone()).await.unwrap();
            assert_eq!(sessions.lock().unwrap()["instanceA"].view.request_id,original.request_id);
            assert_eq!(sessions.lock().unwrap()["instanceA"].view.error.as_deref(),Some("pipe closed"));
            assert_eq!(environment.actions.lock().unwrap().len(),3);
            assert!(reserve_login(&sessions,"instanceA",2,"codex").unwrap().is_some());
        });
    }
    #[test]
    fn cancellation_intent_cannot_overwrite_a_successful_original_result() {
        let sessions=broker(); let original=reserve_login(&sessions,"instanceA",2,"codex").unwrap().unwrap();
        sessions.lock().unwrap().get_mut("instanceA").unwrap().cancel_requested=true;
        for action in ["status","cancel"] {
            save_error(&sessions,"instanceA",&original.request_id,"earlier transport failure".into()).unwrap();
            assert!(!apply_reply(&sessions,"instanceA",&original.request_id,action,LoginReply {
                schema:"gogoke.37.owner-login.v1".into(),instance_id:"instanceA".into(),
                request_id:original.request_id.clone(),state:"LOGGED_IN".into(),output:String::new(),settled:true
            }).unwrap());
            assert_eq!(sessions.lock().unwrap()["instanceA"].view.state,"LOGGED_IN");
            assert!(sessions.lock().unwrap()["instanceA"].view.error.is_none());
        }
    }
    #[test]
    fn cancel_uses_the_original_request_and_survives_a_new_page_read() {
        runtime().block_on(async {
            let sessions=broker(); let original=reserve_login(&sessions,"instanceA",2,"codex").unwrap().unwrap();
            sessions.lock().unwrap().get_mut("instanceA").unwrap().cancel_requested=true;
            save_error(&sessions,"instanceA",&original.request_id,"prior stop observation failed".into()).unwrap();
            let environment=fake(vec![Ok(("PENDING","",false)),Ok(("LOGGED_OUT","",true))]);
            drive(Arc::clone(&environment),Arc::clone(&sessions),"instanceA".into(),original.clone()).await.unwrap();
            let actions=environment.actions.lock().unwrap();
            assert_eq!(actions[1]["action"],"cancel");assert_eq!(actions[1]["requestId"],original.request_id);
            drop(actions);
            assert_eq!(page(environment.as_ref(),&sessions).await.unwrap().instances[0].login.as_ref().unwrap().state,"CANCELLED");
            assert!(sessions.lock().unwrap()["instanceA"].view.error.is_none());
            assert!(reserve_login(&sessions,"instanceA",2,"codex").unwrap().is_some(),
                "confirmed cancellation must permit a new host-owned request");
        });
    }
    #[test]
    fn ordinary_oauth_waits_for_complete_url_and_opens_the_original_once() {
        runtime().block_on(async {
            let sessions = broker();
            let original = reserve_login(&sessions,"instanceA",2,"codex").unwrap().unwrap();
            let url = "https://auth.openai.com/oauth/authorize?state=test-state&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback";
            let environment = fake(vec![
                Ok(("PENDING", "Starting local login server on http://localhost:1455.\nhttps://auth.openai.com/oauth/authorize?state=test-", false)),
                Ok(("PENDING", "https://auth.openai.com/oauth/authorize?state=test-state&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback\n", false)),
                Ok(("PENDING", "https://auth.openai.com/oauth/authorize?state=test-state&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback\n", false)),
                Ok(("LOGGED_IN", "Successfully logged in", true)),
            ]);
            assert!(authorization_url("https://auth.openai.com/oauth/authorize?state=test-","codex").is_none());
            drive(Arc::clone(&environment), Arc::clone(&sessions), "instanceA".into(), original.clone()).await.unwrap();
            assert_eq!(environment.opened_urls.lock().unwrap().as_slice(), &[url.to_owned()]);
            let reopened = page(environment.as_ref(), &sessions).await.unwrap();
            let login = reopened.instances[0].login.as_ref().unwrap();
            assert_eq!(login.authorization_url.as_deref(), Some(url));
            assert_eq!(login.request_id, original.request_id);
            assert!(login.settled);
        });
    }
    #[test]
    fn only_exact_cli_authorization_is_eligible_and_terminal_controls_are_removed() {
        assert_eq!(display_text("\u{1b}[94mABCD-EFGHI\u{1b}[0m\r\n"),"ABCD-EFGHI\n");
        assert_eq!(device_code("device-code\nABCD-EFGHI").as_deref(),Some("ABCD-EFGHI"));
        for invalid in [
            "https://auth.openai.com.example/oauth/authorize?state=test\n",
            "https://auth.openai.com@other.example/oauth/authorize?state=test\n",
            "https://other.example@auth.openai.com/oauth/authorize?state=test\n",
            "http://auth.openai.com/oauth/authorize?state=test\n",
            "https://auth.openai.com:444/oauth/authorize?state=test\n",
            "https://auth.openai.com/oauth/authorize?state=test#fragment\n",
            "https://auth.openai.com/docs\n",
        ] {
            assert!(authorization_url(invalid,"codex").is_none(), "must not open: {invalid}");
        }
        assert_eq!(authorization_url(&format!("{AUTHORIZATION_URL}\n"),"codex").as_deref(), Some(AUTHORIZATION_URL));
        let oauth="https://auth.openai.com/oauth/authorize?state=original-state";
        assert_eq!(authorization_url(&format!("│  Go to: {oauth}\n"),"opencode").as_deref(),Some(oauth));
        assert!(authorization_url(&format!("│  Go to: {oauth}"),"opencode").is_none());
        assert!(authorization_url(&format!("{oauth}\n"),"opencode").is_none());
        for driver in ["claude","grok","antigravity","unknown"] {
            assert!(authorization_url(&format!("{oauth}\n"),driver).is_none());
            assert!(authorization_url(&format!("│  Go to: {oauth}\n"),driver).is_none());
        }
        assert!(decode_login(r#"{"schema":"gogoke.37.owner-login.v1","instanceId":"other","requestId":"r","state":"PENDING","output":"","settled":false}"#,"instanceA","r").is_err());
    }
}
#[tauri::command]
pub(crate) async fn gogoke_design37_instance_cancel(app: tauri::AppHandle, instance_id: String) -> Result<InstancePage, String> {
    {
        let mut map = lock_sessions(sessions())?;
        let record = map.get_mut(&instance_id).ok_or("GOGOKE_INSTANCE_LOGIN_NOT_STARTED")?;
        if !record.settled { record.cancel_requested = true; }
    }
    gogoke_design37_instances(app).await
}
