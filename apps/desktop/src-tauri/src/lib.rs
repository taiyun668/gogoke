#[cfg(desktop)]
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
#[cfg(desktop)]
use tauri::RunEvent;
#[cfg(target_os = "macos")]
use tauri::WindowEvent;

mod backend;
mod codex;
mod daemon_binary;
mod dictation;
mod event_sink;
mod files;
mod git;
mod git_utils;
mod gogoke_update;
#[cfg(target_os = "windows")]
mod gogoke_uninstall;
mod local_usage;
#[cfg(desktop)]
mod menu;
#[cfg(not(desktop))]
#[path = "menu_mobile.rs"]
mod menu;
mod notifications;
mod prompts;
pub mod public_core;
mod public_runtime;
mod release_policy;
#[cfg(target_os = "windows")]
mod resource_trust;
mod remote_backend;
mod rules;
mod settings;
mod shared;
mod state;
mod storage;
mod tailscale;
#[cfg(desktop)]
mod terminal;
#[cfg(not(desktop))]
#[path = "terminal_mobile.rs"]
mod terminal;
mod tray;
mod types;
mod utils;
mod window;
mod workspaces;

#[cfg(desktop)]
static EXIT_CLEANUP_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

#[cfg(desktop)]
fn keep_daemon_running_after_close(app_handle: &tauri::AppHandle) -> bool {
    let state = app_handle.state::<state::AppState>();
    tauri::async_runtime::block_on(async {
        state
            .app_settings
            .lock()
            .await
            .keep_daemon_running_after_app_close
    })
}

#[cfg(desktop)]
async fn stop_managed_daemons_for_exit(app_handle: tauri::AppHandle) {
    let state = app_handle.state::<state::AppState>();
    let _ = tailscale::tailscale_daemon_stop(state).await;
}

#[tauri::command]
fn is_mobile_runtime() -> bool {
    cfg!(any(target_os = "ios", target_os = "android"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "windows")]
    {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        if arguments.iter().any(|argument| argument == "--gogoke-update-owned-inventory") {
            let result = if arguments.len() == 1 {
                gogoke_uninstall::write_update_owned_inventory()
            } else {
                Err("GOGOKE_UPDATE_INVENTORY_ARGUMENTS_INVALID".to_string())
            };
            if let Err(error) = &result { eprintln!("{error}"); }
            std::process::exit(if result.is_ok() { 0 } else { 1 });
        }
        if arguments.iter().any(|argument| argument == "--gogoke-install-shortcut") {
            let result = gogoke_uninstall::install_shortcut(&arguments);
            if let Err(error) = &result { eprintln!("{error}"); }
            std::process::exit(if result.is_ok() { 0 } else { 1 });
        }
        if arguments.iter().any(|argument| argument == "--uninstall") {
            if !(arguments.len() == 1 && arguments[0] == "--uninstall"
                || arguments.len() == 2 && arguments[0] == "--uninstall" && arguments[1] == "--quiet") {
                eprintln!("GOGOKE_UNINSTALL_ARGUMENTS_INVALID");
                std::process::exit(1);
            }
            let result = gogoke_uninstall::run();
            if let Err(error) = &result { eprintln!("{error}"); }
            std::process::exit(if result.is_ok() { 0 } else { 1 });
        }
    }

    #[cfg(target_os = "windows")]
    if let Some(path) = std::env::args().find_map(|argument| {
        argument
            .strip_prefix("--gogoke-install-resources=")
            .map(std::path::PathBuf::from)
    }) {
        let result = resource_trust::install_resources(&path);
        if let Err(error) = &result { eprintln!("{error}"); }
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }

    #[cfg(target_os = "windows")]
    if std::env::args().any(|argument| {
        argument.starts_with("--gogoke-verify-install-set=")
            || argument.starts_with("--gogoke-install-target=")
    }) {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        let result = if arguments.len() == 1 {
            match arguments[0].strip_prefix("--gogoke-verify-install-set=") {
                Some(source) => resource_trust::verify_install_set(std::path::Path::new(source)),
                None => Err("GOGOKE_INSTALL_PREFLIGHT_ARGUMENTS_INVALID".to_string()),
            }
        } else if arguments.len() == 2 {
            match (
                arguments[0].strip_prefix("--gogoke-verify-install-set="),
                arguments[1].strip_prefix("--gogoke-install-target="),
            ) {
                (Some(source), Some(target)) => resource_trust::verify_install_target(
                    std::path::Path::new(source),
                    std::path::Path::new(target),
                ),
                _ => Err("GOGOKE_INSTALL_PREFLIGHT_ARGUMENTS_INVALID".to_string()),
            }
        } else {
            Err("GOGOKE_INSTALL_PREFLIGHT_ARGUMENTS_INVALID".to_string())
        };
        if let Err(error) = &result { eprintln!("{error}"); }
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }

    #[cfg(target_os = "windows")]
    let verified_resources = if cfg!(debug_assertions) {
        None
    } else {
        match resource_trust::verify_bootstrap() {
            Ok(resources) => Some(resource_trust::ResourceState::new(resources)),
            Err(error) => { eprintln!("{error}"); return; }
        }
    };
    let mut context = tauri::generate_context!();
    #[cfg(target_os = "windows")]
    let verified_main_config = if let Some(resources) = &verified_resources {
        let config = context.config().app.windows.iter()
            .find(|window| window.label == "main")
            .cloned()
            .expect("gogoke main window config is required");
        context.config_mut().app.windows.clear();
        if resources.current().expect("verified resource state").domain == resource_trust::Domain::Candidate {
            context.config_mut().identifier = "app.gogoke.desktop.candidate".to_string();
        }
        Some(config)
    } else { None };

    #[cfg(target_os = "linux")]
    {
        // Avoid WebKit compositing issues on NVIDIA Linux setups (GBM buffer errors).
        if std::env::var_os("__NV_PRIME_RENDER_OFFLOAD").is_none() {
            std::env::set_var("__NV_PRIME_RENDER_OFFLOAD", "1");
        }
        let is_wayland = std::env::var("XDG_SESSION_TYPE")
            .map(|session| session.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false)
            || std::env::var_os("WAYLAND_DISPLAY").is_some();
        let has_nvidia = std::path::Path::new("/proc/driver/nvidia/version").exists();
        if is_wayland && has_nvidia && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
        {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        let is_x11 = !is_wayland && std::env::var_os("DISPLAY").is_some();
        // Work around sporadic blank WebKitGTK renders on X11 by disabling compositing mode.
        // Keep Wayland untouched because this can interfere with input behavior on some setups.
        if is_x11 && std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
    }

    #[cfg(desktop)]
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(menu::MenuItemRegistry::<tauri::Wry>::default())
        .manage(tray::TrayState::default())
        .on_menu_event(menu::handle_menu_event)
        .enable_macos_default_menu(false)
        .menu(menu::build_menu);

    #[cfg(not(desktop))]
    let builder = tauri::Builder::default();

    #[cfg(target_os = "windows")]
    let builder = {
        let protocol_resources = verified_resources.clone();
        builder.register_uri_scheme_protocol("gogoke-resource", move |_context, request| {
            let path = request.uri().path();
            let response = protocol_resources.as_ref()
                .ok_or_else(|| "GOGOKE_RESOURCE_DOMAIN_UNAVAILABLE".to_string())
                .and_then(|resources| resources.read_frontend_request(path));
            match response {
                Ok((bytes, content_type)) => tauri::http::Response::builder()
                    .status(tauri::http::StatusCode::OK)
                    .header("Content-Type", content_type)
                    .header("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' http://127.0.0.1:* ws://127.0.0.1:*; object-src 'none'; base-uri 'self'")
                    .body(bytes)
                    .expect("fixed resource response headers"),
                Err(error) => {
                    eprintln!("{error}");
                    tauri::http::Response::builder()
                        .status(tauri::http::StatusCode::NOT_FOUND)
                        .body(Vec::new())
                        .expect("fixed resource error response")
                }
            }
        })
    };

    #[cfg(target_os = "windows")]
    let resource_for_setup = verified_resources.clone();
    let builder = builder
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            #[cfg(target_os = "macos")]
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(move |app| {
            let state = state::AppState::load(&app.handle());
            app.manage(state);
            #[cfg(target_os = "windows")]
            if let (Some(resources), Some(mut main_config)) =
                (&resource_for_setup, verified_main_config.as_ref().cloned())
            {
                let set_id = resources.current().expect("verified resource state").set_id;
                main_config.url = tauri::WebviewUrl::CustomProtocol(
                    reqwest::Url::parse(&format!(
                        "gogoke-resource://localhost/{set_id}/index.html"
                    ))
                    .expect("verified gogoke resource URL"),
                );
                app.manage(resources.clone());
                tauri::WebviewWindowBuilder::from_config(app.handle(), &main_config)?.build()?;
            }
            #[cfg(target_os = "macos")]
            {
                let tray_state = app.state::<tray::TrayState>();
                tray::initialize(&app.handle(), tray_state.inner())?;
            }
            #[cfg(target_os = "windows")]
            {
                if let Some(main_window) = app.get_webview_window("main") {
                    let _ = main_window.set_decorations(false);
                    // Keep menu accelerators wired while suppressing a visible native menu bar.
                    let _ = main_window.hide_menu();
                }
            }
            #[cfg(desktop)]
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = app_handle.state::<state::AppState>();
                    let settings = state.app_settings.lock().await.clone();
                    if matches!(
                        settings.remote_backend_provider,
                        crate::types::RemoteBackendProvider::Tcp
                    ) {
                        if matches!(settings.backend_mode, crate::types::BackendMode::Remote) {
                            // Remote mode: ensure daemon is up and version-current.
                            let state = app_handle.state::<state::AppState>();
                            let _ = tailscale::tailscale_daemon_start(state).await;
                        } else {
                            // Local mode: only enforce version if daemon is already running.
                            let state = app_handle.state::<state::AppState>();
                            if let Ok(status) = tailscale::tailscale_daemon_status(state).await {
                                if matches!(status.state, crate::types::TcpDaemonState::Running) {
                                    let state = app_handle.state::<state::AppState>();
                                    let _ = tailscale::tailscale_daemon_start(state).await;
                                }
                            }
                        }
                    }
                });
            }
            #[cfg(target_os = "ios")]
            {
                if let Some(main_webview) = app.get_webview_window("main") {
                    let _ = window::configure_ios_webview_edge_to_edge(&main_webview);
                }
            }
            Ok(())
        });

    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_window_state::Builder::default().build());

    let app = builder
        .plugin(tauri_plugin_liquid_glass::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            settings::get_app_settings,
            settings::update_app_settings,
            settings::get_codex_config_path,
            gogoke_update::gogoke_product_version,
            gogoke_update::gogoke_update_check,
            gogoke_update::gogoke_update_install,
            gogoke_update::gogoke_update_signal_ready,
            gogoke_update::gogoke_update_take_failure,
            public_runtime::product_entry::gogoke_r2_goal_probe,
            files::file_read,
            files::file_write,
            files::read_image_as_data_url,
            files::write_text_file,
            codex::get_config_model,
            menu::menu_set_accelerators,
            tray::set_tray_recent_threads,
            tray::set_tray_session_usage,
            codex::codex_doctor,
            codex::codex_update,
            workspaces::list_workspaces,
            workspaces::is_workspace_path_dir,
            workspaces::add_workspace,
            workspaces::add_workspace_from_git_url,
            workspaces::add_clone,
            workspaces::add_worktree,
            workspaces::worktree_setup_status,
            workspaces::worktree_setup_mark_ran,
            workspaces::remove_workspace,
            workspaces::remove_worktree,
            workspaces::rename_worktree,
            workspaces::rename_worktree_upstream,
            workspaces::apply_worktree_changes,
            workspaces::update_workspace_settings,
            workspaces::set_workspace_runtime_codex_args,
            codex::start_thread,
            codex::send_user_message,
            codex::turn_steer,
            codex::turn_interrupt,
            codex::start_review,
            codex::respond_to_server_request,
            codex::remember_approval_rule,
            codex::generate_commit_message,
            codex::generate_run_metadata,
            codex::generate_agent_description,
            codex::resume_thread,
            codex::read_thread,
            codex::thread_live_subscribe,
            codex::thread_live_unsubscribe,
            codex::fork_thread,
            codex::list_threads,
            codex::list_mcp_server_status,
            codex::archive_thread,
            codex::compact_thread,
            codex::set_thread_name,
            codex::collaboration_mode_list,
            workspaces::connect_workspace,
            git::get_git_status,
            git::init_git_repo,
            git::create_github_repo,
            git::list_git_roots,
            git::get_git_diffs,
            git::get_git_log,
            git::get_git_commit_diff,
            git::get_git_remote,
            git::stage_git_file,
            git::stage_git_all,
            git::unstage_git_file,
            git::revert_git_file,
            git::revert_git_all,
            git::commit_git,
            git::push_git,
            git::pull_git,
            git::fetch_git,
            git::sync_git,
            git::get_github_issues,
            git::get_github_pull_requests,
            git::get_github_pull_request_diff,
            git::get_github_pull_request_comments,
            git::checkout_github_pull_request,
            workspaces::list_workspace_files,
            workspaces::read_workspace_file,
            workspaces::open_workspace_in,
            workspaces::get_open_app_icon,
            git::list_git_branches,
            git::checkout_git_branch,
            git::create_git_branch,
            codex::model_list,
            codex::experimental_feature_list,
            codex::set_codex_feature_flag,
            codex::get_agents_settings,
            codex::set_agents_core_settings,
            codex::create_agent,
            codex::update_agent,
            codex::delete_agent,
            codex::read_agent_config_toml,
            codex::write_agent_config_toml,
            codex::account_rate_limits,
            codex::account_read,
            codex::codex_login,
            codex::codex_login_cancel,
            codex::skills_list,
            codex::apps_list,
            prompts::prompts_list,
            prompts::prompts_create,
            prompts::prompts_update,
            prompts::prompts_delete,
            prompts::prompts_move,
            prompts::prompts_workspace_dir,
            prompts::prompts_global_dir,
            terminal::terminal_open,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_close,
            dictation::dictation_model_status,
            dictation::dictation_download_model,
            dictation::dictation_cancel_download,
            dictation::dictation_remove_model,
            dictation::dictation_start,
            dictation::dictation_request_permission,
            dictation::dictation_stop,
            dictation::dictation_cancel,
            local_usage::local_usage_snapshot,
            notifications::is_macos_debug_build,
            notifications::app_build_type,
            notifications::send_notification_fallback,
            tailscale::tailscale_status,
            tailscale::tailscale_daemon_command_preview,
            tailscale::tailscale_daemon_start,
            tailscale::tailscale_daemon_stop,
            tailscale::tailscale_daemon_status,
            is_mobile_runtime
        ])
        .build(context)
        .expect("error while running tauri application");

    app.run(|app_handle, event| {
        #[cfg(desktop)]
        if let RunEvent::ExitRequested { api, .. } = event {
            if !EXIT_CLEANUP_IN_PROGRESS.load(Ordering::SeqCst)
                && !keep_daemon_running_after_close(app_handle)
            {
                api.prevent_exit();
                EXIT_CLEANUP_IN_PROGRESS.store(true, Ordering::SeqCst);
                let app_handle = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    stop_managed_daemons_for_exit(app_handle.clone()).await;
                    app_handle.exit(0);
                });
            }
            return;
        }

        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = event {
            if let Some(window) = app_handle.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    });
}
