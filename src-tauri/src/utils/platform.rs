use std::env;
use std::path::PathBuf;
use std::sync::OnceLock;

static RESOURCE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// 获取操作系统类型
pub fn get_os() -> String {
    env::consts::OS.to_string()
}

/// 获取系统架构
pub fn get_arch() -> String {
    env::consts::ARCH.to_string()
}

/// 初始化 Tauri 资源目录
pub fn set_resource_dir(path: Option<PathBuf>) {
    let _ = RESOURCE_DIR.set(path);
}

/// 获取 Tauri 资源目录
pub fn get_resource_dir() -> Option<PathBuf> {
    RESOURCE_DIR.get().cloned().flatten()
}

/// 获取打包运行时根目录
pub fn get_bundled_runtime_dir() -> Option<PathBuf> {
    if let Ok(path) = env::var("BOSICLAW_RUNTIME_DIR") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    if let Some(resource_dir) = get_resource_dir() {
        let runtime_dir = resource_dir.join("runtime");
        if runtime_dir.exists() {
            return Some(runtime_dir);
        }
    }

    if cfg!(debug_assertions) {
        let dev_runtime_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime");
        if dev_runtime_dir.exists() {
            return Some(dev_runtime_dir);
        }
    }

    None
}

fn fallback_home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

fn expand_home_alias(path: &str) -> PathBuf {
    match path {
        "~" => fallback_home_dir().unwrap_or_else(|| PathBuf::from(path)),
        _ => {
            if let Some(stripped) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
                if let Some(home) = fallback_home_dir() {
                    return home.join(stripped);
                }
            }
            PathBuf::from(path)
        }
    }
}

fn resolve_path_from_env(key: &str) -> Option<PathBuf> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| expand_home_alias(&value))
}

fn get_effective_home_dir() -> Option<PathBuf> {
    resolve_path_from_env("OPENCLAW_HOME").or_else(fallback_home_dir)
}

fn get_state_dir_path() -> PathBuf {
    resolve_path_from_env("OPENCLAW_STATE_DIR")
        .or_else(|| get_effective_home_dir().map(|home| home.join(".openclaw")))
        .unwrap_or_else(|| {
            if is_windows() {
                PathBuf::from("~\\.openclaw")
            } else {
                PathBuf::from("~/.openclaw")
            }
        })
}

fn get_config_file_path_buf() -> PathBuf {
    resolve_path_from_env("OPENCLAW_CONFIG_PATH")
        .unwrap_or_else(|| get_state_dir_path().join("openclaw.json"))
}

/// 获取 OpenClaw 状态目录路径
pub fn get_config_dir() -> String {
    get_state_dir_path().display().to_string()
}

/// 获取环境变量文件路径
pub fn get_env_file_path() -> String {
    get_state_dir_path().join("env").display().to_string()
}

/// 获取 openclaw.json 配置文件路径
pub fn get_config_file_path() -> String {
    get_config_file_path_buf().display().to_string()
}

/// 获取日志文件路径
pub fn get_log_file_path() -> String {
    get_state_dir_path()
        .join("logs")
        .join("gateway.err.log")
        .display()
        .to_string()
}

/// 检测当前平台是否为 macOS
pub fn is_macos() -> bool {
    env::consts::OS == "macos"
}

/// 检测当前平台是否为 Windows
pub fn is_windows() -> bool {
    env::consts::OS == "windows"
}

/// 检测当前平台是否为 Linux
pub fn is_linux() -> bool {
    env::consts::OS == "linux"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn restore_env(key: &str, value: Option<String>) {
        match value {
            Some(value) => env::set_var(key, value),
            None => env::remove_var(key),
        }
    }

    #[test]
    fn config_path_prefers_openclaw_config_path() {
        let _guard = env_lock().lock().expect("env lock");
        let previous_config = env::var("OPENCLAW_CONFIG_PATH").ok();
        let previous_state_dir = env::var("OPENCLAW_STATE_DIR").ok();
        let previous_home = env::var("OPENCLAW_HOME").ok();

        env::set_var("OPENCLAW_CONFIG_PATH", "/tmp/openclaw/custom.json");
        env::remove_var("OPENCLAW_STATE_DIR");
        env::remove_var("OPENCLAW_HOME");

        assert_eq!(get_config_file_path(), "/tmp/openclaw/custom.json");

        restore_env("OPENCLAW_CONFIG_PATH", previous_config);
        restore_env("OPENCLAW_STATE_DIR", previous_state_dir);
        restore_env("OPENCLAW_HOME", previous_home);
    }

    #[test]
    fn state_dir_prefers_openclaw_state_dir_over_home() {
        let _guard = env_lock().lock().expect("env lock");
        let previous_config = env::var("OPENCLAW_CONFIG_PATH").ok();
        let previous_state_dir = env::var("OPENCLAW_STATE_DIR").ok();
        let previous_home = env::var("OPENCLAW_HOME").ok();

        env::remove_var("OPENCLAW_CONFIG_PATH");
        env::set_var("OPENCLAW_STATE_DIR", "/tmp/openclaw-state");
        env::set_var("OPENCLAW_HOME", "/tmp/openclaw-home");

        assert_eq!(get_config_dir(), "/tmp/openclaw-state");
        assert_eq!(get_config_file_path(), "/tmp/openclaw-state/openclaw.json");

        restore_env("OPENCLAW_CONFIG_PATH", previous_config);
        restore_env("OPENCLAW_STATE_DIR", previous_state_dir);
        restore_env("OPENCLAW_HOME", previous_home);
    }
}
