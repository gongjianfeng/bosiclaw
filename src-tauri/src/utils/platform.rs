use std::env;
use std::path::PathBuf;

/// 获取操作系统类型
pub fn get_os() -> String {
    env::consts::OS.to_string()
}

/// 获取系统架构
pub fn get_arch() -> String {
    env::consts::ARCH.to_string()
}

/// 获取配置目录路径
pub fn get_config_dir() -> String {
    if let Some(home) = dirs::home_dir() {
        if is_windows() {
            format!("{}\\.openclaw", home.display())
        } else {
            format!("{}/.openclaw", home.display())
        }
    } else {
        String::from("~/.openclaw")
    }
}

/// 获取环境变量文件路径
pub fn get_env_file_path() -> String {
    if is_windows() {
        format!("{}\\env", get_config_dir())
    } else {
        format!("{}/env", get_config_dir())
    }
}

/// 获取 openclaw.json 配置文件路径
pub fn get_config_file_path() -> String {
    if is_windows() {
        format!("{}\\openclaw.json", get_config_dir())
    } else {
        format!("{}/openclaw.json", get_config_dir())
    }
}

/// 获取日志文件路径
pub fn get_log_file_path() -> String {
    if is_windows() {
        format!("{}\\logs\\gateway.err.log", get_config_dir())
    } else {
        format!("{}/logs/gateway.err.log", get_config_dir())
    }
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

/// 获取应用托管运行时目录
pub fn get_manager_runtime_dir() -> String {
    let mut base = dirs::data_local_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));

    // 使用独立目录避免污染 openclaw 自身配置。
    let app_dir = if is_windows() { "BosiClaw" } else { "bosiclaw" };
    base.push(app_dir);
    base.push("runtime");
    base.display().to_string()
}
