// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::Manager;

/// Argumento usado pelo binário de duplo propósito para entrar em modo daemon.
const IDLE_DAEMON_FLAG: &str = "--idle-daemon";

/// Nome do diretório controlado pelo app dentro da pasta temporária do sistema.
const TEMP_ROOT_NAME: &str = "process_simulator";

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

        // Remove a raiz se ela ficou vazia; não falha se outro processo/app ainda a usa.
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
    target_name: String,
    state: tauri::State<'_, SharedSimulationManager>,
) -> Result<SimulationStatus, String> {
    let safe_name = sanitize_target_name(&target_name)?;
    let mut manager = state
        .lock()
        .map_err(|_| "estado de simulação está bloqueado/contaminado".to_string())?;

    // Este utilitário mantém uma única simulação ativa por vez.
    manager.stop_active()?;

    let current_exe = std::env::current_exe()
        .map_err(|error| format!("falha ao resolver binário atual: {error}"))?;
    let work_dir = build_isolated_work_dir()?;
    fs::create_dir_all(&work_dir).map_err(|error| {
        format!(
            "falha ao criar diretório temporário {}: {error}",
            work_dir.display()
        )
    })?;

    let simulated_exe = work_dir.join(&safe_name);
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

    manager.target_name = Some(safe_name);
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
    if std::env::args().any(|arg| arg == IDLE_DAEMON_FLAG) {
        loop {
            std::thread::park();
        }
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

fn sanitize_target_name(target_name: &str) -> Result<String, String> {
    let trimmed = target_name.trim();

    if trimmed.is_empty() {
        return Err("nome do processo não pode ser vazio".to_string());
    }

    if trimmed.len() > 80 {
        return Err("nome do processo deve ter no máximo 80 caracteres".to_string());
    }

    if trimmed.contains('/') || trimmed.contains('\\') || trimmed == "." || trimmed == ".." {
        return Err("nome do processo deve ser apenas um nome de arquivo".to_string());
    }

    if trimmed.ends_with('.') {
        return Err("nome do processo não pode terminar com ponto".to_string());
    }

    let is_safe = trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));

    if !is_safe {
        return Err("use apenas letras, números, ponto, hífen ou underscore".to_string());
    }

    if is_windows_reserved_name(trimmed) {
        return Err("nome do processo é reservado pelo Windows".to_string());
    }

    Ok(trimmed.to_string())
}

fn build_isolated_work_dir() -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("relógio do sistema inválido: {error}"))?
        .as_nanos();

    Ok(temp_root().join(format!(
        "run-{}-{timestamp}",
        std::process::id()
    )))
}

fn temp_root() -> PathBuf {
    std::env::temp_dir().join(TEMP_ROOT_NAME)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_portable_process_names() {
        assert_eq!(sanitize_target_name("game.exe"), Ok("game.exe".to_string()));
        assert_eq!(
            sanitize_target_name("linux_binary-1_2"),
            Ok("linux_binary-1_2".to_string())
        );
    }

    #[test]
    fn rejects_paths_and_invalid_windows_names() {
        assert!(sanitize_target_name("../game").is_err());
        assert!(sanitize_target_name("game/name").is_err());
        assert!(sanitize_target_name("game\\name").is_err());
        assert!(sanitize_target_name("game.").is_err());
        assert!(sanitize_target_name("con.exe").is_err());
        assert!(sanitize_target_name("LPT1").is_err());
    }

    #[test]
    fn uses_system_temp_directory() {
        assert_eq!(temp_root(), std::env::temp_dir().join(TEMP_ROOT_NAME));
    }
}
