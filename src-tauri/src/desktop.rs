use crate::{
    config::{Config, ConfigStore},
    engine::{Engine, Snapshot},
    models::{self, Catalogue, Download, Downloads},
    updates::{DownloadProgress, DownloadedUpdate, UpdateInfo, Updates},
    viewport,
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{
    Emitter, Manager, State,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_opener::OpenerExt;

#[derive(Default)]
struct HeldHotkeys(Mutex<HashSet<u32>>);

#[tauri::command]
fn get_model_catalogue(downloads: State<'_, Arc<Downloads>>) -> Catalogue {
    models::catalogue(downloads.root())
}

#[tauri::command]
fn get_model_downloads(downloads: State<'_, Arc<Downloads>>) -> Vec<Download> {
    downloads.snapshot()
}

#[tauri::command]
async fn download_model(
    downloads: State<'_, Arc<Downloads>>,
    model_id: String,
    variant_id: String,
) -> Result<String, String> {
    downloads
        .start(&model_id, &variant_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn pause_model_download(downloads: State<'_, Arc<Downloads>>, id: String) {
    downloads.cancel(&id);
}

#[tauri::command]
async fn load_model(
    engine: State<'_, Arc<Engine>>,
    downloads: State<'_, Arc<Downloads>>,
    model_id: String,
    variant_id: String,
) -> Result<Snapshot, String> {
    let _workflow = engine
        .workflow
        .try_lock()
        .map_err(|_| "请先结束当前识别".to_string())?;
    ensure_idle(&engine).map_err(|error| error.to_string())?;
    let (model, variant) =
        models::find_variant(&model_id, &variant_id).map_err(|error| error.to_string())?;
    let mut config = engine.snapshot().config;
    if model.backend == "windows" {
        config.model.model_path.clear();
        config.model.mmproj_path.clear();
    } else {
        let (main, projector) = models::variant_paths(downloads.root(), &model_id, variant)
            .map_err(|error| error.to_string())?;
        for (path, file) in std::iter::once(&main)
            .chain(projector.iter())
            .zip(&variant.files)
        {
            models::verify(path, file, &tokio_util::sync::CancellationToken::new())
                .await
                .map_err(|error| error.to_string())?;
        }
        let base = engine.store.lock().unwrap().base_dir.clone();
        config.model.model_path = stored_model_path(&base, main);
        config.model.mmproj_path = projector
            .map(|path| stored_model_path(&base, path))
            .unwrap_or_default();
    }
    config.model.profile = model_id;
    config.model.context_size = model.context_size;
    config.ocr.prompt = crate::profiles::default_prompt(&config.model.profile).into();
    config.model.image_min_tokens =
        crate::profiles::profile(&config.model.profile).image_min_tokens;
    config.ocr.max_image_dimension =
        crate::profiles::profile(&config.model.profile).max_image_dimension;
    if config.model.profile != "xiaomi-ocr" {
        config.ocr.xiaomi_table_postprocess = false;
    }
    let restart = engine
        .save_config(config)
        .map_err(|error| error.to_string())?;
    if restart {
        let engine = engine.inner().clone();
        tauri::async_runtime::spawn(async move {
            let _ = engine.start_server().await;
        });
    }
    Ok(engine.snapshot())
}

fn stored_model_path(base: &std::path::Path, path: PathBuf) -> String {
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    canonical
        .strip_prefix(base)
        .unwrap_or(&canonical)
        .to_string_lossy()
        .into_owned()
}

#[tauri::command]
fn open_model_page(
    app: tauri::AppHandle,
    model_id: String,
    variant_id: Option<String>,
) -> Result<(), String> {
    let model = models::CATALOGUE
        .models
        .iter()
        .find(|entry| entry.id == model_id)
        .ok_or("模型不存在")?;
    if model.backend == "windows" {
        return app
            .opener()
            .open_url(
                "https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr.ocrengine",
                None::<String>,
            )
            .map_err(|error| error.to_string());
    }
    let repo = variant_id
        .as_deref()
        .and_then(|id| model.variants.iter().find(|variant| variant.id == id))
        .and_then(|variant| variant.files.first())
        .map(|file| file.repo.as_str())
        .unwrap_or(&model.source_repo);
    app.opener()
        .open_url(format!("https://huggingface.co/{repo}"), None::<String>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_snapshot(engine: State<'_, Arc<Engine>>) -> Snapshot {
    engine.snapshot()
}

#[tauri::command]
fn open_ocr_languages(app: tauri::AppHandle) -> Result<(), String> {
    app.opener()
        .open_url("ms-settings:regionlanguage", None::<String>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn refresh_hardware(engine: State<'_, Arc<Engine>>) -> Result<Snapshot, String> {
    Ok(engine.refresh_hardware().await)
}

#[tauri::command]
fn capture(app: tauri::AppHandle, engine: State<'_, Arc<Engine>>) {
    start_capture(app, engine.inner().clone());
}

#[tauri::command]
fn cancel(engine: State<'_, Arc<Engine>>) {
    engine.cancel();
}

#[tauri::command]
fn update_result_text(
    engine: State<'_, Arc<Engine>>,
    id: u64,
    text: String,
) -> Result<Snapshot, String> {
    engine
        .update_result_text(id, text)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn clear_result(engine: State<'_, Arc<Engine>>) -> Result<Snapshot, String> {
    engine.clear_result().map_err(|error| error.to_string())
}

fn request_capture(app: &tauri::AppHandle) {
    if app.state::<Arc<Engine>>().snapshot().phase == "selecting" {
        return;
    }
    let _ = app.emit("ocr-request-capture", ());
}

#[tauri::command]
fn set_hotkey_recording(
    app: tauri::AppHandle,
    engine: State<'_, Arc<Engine>>,
    recording: bool,
) -> Result<(), String> {
    if recording {
        app.global_shortcut()
            .unregister_all()
            .map_err(|error| error.to_string())
    } else {
        let result =
            apply_hotkeys(&app, &engine.snapshot().config).map_err(|error| error.to_string());
        engine.hotkey_error(result.as_ref().err().cloned());
        result
    }
}

#[tauri::command]
fn recognize_file(app: tauri::AppHandle, engine: State<'_, Arc<Engine>>, path: String) {
    let engine = engine.inner().clone();
    let request = engine.request();
    tauri::async_runtime::spawn(async move {
        let _workflow = engine.workflow.lock().await;
        if !engine.current_request(request) {
            return;
        }
        let token = match engine.begin(
            request,
            PathBuf::from(&path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            "recognizing",
        ) {
            Ok(token) => token,
            Err(error) => {
                engine.notice(error.to_string());
                return;
            }
        };
        let image = tokio::task::spawn_blocking(move || image::open(path)).await;
        let outcome = match image {
            Ok(Ok(image)) => engine.recognize(request, image, token.clone()).await,
            Ok(Err(error)) => Err(error.into()),
            Err(error) => Err(error.into()),
        };
        if let Err(error) = outcome {
            engine.fail(request, &error.to_string(), token.is_cancelled());
            if token.is_cancelled()
                && engine.snapshot().config.model.profile == "teleocr"
                && !engine.snapshot().ready
            {
                let restart = engine.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = restart.start_server().await;
                });
            }
        }
        show_main(&app);
    });
}

#[tauri::command]
async fn save_settings(
    app: tauri::AppHandle,
    engine: State<'_, Arc<Engine>>,
    config: Config,
) -> Result<Snapshot, String> {
    let _workflow = engine
        .workflow
        .try_lock()
        .map_err(|_| "请先结束当前识别".to_string())?;
    config.validate().map_err(|error| error.to_string())?;
    ensure_idle(&engine).map_err(|error| error.to_string())?;
    let previous = engine.snapshot().config;
    let hotkeys_changed = previous.hotkey != config.hotkey;
    if hotkeys_changed {
        apply_hotkeys(&app, &config).map_err(|error| error.to_string())?;
        engine.hotkey_error(None);
    }
    let restart = match engine.save_config(config.clone()) {
        Ok(restart) => restart,
        Err(error) => {
            if hotkeys_changed {
                let _ = apply_hotkeys(&app, &previous);
            }
            return Err(error.to_string());
        }
    };
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_always_on_top(config.ui.always_on_top);
    }
    if restart {
        let engine = engine.inner().clone();
        tauri::async_runtime::spawn(async move {
            let _ = engine.start_server().await;
        });
    }
    Ok(engine.snapshot())
}

#[tauri::command]
fn retry_engine(engine: State<'_, Arc<Engine>>) -> Result<(), String> {
    ensure_idle(&engine).map_err(|error| error.to_string())?;
    engine.prepare_reload();
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn(async move {
        let _ = engine.start_server().await;
    });
    Ok(())
}

#[tauri::command]
fn export_text(path: String, text: String) -> Result<(), String> {
    fs::write(path, text).map_err(|error| error.to_string())
}

#[tauri::command]
async fn check_updates(updates: State<'_, Arc<Updates>>) -> Result<UpdateInfo, String> {
    updates.check().await.map_err(|error| error.to_string())
}

#[tauri::command]
async fn download_update(
    app: tauri::AppHandle,
    updates: State<'_, Arc<Updates>>,
) -> Result<DownloadedUpdate, String> {
    let downloads = app
        .path()
        .download_dir()
        .map_err(|error| error.to_string())?;
    updates
        .download(downloads, move |progress: DownloadProgress| {
            let _ = app.emit("update-download-progress", progress);
        })
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn install_update(
    app: tauri::AppHandle,
    updates: State<'_, Arc<Updates>>,
) -> Result<(), String> {
    updates.launch().await.map_err(|error| error.to_string())?;
    exit_gracefully(app);
    Ok(())
}

#[tauri::command]
fn quit(app: tauri::AppHandle) {
    exit_gracefully(app);
}

fn ensure_idle(engine: &Engine) -> Result<()> {
    ensure!(
        !engine.busy() && engine.snapshot().phase != "loading",
        "请等待当前操作结束"
    );
    Ok(())
}

fn show_main(app: &tauri::AppHandle) {
    if app
        .try_state::<Arc<Engine>>()
        .is_some_and(|engine| engine.snapshot().phase == "selecting")
    {
        return;
    }
    restore_main(app);
}
fn restore_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn start_capture(app: tauri::AppHandle, engine: Arc<Engine>) {
    if engine.snapshot().phase == "selecting" {
        return;
    }
    if !engine.snapshot().ready {
        engine.notice("识别引擎正在准备，请稍候或检查设置");
        show_main(&app);
        return;
    }
    let request = engine.request();
    tauri::async_runtime::spawn(async move {
        let _workflow = engine.workflow.lock().await;
        if !engine.current_request(request) {
            return;
        }
        let token = match engine.begin(request, "屏幕选区".into(), "selecting") {
            Ok(token) => token,
            Err(error) => {
                engine.notice(error.to_string());
                return;
            }
        };
        let selected = async {
            let window = app.get_webview_window("main").context("主窗口不可用")?;
            let handle = window.hwnd()?.0 as usize;
            let (send, receive) = tokio::sync::oneshot::channel();
            app.run_on_main_thread(move || {
                let _ = send.send(crate::native::hide_for_capture(handle));
            })?;
            receive.await.context("隐藏窗口已取消")??;
            ensure!(!token.is_cancelled(), "已取消框选");
            let selection_token = token.clone();
            tokio::task::spawn_blocking(move || crate::native::select_region(selection_token))
                .await?
        }
        .await;
        match selected {
            Ok(Some(image)) => {
                restore_main(&app);
                if let Err(error) = engine.recognize(request, image, token.clone()).await {
                    engine.fail(request, &error.to_string(), token.is_cancelled());
                    if token.is_cancelled()
                        && engine.snapshot().config.model.profile == "teleocr"
                        && !engine.snapshot().ready
                    {
                        let restart = engine.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = restart.start_server().await;
                        });
                    }
                }
            }
            Ok(None) => engine.fail(request, "已取消框选", true),
            Err(error) => engine.fail(request, &error.to_string(), token.is_cancelled()),
        }
        if engine.current_request(request) {
            show_main(&app);
        }
    });
}

fn shortcut(text: &str) -> Result<Shortcut> {
    let normalized = text
        .split('+')
        .map(|part| match part.trim().to_lowercase().as_str() {
            "ctrl" | "control" => "Control".into(),
            "win" | "windows" | "meta" | "super" => "Super".into(),
            "esc" => "Escape".into(),
            _ => part.trim().to_owned(),
        })
        .collect::<Vec<String>>()
        .join("+");
    normalized
        .parse()
        .with_context(|| format!("无法识别快捷键：{text}"))
}

fn apply_hotkeys(app: &tauri::AppHandle, config: &Config) -> Result<()> {
    let manager = app.global_shortcut();
    let previous = app.state::<Arc<Engine>>().snapshot().config.hotkey;
    let pending: Vec<Shortcut> = if config.hotkey.enabled {
        let trigger = shortcut(&config.hotkey.trigger)?;
        let mut keys = vec![trigger];
        if !config.hotkey.quit.is_empty() {
            let quit = shortcut(&config.hotkey.quit)?;
            ensure!(quit != trigger, "框选和退出快捷键应不同");
            keys.push(quit);
        }
        keys
    } else {
        Vec::new()
    };
    manager.unregister_all()?;
    for key in &pending {
        if let Err(error) = manager.register(*key) {
            let _ = manager.unregister_all();
            if previous.enabled {
                if let Ok(key) = shortcut(&previous.trigger) {
                    let _ = manager.register(key);
                }
                if let Ok(key) = shortcut(&previous.quit) {
                    let _ = manager.register(key);
                }
            }
            anyhow::bail!("快捷键已被占用，请重新录入：{error}");
        }
    }
    Ok(())
}

fn fit_window(window: &tauri::WebviewWindow, center: bool) -> Result<()> {
    let monitor = window
        .current_monitor()?
        .or(window.primary_monitor()?)
        .context("无法读取显示器信息")?;
    let work = monitor.work_area();
    let side = viewport::square_side(work.size.width, work.size.height, monitor.scale_factor());
    window.set_min_size(Some(tauri::LogicalSize::new(
        360.0_f64.min(side),
        360.0_f64.min(side),
    )))?;
    window.set_size(tauri::LogicalSize::new(side, side))?;
    if center {
        let pixels = (side * monitor.scale_factor()) as i32;
        window.set_position(tauri::PhysicalPosition::new(
            work.position.x + (work.size.width as i32 - pixels) / 2,
            work.position.y + (work.size.height as i32 - pixels) / 2,
        ))?;
    }
    Ok(())
}

fn exit_gracefully(app: tauri::AppHandle) {
    let engine = app.state::<Arc<Engine>>().inner().clone();
    tauri::async_runtime::spawn(async move {
        engine.shutdown().await;
        app.exit(0);
    });
}

pub fn run() {
    // NSIS bundles resources inside its resources directory; DirectML is loaded
    // dynamically by ONNX Runtime and needs to be on the DLL search path.
    if let Some(resource_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("resources")))
        .filter(|dir| dir.join("DirectML.dll").is_file())
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW;
        let wide: Vec<u16> = resource_dir
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe {
            SetDllDirectoryW(wide.as_ptr());
        }
    }
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, pressed, event| {
                    let keys = app.state::<HeldHotkeys>();
                    let mut held = keys.0.lock().unwrap();
                    if event.state != ShortcutState::Pressed {
                        held.remove(&pressed.id());
                        return;
                    }
                    if !held.insert(pressed.id()) {
                        return;
                    }
                    drop(held);
                    let engine = app.state::<Arc<Engine>>().inner().clone();
                    let config = engine.snapshot().config;
                    if shortcut(&config.hotkey.trigger).is_ok_and(|key| key == *pressed) {
                        request_capture(app);
                    } else if shortcut(&config.hotkey.quit).is_ok_and(|key| key == *pressed) {
                        exit_gracefully(app.clone());
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh_hardware,
            get_model_catalogue,
            get_model_downloads,
            download_model,
            pause_model_download,
            load_model,
            open_model_page,
            open_ocr_languages,
            capture,
            cancel,
            update_result_text,
            clear_result,
            set_hotkey_recording,
            recognize_file,
            save_settings,
            retry_engine,
            export_text,
            check_updates,
            download_update,
            install_update,
            quit
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let engine = Engine::new(ConfigStore::load()?, move |event| {
                let _ = handle.emit("ocr-state", event);
            })?;
            app.manage(engine.clone());
            let runtime = app
                .path()
                .resource_dir()
                .ok()
                .and_then(|dir| fs::read_to_string(dir.join("runtime.json")).ok())
                .or_else(|| {
                    fs::read_to_string(engine.store.lock().unwrap().exe_dir.join("runtime.json"))
                        .ok()
                });
            let backend = runtime
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                .and_then(|value| value["backend"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "CPU".into());
            app.manage(Arc::new(Updates::new(backend)?));
            app.manage(HeldHotkeys::default());
            let model_root = {
                let store = engine.store.lock().unwrap();
                if store.exe_dir.join("models").is_dir() {
                    store.exe_dir.join("models")
                } else {
                    store.base_dir.join("models")
                }
            };
            let handle = app.handle().clone();
            let downloads = Downloads::new(model_root, move |state| {
                let _ = handle.emit("model-downloads", state);
            })?;
            app.manage(downloads);
            let monitor = Arc::downgrade(&engine);
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let Some(engine) = monitor.upgrade() else {
                        break;
                    };
                    engine.refresh_backend_status().await;
                }
            });
            let window = app.get_webview_window("main").context("主窗口创建失败")?;
            fit_window(&window, true)?;
            window.set_always_on_top(engine.snapshot().config.ui.always_on_top)?;
            let scale_window = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::ScaleFactorChanged { .. } = event {
                    let _ = fit_window(&scale_window, false);
                }
            });
            window.show()?;
            let open = MenuItem::with_id(app, "open", "打开 BlinkOCR", true, None::<&str>)?;
            let capture_item = MenuItem::with_id(app, "capture", "框选识别", true, None::<&str>)?;
            let exit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &capture_item, &exit])?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?;
            TrayIconBuilder::new()
                .icon(icon)
                .tooltip("BlinkOCR")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "capture" => request_capture(app),
                    "quit" => exit_gracefully(app.clone()),
                    _ => (),
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;
            if let Err(error) = apply_hotkeys(app.handle(), &engine.snapshot().config) {
                engine.hotkey_error(Some(error.to_string()));
            }
            tauri::async_runtime::spawn(async move {
                let _ = engine.start_server().await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(|handle, event| {
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
                exit_gracefully(handle.clone());
            }
        }),
        Err(error) => unsafe {
            let message: Vec<u16> = format!("BlinkOCR 启动失败：{error}")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let title: Vec<u16> = "BlinkOCR".encode_utf16().chain(Some(0)).collect();
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                0x10,
            );
        },
    }
}
