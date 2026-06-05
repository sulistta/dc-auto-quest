// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::Manager;

/// Argumento usado pelo binário de duplo propósito para entrar em modo daemon.
const IDLE_DAEMON_FLAG: &str = "--idle-daemon";

/// Argumento usado para dar título real à janela fake-game no Windows.
const WINDOW_TITLE_FLAG: &str = "--window-title";

/// Nome do diretório controlado pelo app dentro da pasta temporária do sistema.
const TEMP_ROOT_NAME: &str = "DCAutoQuest";

/// Subpasta usada para deixar os processos parecidos com uma instalação de jogo.
const FAKE_GAMES_DIR_NAME: &str = "fake_games";

type SharedSimulationManager = Mutex<SimulationManager>;

#[derive(Debug, Clone, Serialize)]
struct SimulationStatus {
    active: bool,
    pid: Option<u32>,
    target_name: Option<String>,
    executable_path: Option<String>,
}

#[derive(Default)]
struct SimulationManager {
    child: Option<Child>,
    target_name: Option<String>,
    executable_path: Option<PathBuf>,
    work_dir: Option<PathBuf>,
}

impl SimulationManager {
    fn status(&mut self) -> SimulationStatus {
        // Atualiza o estado caso o filho tenha terminado fora do fluxo normal.
        if let Some(child) = self.child.as_mut() {
            if matches!(child.try_wait(), Ok(Some(_))) {
                self.child = None;
            }
        }

        SimulationStatus {
            active: self.child.is_some(),
            pid: self.child.as_ref().map(Child::id),
            target_name: self.target_name.clone(),
            executable_path: self
                .executable_path
                .as_ref()
                .map(|path| path.display().to_string()),
        }
    }

    fn stop_active(&mut self) -> Result<(), String> {
        if let Some(mut child) = self.child.take() {
            match child.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => {
                    child
                        .kill()
                        .map_err(|error| format!("falha ao finalizar processo filho: {error}"))?;
                    let _ = child.wait();
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("falha ao consultar processo filho: {error}"));
                }
            }
        }

        if let Some(work_dir) = self.work_dir.take() {
            if work_dir.exists() {
                fs::remove_dir_all(&work_dir).map_err(|error| {
                    format!(
                        "falha ao remover diretório temporário {}: {error}",
                        work_dir.display()
                    )
                })?;
            }
        }

        // Remove as raízes se ficaram vazias; não falha se outro processo/app ainda as usa.
        let _ = fs::remove_dir(temp_root().join(FAKE_GAMES_DIR_NAME));
        let _ = fs::remove_dir(temp_root());

        self.target_name = None;
        self.executable_path = None;
        Ok(())
    }
}

impl Drop for SimulationManager {
    fn drop(&mut self) {
        // Garante cleanup best-effort se o app fechar sem chamar stop_simulation.
        let _ = self.stop_active();
    }
}

#[tauri::command]
fn start_simulation(
    game_title: String,
    target_name: String,
    state: tauri::State<'_, SharedSimulationManager>,
) -> Result<SimulationStatus, String> {
    let safe_name = sanitize_target_path(&target_name)?;
    let safe_game_title = sanitize_game_title(&game_title);
    let mut manager = state
        .lock()
        .map_err(|_| "estado de simulação está bloqueado/contaminado".to_string())?;

    // Este utilitário mantém uma única simulação ativa por vez.
    manager.stop_active()?;

    let current_exe = std::env::current_exe()
        .map_err(|error| format!("falha ao resolver binário atual: {error}"))?;
    let work_dir = build_isolated_work_dir(&safe_game_title)?;
    fs::create_dir_all(&work_dir).map_err(|error| {
        format!(
            "falha ao criar diretório temporário {}: {error}",
            work_dir.display()
        )
    })?;

    let simulated_exe = work_dir.join(&safe_name);
    if let Some(parent) = simulated_exe.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "falha ao criar diretório do executável simulado {}: {error}",
                parent.display()
            )
        })?;
    }

    fs::copy(&current_exe, &simulated_exe).map_err(|error| {
        format!(
            "falha ao copiar {} para {}: {error}",
            current_exe.display(),
            simulated_exe.display()
        )
    })?;

    ensure_executable_permissions(&simulated_exe)?;

    let child = Command::new(&simulated_exe)
        .arg(IDLE_DAEMON_FLAG)
        .arg(WINDOW_TITLE_FLAG)
        .arg(safe_game_title)
        .current_dir(&work_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            format!(
                "falha ao iniciar processo simulado {}: {error}",
                simulated_exe.display()
            )
        })?;

    manager.target_name = Some(path_to_portable_string(&safe_name));
    manager.executable_path = Some(simulated_exe);
    manager.work_dir = Some(work_dir);
    manager.child = Some(child);

    Ok(manager.status())
}

#[tauri::command]
fn stop_simulation(
    state: tauri::State<'_, SharedSimulationManager>,
) -> Result<SimulationStatus, String> {
    let mut manager = state
        .lock()
        .map_err(|_| "estado de simulação está bloqueado/contaminado".to_string())?;
    manager.stop_active()?;
    Ok(manager.status())
}

fn main() {
    // Intercepta o modo daemon antes de qualquer inicialização do Tauri/webview.
    let args = std::env::args().collect::<Vec<_>>();
    if args.iter().any(|arg| arg == IDLE_DAEMON_FLAG) {
        run_fake_game_window(read_window_title(&args));
        return;
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .manage(SharedSimulationManager::default())
        .invoke_handler(tauri::generate_handler![start_simulation, stop_simulation])
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                if let Ok(mut manager) = window.state::<SharedSimulationManager>().lock() {
                    let _ = manager.stop_active();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn sanitize_target_path(target_name: &str) -> Result<PathBuf, String> {
    let trimmed = target_name.trim();

    if trimmed.is_empty() {
        return Err("caminho do executável não pode ser vazio".to_string());
    }

    if trimmed.len() > 180 {
        return Err("caminho do executável deve ter no máximo 180 caracteres".to_string());
    }

    if trimmed.starts_with('/') || trimmed.starts_with('\\') || Path::new(trimmed).is_absolute() {
        return Err("caminho do executável deve ser relativo".to_string());
    }

    let parts = trimmed
        .replace('\\', "/")
        .split('/')
        .map(str::trim)
        .map(validate_path_part)
        .collect::<Result<Vec<_>, _>>()?;

    let mut path = PathBuf::new();
    for part in parts {
        path.push(part);
    }

    Ok(path)
}

fn validate_path_part(part: &str) -> Result<String, String> {
    if part.is_empty() || part == "." || part == ".." {
        return Err("caminho do executável contém parte inválida".to_string());
    }

    if part.ends_with('.') {
        return Err("parte do caminho não pode terminar com ponto".to_string());
    }

    let is_safe = part
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));

    if !is_safe {
        return Err(
            "use apenas letras, números, ponto, hífen, underscore ou barra".to_string(),
        );
    }

    if is_windows_reserved_name(part) {
        return Err("parte do caminho é reservada pelo Windows".to_string());
    }

    Ok(part.to_string())
}

fn build_isolated_work_dir(game_title: &str) -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("relógio do sistema inválido: {error}"))?
        .as_nanos();

    Ok(temp_root()
        .join(FAKE_GAMES_DIR_NAME)
        .join(format!("{game_title}-{}-{timestamp}", std::process::id())))
}

fn temp_root() -> PathBuf {
    std::env::temp_dir().join(TEMP_ROOT_NAME)
}

fn path_to_portable_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn sanitize_game_title(title: &str) -> String {
    let normalized = title
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '_' | '-' | '.') {
                Some(ch)
            } else if ch.is_whitespace() {
                Some(' ')
            } else {
                None
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches('.')
        .chars()
        .take(80)
        .collect::<String>();

    if normalized.is_empty() {
        "Game".to_string()
    } else {
        normalized
    }
}

fn read_window_title(args: &[String]) -> String {
    args.windows(2)
        .find_map(|window| {
            if window[0] == WINDOW_TITLE_FLAG {
                Some(window[1].clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "Game".to_string())
}

fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();

    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[cfg(unix)]
fn ensure_executable_permissions(path: &PathBuf) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| format!("falha ao ler permissões de {}: {error}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("falha ao aplicar chmod 755 em {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn ensure_executable_permissions(_path: &PathBuf) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
fn run_fake_game_window(_title: String) {
    loop {
        std::thread::park();
    }
}

#[cfg(windows)]
fn run_fake_game_window(title: String) {
    windows_fake_window::run(title);
}

#[cfg(windows)]
mod windows_fake_window {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    type Bool = i32;
    type Dword = u32;
    type Hbrush = *mut c_void;
    type Hcursor = *mut c_void;
    type Hicon = *mut c_void;
    type Hinstance = *mut c_void;
    type Hmenu = *mut c_void;
    type Hwnd = *mut c_void;
    type Lparam = isize;
    type Lresult = isize;
    type Uint = u32;
    type Wparam = usize;

    const CS_HREDRAW: Uint = 0x0002;
    const CS_VREDRAW: Uint = 0x0001;
    const CW_USEDEFAULT: i32 = 0x80000000u32 as i32;
    const IDC_ARROW: usize = 32512;
    const SW_SHOWNORMAL: i32 = 1;
    const WM_DESTROY: Uint = 0x0002;
    const WM_PAINT: Uint = 0x000F;
    const WS_OVERLAPPEDWINDOW: Dword = 0x00CF0000;
    const COLOR_WINDOW: isize = 5;

    #[repr(C)]
    struct WndClassW {
        style: Uint,
        lpfn_wnd_proc: Option<unsafe extern "system" fn(Hwnd, Uint, Wparam, Lparam) -> Lresult>,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: Hinstance,
        h_icon: Hicon,
        h_cursor: Hcursor,
        hbr_background: Hbrush,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: Uint,
        w_param: Wparam,
        l_param: Lparam,
        time: Dword,
        pt: Point,
    }

    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct PaintStruct {
        hdc: *mut c_void,
        f_erase: Bool,
        rc_paint: Rect,
        f_restore: Bool,
        f_inc_update: Bool,
        rgb_reserved: [u8; 32],
    }

    #[link(name = "user32")]
    extern "system" {
        fn BeginPaint(hwnd: Hwnd, lp_paint: *mut PaintStruct) -> *mut c_void;
        fn CreateWindowExW(
            dw_ex_style: Dword,
            lp_class_name: *const u16,
            lp_window_name: *const u16,
            dw_style: Dword,
            x: i32,
            y: i32,
            n_width: i32,
            n_height: i32,
            h_wnd_parent: Hwnd,
            h_menu: Hmenu,
            h_instance: Hinstance,
            lp_param: *mut c_void,
        ) -> Hwnd;
        fn DefWindowProcW(hwnd: Hwnd, msg: Uint, w_param: Wparam, l_param: Lparam) -> Lresult;
        fn DispatchMessageW(lp_msg: *const Msg) -> Lresult;
        fn EndPaint(hwnd: Hwnd, lp_paint: *const PaintStruct) -> Bool;
        fn FillRect(hdc: *mut c_void, lprc: *const Rect, hbr: Hbrush) -> i32;
        fn GetMessageW(lp_msg: *mut Msg, hwnd: Hwnd, msg_filter_min: Uint, msg_filter_max: Uint)
            -> Bool;
        fn LoadCursorW(h_instance: Hinstance, lp_cursor_name: *const u16) -> Hcursor;
        fn PostQuitMessage(n_exit_code: i32);
        fn RegisterClassW(lp_wnd_class: *const WndClassW) -> u16;
        fn ShowWindow(hwnd: Hwnd, n_cmd_show: i32) -> Bool;
        fn TranslateMessage(lp_msg: *const Msg) -> Bool;
        fn UpdateWindow(hwnd: Hwnd) -> Bool;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(lp_module_name: *const u16) -> Hinstance;
    }

    pub fn run(title: String) {
        let class_name = to_wide("DCAutoQuestFakeGameWindow");
        let window_title = to_wide(&title);

        unsafe {
            let instance = GetModuleHandleW(null());
            let wnd_class = WndClassW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfn_wnd_proc: Some(window_proc),
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance: instance,
                h_icon: null_mut(),
                h_cursor: LoadCursorW(null_mut(), IDC_ARROW as *const u16),
                hbr_background: (COLOR_WINDOW + 1) as Hbrush,
                lpsz_menu_name: null(),
                lpsz_class_name: class_name.as_ptr(),
            };

            RegisterClassW(&wnd_class);
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                window_title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                800,
                600,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            );

            if hwnd.is_null() {
                loop {
                    std::thread::park();
                }
            }

            ShowWindow(hwnd, SW_SHOWNORMAL);
            UpdateWindow(hwnd);

            let mut msg = std::mem::zeroed::<Msg>();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: Hwnd,
        msg: Uint,
        w_param: Wparam,
        l_param: Lparam,
    ) -> Lresult {
        match msg {
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            WM_PAINT => {
                let mut paint = std::mem::zeroed::<PaintStruct>();
                let hdc = BeginPaint(hwnd, &mut paint);
                FillRect(hdc, &paint.rc_paint, (COLOR_WINDOW + 1) as Hbrush);
                EndPaint(hwnd, &paint);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w_param, l_param),
        }
    }

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_portable_relative_executable_paths() {
        assert_eq!(sanitize_target_path("game.exe"), Ok(PathBuf::from("game.exe")));
        assert_eq!(
            sanitize_target_path("bin/linux_binary-1_2"),
            Ok(PathBuf::from("bin").join("linux_binary-1_2"))
        );
    }

    #[test]
    fn rejects_absolute_traversal_and_invalid_windows_names() {
        assert!(sanitize_target_path("../game").is_err());
        assert!(sanitize_target_path("/game.exe").is_err());
        assert!(sanitize_target_path("C:\\game.exe").is_err());
        assert!(sanitize_target_path("game.").is_err());
        assert!(sanitize_target_path("bin/con.exe").is_err());
        assert!(sanitize_target_path("LPT1").is_err());
    }

    #[test]
    fn uses_system_temp_directory() {
        assert_eq!(temp_root(), std::env::temp_dir().join(TEMP_ROOT_NAME));
    }

    #[test]
    fn builds_fake_game_directory_under_temp_root() {
        let dir = build_isolated_work_dir("Overwatch 2").expect("valid temp dir");

        assert!(dir.starts_with(temp_root().join(FAKE_GAMES_DIR_NAME)));
        assert!(dir.to_string_lossy().contains("Overwatch 2"));
    }
}
