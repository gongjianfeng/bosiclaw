use crate::utils::{platform, shell};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tauri::command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const REQUIRED_NODE_MAJOR: u32 = 22;
const MANAGED_NODE_VERSION: &str = "22.13.1";
const MANAGED_GIT_VERSION: &str = "2.44.0.windows.1";
const NPM_REGISTRY_MIRROR: &str = "https://registry.npmmirror.com";
const DEFAULT_GITHUB_PROXY: &str = "https://gh-proxy.com";
const OPENCLAW_INSTALL_RETRIES: usize = 3;

/// 环境检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentStatus {
    /// Node.js 是否安装
    pub node_installed: bool,
    /// Node.js 版本
    pub node_version: Option<String>,
    /// Node.js 版本是否满足要求 (>=22)
    pub node_version_ok: bool,
    /// OpenClaw 是否安装
    pub openclaw_installed: bool,
    /// OpenClaw 版本
    pub openclaw_version: Option<String>,
    /// 配置目录是否存在
    pub config_dir_exists: bool,
    /// 是否全部就绪
    pub ready: bool,
    /// 操作系统
    pub os: String,
}

/// 安装进度
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallProgress {
    pub step: String,
    pub progress: u8,
    pub message: String,
    pub error: Option<String>,
}

/// 安装结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
struct ManagedInstallPaths {
    runtime_dir: PathBuf,
    node_dir: PathBuf,
    git_dir: PathBuf,
    app_dir: PathBuf,
    npm_cache_dir: PathBuf,
    npmrc_path: PathBuf,
    git_config_path: PathBuf,
}

impl ManagedInstallPaths {
    fn new() -> Self {
        let runtime_dir = PathBuf::from(platform::get_manager_runtime_dir());
        Self {
            node_dir: runtime_dir.join("nodejs"),
            git_dir: runtime_dir.join("git_env"),
            app_dir: runtime_dir.join("openclaw_app"),
            npm_cache_dir: runtime_dir.join(".npm-cache"),
            npmrc_path: runtime_dir.join("openclaw_app").join(".npmrc"),
            git_config_path: runtime_dir.join("gitconfig"),
            runtime_dir,
        }
    }
}

fn install_success(message: impl Into<String>) -> InstallResult {
    InstallResult {
        success: true,
        message: message.into(),
        error: None,
    }
}

fn install_failure(message: impl Into<String>, error: impl Into<String>) -> InstallResult {
    InstallResult {
        success: false,
        message: message.into(),
        error: Some(error.into()),
    }
}

fn normalize_version(value: &str) -> String {
    value.trim().trim_start_matches('v').to_string()
}

fn required_node_version_hint() -> String {
    format!("v{}+", REQUIRED_NODE_MAJOR)
}

fn get_github_proxy() -> Option<String> {
    match std::env::var("OPENCLAW_GITHUB_PROXY") {
        Ok(value) => {
            let value = value.trim();
            if value.is_empty()
                || value.eq_ignore_ascii_case("none")
                || value.eq_ignore_ascii_case("direct")
            {
                None
            } else {
                Some(value.trim_end_matches('/').to_string())
            }
        }
        Err(_) => Some(DEFAULT_GITHUB_PROXY.to_string()),
    }
}

fn build_github_target_url(proxy: Option<&str>) -> String {
    match proxy {
        Some(proxy) if !proxy.is_empty() => {
            format!("{}/https://github.com/", proxy.trim_end_matches('/'))
        }
        _ => "https://github.com/".to_string(),
    }
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}: {}", parent.display(), e))?;
    }
    Ok(())
}

fn ensure_managed_runtime_layout(paths: &ManagedInstallPaths) -> Result<(), String> {
    fs::create_dir_all(&paths.runtime_dir)
        .map_err(|e| format!("创建运行时目录失败 {}: {}", paths.runtime_dir.display(), e))?;
    fs::create_dir_all(&paths.app_dir)
        .map_err(|e| format!("创建 OpenClaw 目录失败 {}: {}", paths.app_dir.display(), e))?;
    fs::create_dir_all(&paths.npm_cache_dir).map_err(|e| {
        format!(
            "创建 npm 缓存目录失败 {}: {}",
            paths.npm_cache_dir.display(),
            e
        )
    })?;

    let package_json_path = paths.app_dir.join("package.json");
    if !package_json_path.exists() {
        fs::write(
            &package_json_path,
            "{\n  \"name\": \"openclaw-managed-runtime\",\n  \"version\": \"1.0.0\",\n  \"private\": true\n}\n",
        )
        .map_err(|e| format!("写入 package.json 失败: {}", e))?;
    }

    let npmrc = format!(
        "registry={}\nfund=false\naudit=false\nprefer-offline=true\n",
        NPM_REGISTRY_MIRROR
    );
    fs::write(&paths.npmrc_path, npmrc).map_err(|e| format!("写入 .npmrc 失败: {}", e))?;
    Ok(())
}

fn write_git_proxy_config(path: &Path, proxy: Option<&str>) -> Result<String, String> {
    ensure_parent_dir(path)?;
    let target = build_github_target_url(proxy);
    let content = format!(
        "[url \"{target}\"]\n\tinsteadOf = ssh://git@github.com/\n\tinsteadOf = git@github.com:\n\tinsteadOf = git://github.com/\n\tinsteadOf = https://github.com/\n"
    );
    fs::write(path, content).map_err(|e| format!("写入 Git 配置失败: {}", e))?;
    Ok(target)
}

fn remove_dir_if_exists(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    for attempt in 0..3 {
        match fs::remove_dir_all(path) {
            Ok(_) => return Ok(()),
            Err(e) if attempt < 2 => {
                warn!(
                    "[安装器] 删除目录失败，准备重试: {} - {}",
                    path.display(),
                    e
                );
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(e) => return Err(format!("删除目录失败 {}: {}", path.display(), e)),
        }
    }

    Ok(())
}

fn run_process(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    envs: &[(&str, String)],
) -> Result<String, String> {
    let mut command = if program.ends_with(".cmd") {
        let mut cmd = Command::new("cmd");
        cmd.arg("/c").arg(program);
        cmd.args(args);
        cmd
    } else {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd
    };

    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }

    command.env("PATH", shell::get_extended_path());
    for (key, value) in envs {
        command.env(key, value);
    }

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let output = command
        .output()
        .map_err(|e| format!("执行命令失败 {}: {}", program, e))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if output.status.success() {
        if stdout.is_empty() {
            Ok(stderr)
        } else if stderr.is_empty() {
            Ok(stdout)
        } else {
            Ok(format!("{}\n{}", stdout, stderr).trim().to_string())
        }
    } else if stderr.is_empty() {
        Err(stdout)
    } else if stdout.is_empty() {
        Err(stderr)
    } else {
        Err(format!("{}\n{}", stdout, stderr).trim().to_string())
    }
}

fn run_npm_command(
    args: &[&str],
    cwd: Option<&Path>,
    extra_envs: &[(&str, String)],
) -> Result<String, String> {
    let npm_path = shell::get_npm_path().ok_or_else(|| "找不到 npm 命令".to_string())?;
    run_process(&npm_path, args, cwd, extra_envs)
}

fn run_node_command(node_path: &str, args: &[&str]) -> Result<String, String> {
    run_process(node_path, args, None, &[])
}

fn run_powershell_script(script: &str) -> Result<String, String> {
    shell::run_powershell_output(script)
}

fn download_file(url: &str, destination: &Path) -> Result<(), String> {
    ensure_parent_dir(destination)?;

    if platform::is_windows() {
        let escaped_url = url.replace('\'', "''");
        let escaped_dest = destination.display().to_string().replace('\'', "''");
        let script = format!(
            "$ErrorActionPreference = 'Stop'; \
             $ProgressPreference = 'SilentlyContinue'; \
             Invoke-WebRequest -UseBasicParsing -Uri '{escaped_url}' -OutFile '{escaped_dest}'"
        );
        run_powershell_script(&script)?;
    } else {
        let dest = destination.display().to_string();
        run_process(
            "curl",
            &[
                "-L",
                "--fail",
                "--retry",
                "3",
                "--retry-delay",
                "2",
                "-o",
                &dest,
                url,
            ],
            None,
            &[],
        )?;
    }

    Ok(())
}

fn expand_zip(zip_path: &Path, destination: &Path) -> Result<(), String> {
    remove_dir_if_exists(destination)?;
    ensure_parent_dir(destination)?;

    let zip = zip_path.display().to_string();
    let dest = destination.display().to_string();

    if platform::is_windows() {
        let script = format!(
            "$ErrorActionPreference = 'Stop'; \
             Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
            zip.replace('\'', "''"),
            dest.replace('\'', "''")
        );
        run_powershell_script(&script)?;
    } else {
        run_process("unzip", &["-o", &zip, "-d", &dest], None, &[])?;
    }

    Ok(())
}

fn extract_tar_gz(archive_path: &Path, destination: &Path) -> Result<(), String> {
    remove_dir_if_exists(destination)?;
    fs::create_dir_all(destination)
        .map_err(|e| format!("创建目录失败 {}: {}", destination.display(), e))?;

    let archive = archive_path.display().to_string();
    let dest = destination.display().to_string();
    run_process("tar", &["-xzf", &archive, "-C", &dest], None, &[])?;
    Ok(())
}

fn windows_node_zip_url() -> String {
    format!(
        "https://registry.npmmirror.com/-/binary/node/v{0}/node-v{0}-win-x64.zip",
        MANAGED_NODE_VERSION
    )
}

fn windows_git_zip_url() -> String {
    format!(
        "https://npmmirror.com/mirrors/git-for-windows/v{0}/MinGit-2.44.0-64-bit.zip",
        MANAGED_GIT_VERSION
    )
}

fn macos_node_archive_url() -> Option<String> {
    let arch = platform::get_arch();
    let target = match arch.as_str() {
        "aarch64" | "arm64" => "darwin-arm64",
        "x86_64" => "darwin-x64",
        _ => return None,
    };

    Some(format!(
        "https://npmmirror.com/mirrors/node/v{0}/node-v{0}-{1}.tar.gz",
        MANAGED_NODE_VERSION, target
    ))
}

fn ensure_windows_node_runtime(paths: &ManagedInstallPaths) -> Result<(), String> {
    if paths.node_dir.join("node.exe").exists() {
        return Ok(());
    }

    let zip_path = paths
        .runtime_dir
        .join(format!("node-v{}-win-x64.zip", MANAGED_NODE_VERSION));
    let extract_dir = paths.runtime_dir.join("node_extract");

    info!(
        "[安装Node.js] 下载 Windows Node 运行时: {}",
        windows_node_zip_url()
    );
    download_file(&windows_node_zip_url(), &zip_path)?;
    expand_zip(&zip_path, &extract_dir)?;

    let extracted = fs::read_dir(&extract_dir)
        .map_err(|e| format!("读取 Node 解压目录失败: {}", e))?
        .filter_map(|entry| entry.ok())
        .find(|entry| entry.file_name().to_string_lossy().starts_with("node-v"))
        .map(|entry| entry.path())
        .ok_or_else(|| "未找到解压后的 Node 目录".to_string())?;

    remove_dir_if_exists(&paths.node_dir)?;
    fs::rename(&extracted, &paths.node_dir).map_err(|e| format!("重命名 Node 目录失败: {}", e))?;
    remove_dir_if_exists(&extract_dir)?;
    Ok(())
}

fn ensure_windows_git_runtime(paths: &ManagedInstallPaths) -> Result<(), String> {
    if paths.git_dir.join("cmd").join("git.exe").exists() {
        return Ok(());
    }

    let zip_path = paths
        .runtime_dir
        .join(format!("git-{}.zip", MANAGED_GIT_VERSION));

    info!("[安装Git] 下载 MinGit 运行时: {}", windows_git_zip_url());
    download_file(&windows_git_zip_url(), &zip_path)?;
    expand_zip(&zip_path, &paths.git_dir)?;
    Ok(())
}

fn ensure_macos_node_runtime(paths: &ManagedInstallPaths) -> Result<(), String> {
    if paths.node_dir.join("bin").join("node").exists() {
        return Ok(());
    }

    let url = macos_node_archive_url().ok_or_else(|| {
        format!(
            "当前 macOS 架构 {} 暂不支持自动下载 Node",
            platform::get_arch()
        )
    })?;
    let archive_path = paths
        .runtime_dir
        .join(format!("node-v{}-macos.tar.gz", MANAGED_NODE_VERSION));
    let extract_dir = paths.runtime_dir.join("node_extract");

    info!("[安装Node.js] 下载 macOS Node 运行时: {}", url);
    download_file(&url, &archive_path)?;
    extract_tar_gz(&archive_path, &extract_dir)?;

    let extracted = fs::read_dir(&extract_dir)
        .map_err(|e| format!("读取 Node 解压目录失败: {}", e))?
        .filter_map(|entry| entry.ok())
        .find(|entry| entry.file_name().to_string_lossy().starts_with("node-v"))
        .map(|entry| entry.path())
        .ok_or_else(|| "未找到解压后的 Node 目录".to_string())?;

    remove_dir_if_exists(&paths.node_dir)?;
    fs::rename(&extracted, &paths.node_dir).map_err(|e| format!("重命名 Node 目录失败: {}", e))?;
    remove_dir_if_exists(&extract_dir)?;
    Ok(())
}

fn detect_llama_build_type() -> &'static str {
    if shell::run_command_output("nvidia-smi", &["-L"]).is_ok() {
        "cuda"
    } else {
        "cpu"
    }
}

fn managed_install_env(
    paths: &ManagedInstallPaths,
    proxy: Option<&str>,
) -> Result<Vec<(&'static str, String)>, String> {
    let git_target = write_git_proxy_config(&paths.git_config_path, proxy)?;
    let build_type = detect_llama_build_type();

    let mut envs = vec![
        ("npm_config_registry", NPM_REGISTRY_MIRROR.to_string()),
        (
            "npm_config_cache",
            paths.npm_cache_dir.display().to_string(),
        ),
        (
            "npm_config_userconfig",
            paths.npmrc_path.display().to_string(),
        ),
        (
            "GIT_CONFIG_GLOBAL",
            paths.git_config_path.display().to_string(),
        ),
        ("NODE_LLAMA_CPP_BUILD_TYPE", build_type.to_string()),
        ("OPENCLAW_GIT_TARGET", git_target),
    ];

    if build_type == "cpu" {
        envs.push(("NODE_LLAMA_CPP_SKIP_DOWNLOAD", "true".to_string()));
        envs.push(("NODE_LLAMA_CPP_FORCE_BUILD", "true".to_string()));
    }

    Ok(envs)
}

fn install_openclaw_with_retries(
    cwd: &Path,
    base_envs: &[(&'static str, String)],
    args: &[&str],
) -> Result<String, String> {
    let mut last_error = String::new();

    for attempt in 1..=OPENCLAW_INSTALL_RETRIES {
        info!(
            "[安装OpenClaw] 第 {}/{} 次尝试...",
            attempt, OPENCLAW_INSTALL_RETRIES
        );
        match run_npm_command(args, Some(cwd), base_envs) {
            Ok(output) => return Ok(output),
            Err(err) => {
                warn!("[安装OpenClaw] 第 {} 次尝试失败: {}", attempt, err);
                last_error = err;
                if attempt < OPENCLAW_INSTALL_RETRIES {
                    std::thread::sleep(Duration::from_secs(3));
                }
            }
        }
    }

    Err(last_error)
}

fn configure_user_npm_registry() {
    let _ = run_npm_command(
        &[
            "config",
            "set",
            "registry",
            NPM_REGISTRY_MIRROR,
            "--location",
            "user",
        ],
        None,
        &[],
    );
}

fn ensure_git_available_for_install() -> Result<(), String> {
    if platform::is_windows() {
        if shell::get_managed_git_path().is_some() || shell::command_exists("git") {
            return Ok(());
        }

        return Err(
            "Windows 安装 openclaw 依赖 Git。当前已参考上游安装器逻辑改为显式检查；请先安装 Git，或先执行 Node.js 安装以自动准备 MinGit。"
                .to_string(),
        );
    }

    if shell::command_exists("git") {
        Ok(())
    } else if platform::is_macos() {
        Err("当前 macOS 未检测到 Git。请先运行 xcode-select --install，或自行安装 Git / Homebrew 后重试。".to_string())
    } else {
        Err("当前系统未检测到 Git，请先安装 Git 后再安装 OpenClaw。".to_string())
    }
}

fn install_openclaw_managed(paths: &ManagedInstallPaths) -> Result<String, String> {
    ensure_managed_runtime_layout(paths)?;
    ensure_git_available_for_install()?;
    configure_user_npm_registry();

    let mut routes = Vec::new();
    if let Some(proxy) = get_github_proxy() {
        routes.push(Some(proxy));
        routes.push(None);
    } else {
        routes.push(None);
    }

    let install_args = ["install", "openclaw@latest", "--no-fund", "--no-audit"];
    let mut errors = Vec::new();

    for route in routes {
        let target = build_github_target_url(route.as_deref());
        info!("[安装OpenClaw] 使用 GitHub 路由: {}", target);
        let envs = managed_install_env(paths, route.as_deref())?;
        match install_openclaw_with_retries(&paths.app_dir, &envs, &install_args) {
            Ok(output) => return Ok(output),
            Err(err) => {
                warn!("[安装OpenClaw] 路由 {} 安装失败: {}", target, err);
                errors.push(format!("路由 {} 失败: {}", target, err));
            }
        }
    }

    Err(errors.join("\n"))
}

fn uninstall_openclaw_managed(paths: &ManagedInstallPaths) -> Result<String, String> {
    ensure_managed_runtime_layout(paths)?;
    let envs = vec![
        ("npm_config_registry", NPM_REGISTRY_MIRROR.to_string()),
        (
            "npm_config_cache",
            paths.npm_cache_dir.display().to_string(),
        ),
        (
            "npm_config_userconfig",
            paths.npmrc_path.display().to_string(),
        ),
    ];

    run_npm_command(&["uninstall", "openclaw"], Some(&paths.app_dir), &envs)
}

fn update_openclaw_managed(paths: &ManagedInstallPaths) -> Result<String, String> {
    install_openclaw_managed(paths)
}

/// 检查环境状态
#[command]
pub async fn check_environment() -> Result<EnvironmentStatus, String> {
    info!("[环境检查] 开始检查系统环境...");

    let os = platform::get_os();
    info!("[环境检查] 操作系统: {}", os);

    // 检查 Node.js
    info!("[环境检查] 检查 Node.js...");
    let node_version = get_node_version();
    let node_installed = node_version.is_some();
    let node_version_ok = check_node_version_requirement(&node_version);
    info!(
        "[环境检查] Node.js: installed={}, version={:?}, version_ok={}",
        node_installed, node_version, node_version_ok
    );

    // 检查 OpenClaw
    info!("[环境检查] 检查 OpenClaw...");
    let openclaw_version = get_openclaw_version();
    let openclaw_installed = openclaw_version.is_some();
    info!(
        "[环境检查] OpenClaw: installed={}, version={:?}",
        openclaw_installed, openclaw_version
    );

    // 检查配置目录
    let config_dir = platform::get_config_dir();
    let config_dir_exists = std::path::Path::new(&config_dir).exists();
    info!(
        "[环境检查] 配置目录: {}, exists={}",
        config_dir, config_dir_exists
    );

    let ready = node_installed && node_version_ok && openclaw_installed;
    info!("[环境检查] 环境就绪状态: ready={}", ready);

    Ok(EnvironmentStatus {
        node_installed,
        node_version,
        node_version_ok,
        openclaw_installed,
        openclaw_version,
        config_dir_exists,
        ready,
        os,
    })
}

/// 获取 Node.js 版本
/// 检测多个可能的安装路径，因为 GUI 应用不继承用户 shell 的 PATH
fn get_node_version() -> Option<String> {
    if let Some(node_path) = shell::get_managed_node_path() {
        if let Ok(version) = run_node_command(&node_path, &["--version"]) {
            let version = version.trim().to_string();
            if !version.is_empty() && version.starts_with('v') {
                info!("[环境检查] 通过托管运行时找到 Node.js: {}", version);
                return Some(version);
            }
        }
    }

    if platform::is_windows() {
        // Windows: 先尝试直接调用（如果 PATH 已更新）
        if let Ok(v) = shell::run_cmd_output("node --version") {
            let version = v.trim().to_string();
            if !version.is_empty() && version.starts_with('v') {
                info!("[环境检查] 通过 PATH 找到 Node.js: {}", version);
                return Some(version);
            }
        }

        // Windows: 检查常见的安装路径
        let possible_paths = get_windows_node_paths();
        for path in possible_paths {
            if std::path::Path::new(&path).exists() {
                // 使用完整路径执行
                let cmd = format!("\"{}\" --version", path);
                if let Ok(output) = shell::run_cmd_output(&cmd) {
                    let version = output.trim().to_string();
                    if !version.is_empty() && version.starts_with('v') {
                        info!("[环境检查] 在 {} 找到 Node.js: {}", path, version);
                        return Some(version);
                    }
                }
            }
        }

        None
    } else {
        // 先尝试直接调用
        if let Ok(v) = shell::run_command_output("node", &["--version"]) {
            return Some(v.trim().to_string());
        }

        // 检测常见的 Node.js 安装路径（macOS/Linux）
        let possible_paths = get_unix_node_paths();
        for path in possible_paths {
            if std::path::Path::new(&path).exists() {
                if let Ok(output) = shell::run_command_output(&path, &["--version"]) {
                    info!("[环境检查] 在 {} 找到 Node.js: {}", path, output.trim());
                    return Some(output.trim().to_string());
                }
            }
        }

        // 尝试通过 shell 加载用户环境来检测
        if let Ok(output) = shell::run_bash_output("source ~/.zshrc 2>/dev/null || source ~/.bashrc 2>/dev/null; node --version 2>/dev/null") {
            if !output.is_empty() && output.starts_with('v') {
                info!("[环境检查] 通过用户 shell 找到 Node.js: {}", output.trim());
                return Some(output.trim().to_string());
            }
        }

        None
    }
}

/// 获取 Unix 系统上可能的 Node.js 路径
fn get_unix_node_paths() -> Vec<String> {
    let mut paths = Vec::new();

    if let Some(path) = shell::get_managed_node_path() {
        paths.push(path);
    }

    // Homebrew (macOS)
    paths.push("/opt/homebrew/bin/node".to_string()); // Apple Silicon
    paths.push("/usr/local/bin/node".to_string()); // Intel Mac

    // 系统安装
    paths.push("/usr/bin/node".to_string());

    // nvm (检查常见版本)
    if let Some(home) = dirs::home_dir() {
        let home_str = home.display().to_string();

        // nvm 默认版本
        paths.push(format!("{}/.nvm/versions/node/v22.0.0/bin/node", home_str));
        paths.push(format!("{}/.nvm/versions/node/v22.1.0/bin/node", home_str));
        paths.push(format!("{}/.nvm/versions/node/v22.2.0/bin/node", home_str));
        paths.push(format!("{}/.nvm/versions/node/v22.11.0/bin/node", home_str));
        paths.push(format!("{}/.nvm/versions/node/v22.12.0/bin/node", home_str));
        paths.push(format!("{}/.nvm/versions/node/v23.0.0/bin/node", home_str));

        // 尝试 nvm alias default（读取 nvm 的 default alias）
        let nvm_default = format!("{}/.nvm/alias/default", home_str);
        if let Ok(version) = std::fs::read_to_string(&nvm_default) {
            let version = version.trim();
            if !version.is_empty() {
                paths.insert(
                    0,
                    format!("{}/.nvm/versions/node/v{}/bin/node", home_str, version),
                );
            }
        }

        // fnm
        paths.push(format!("{}/.fnm/aliases/default/bin/node", home_str));

        // volta
        paths.push(format!("{}/.volta/bin/node", home_str));

        // asdf
        paths.push(format!("{}/.asdf/shims/node", home_str));

        // mise (formerly rtx)
        paths.push(format!("{}/.local/share/mise/shims/node", home_str));
    }

    paths
}

/// 获取 Windows 系统上可能的 Node.js 路径
fn get_windows_node_paths() -> Vec<String> {
    let mut paths = Vec::new();

    if let Some(path) = shell::get_managed_node_path() {
        paths.push(path);
    }

    // 1. 标准安装路径 (Program Files)
    paths.push("C:\\Program Files\\nodejs\\node.exe".to_string());
    paths.push("C:\\Program Files (x86)\\nodejs\\node.exe".to_string());

    // 2. nvm for Windows (nvm4w) - 常见安装位置
    paths.push("C:\\nvm4w\\nodejs\\node.exe".to_string());

    // 3. 用户目录下的各种安装
    if let Some(home) = dirs::home_dir() {
        let home_str = home.display().to_string();

        // nvm for Windows 用户安装
        paths.push(format!(
            "{}\\AppData\\Roaming\\nvm\\current\\node.exe",
            home_str
        ));

        // fnm (Fast Node Manager) for Windows
        paths.push(format!(
            "{}\\AppData\\Roaming\\fnm\\aliases\\default\\node.exe",
            home_str
        ));
        paths.push(format!(
            "{}\\AppData\\Local\\fnm\\aliases\\default\\node.exe",
            home_str
        ));
        paths.push(format!("{}\\.fnm\\aliases\\default\\node.exe", home_str));

        // volta
        paths.push(format!(
            "{}\\AppData\\Local\\Volta\\bin\\node.exe",
            home_str
        ));
        // volta 通过 shim 调用，检查 bin 目录即可

        // scoop 安装
        paths.push(format!(
            "{}\\scoop\\apps\\nodejs\\current\\node.exe",
            home_str
        ));
        paths.push(format!(
            "{}\\scoop\\apps\\nodejs-lts\\current\\node.exe",
            home_str
        ));

        // chocolatey 安装
        paths.push("C:\\ProgramData\\chocolatey\\lib\\nodejs\\tools\\node.exe".to_string());
    }

    // 4. 从注册表读取的安装路径（通过环境变量间接获取）
    if let Ok(program_files) = std::env::var("ProgramFiles") {
        paths.push(format!("{}\\nodejs\\node.exe", program_files));
    }
    if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
        paths.push(format!("{}\\nodejs\\node.exe", program_files_x86));
    }

    // 5. nvm-windows 的符号链接路径（NVM_SYMLINK 环境变量）
    if let Ok(nvm_symlink) = std::env::var("NVM_SYMLINK") {
        paths.insert(0, format!("{}\\node.exe", nvm_symlink));
    }

    // 6. nvm-windows 的 NVM_HOME 路径下的当前版本
    if let Ok(nvm_home) = std::env::var("NVM_HOME") {
        // 尝试读取当前激活的版本
        let settings_path = format!("{}\\settings.txt", nvm_home);
        if let Ok(content) = std::fs::read_to_string(&settings_path) {
            for line in content.lines() {
                if line.starts_with("current:") {
                    if let Some(version) = line.strip_prefix("current:") {
                        let version = version.trim();
                        if !version.is_empty() {
                            paths.insert(0, format!("{}\\v{}\\node.exe", nvm_home, version));
                        }
                    }
                }
            }
        }
    }

    paths
}

/// 获取 OpenClaw 版本
fn get_openclaw_version() -> Option<String> {
    // 使用 run_openclaw 统一处理各平台
    shell::run_openclaw(&["--version"])
        .ok()
        .map(|v| v.trim().to_string())
}

/// 检查 Node.js 版本是否 >= 22
fn check_node_version_requirement(version: &Option<String>) -> bool {
    if let Some(v) = version {
        // 解析版本号 "v22.1.0" -> 22
        let major = normalize_version(v)
            .split('.')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        major >= REQUIRED_NODE_MAJOR
    } else {
        false
    }
}

/// 安装 Node.js
#[command]
pub async fn install_nodejs() -> Result<InstallResult, String> {
    info!("[安装Node.js] 开始安装 Node.js...");
    let os = platform::get_os();
    info!("[安装Node.js] 检测到操作系统: {}", os);

    let result = match os.as_str() {
        "windows" => {
            info!("[安装Node.js] 使用 Windows 安装方式...");
            install_nodejs_windows().await
        }
        "macos" => {
            info!("[安装Node.js] 使用 macOS 安装方式 (镜像优先)...");
            install_nodejs_macos().await
        }
        "linux" => {
            info!("[安装Node.js] 使用 Linux 安装方式...");
            install_nodejs_linux().await
        }
        _ => {
            error!("[安装Node.js] 不支持的操作系统: {}", os);
            Ok(InstallResult {
                success: false,
                message: "不支持的操作系统".to_string(),
                error: Some(format!("不支持的操作系统: {}", os)),
            })
        }
    };

    match &result {
        Ok(r) if r.success => info!("[安装Node.js] ✓ 安装成功"),
        Ok(r) => warn!("[安装Node.js] ✗ 安装失败: {}", r.message),
        Err(e) => error!("[安装Node.js] ✗ 安装错误: {}", e),
    }

    result
}

/// Windows 安装 Node.js
async fn install_nodejs_windows() -> Result<InstallResult, String> {
    if let Some(version) = get_node_version() {
        if check_node_version_requirement(&Some(version.clone())) {
            return Ok(install_success(format!("Node.js 已可用: {}", version)));
        }
    }

    let paths = ManagedInstallPaths::new();
    if let Err(err) = ensure_managed_runtime_layout(&paths)
        .and_then(|_| ensure_windows_node_runtime(&paths))
        .and_then(|_| ensure_windows_git_runtime(&paths))
    {
        return Ok(install_failure("Node.js 安装失败", err));
    }

    configure_user_npm_registry();

    match get_node_version() {
        Some(version) if check_node_version_requirement(&Some(version.clone())) => {
            Ok(install_success(format!(
                "Node.js 已通过托管运行时安装成功: {}。同时已准备 MinGit 与 npm 国内镜像。",
                version
            )))
        }
        Some(version) => Ok(install_failure(
            "Node.js 安装完成但版本不满足要求",
            format!(
                "当前版本 {}，需要 {}",
                version,
                required_node_version_hint()
            ),
        )),
        None => Ok(install_failure(
            "Node.js 安装失败",
            "托管运行时下载完成，但应用仍未检测到 node.exe".to_string(),
        )),
    }
}

/// macOS 安装 Node.js
async fn install_nodejs_macos() -> Result<InstallResult, String> {
    if let Some(version) = get_node_version() {
        if check_node_version_requirement(&Some(version.clone())) {
            return Ok(install_success(format!("Node.js 已可用: {}", version)));
        }
    }

    let paths = ManagedInstallPaths::new();
    if let Err(err) =
        ensure_managed_runtime_layout(&paths).and_then(|_| ensure_macos_node_runtime(&paths))
    {
        return Ok(install_failure(
            "Node.js 安装失败",
            format!(
                "{}。说明：Homebrew 官方安装器依赖 Xcode CLT 且交互较重，不适合在 GUI 里静默引导；当前已改为优先使用 npmmirror 托管运行时。",
                err
            ),
        ));
    }

    configure_user_npm_registry();

    match get_node_version() {
        Some(version) if check_node_version_requirement(&Some(version.clone())) => {
            Ok(install_success(format!(
                "Node.js 已通过托管运行时安装成功: {}。已同步配置 npm 国内镜像。",
                version
            )))
        }
        Some(version) => Ok(install_failure(
            "Node.js 安装完成但版本不满足要求",
            format!(
                "当前版本 {}，需要 {}",
                version,
                required_node_version_hint()
            ),
        )),
        None => Ok(install_failure(
            "Node.js 安装失败",
            "镜像下载完成，但应用仍未检测到托管 Node 运行时".to_string(),
        )),
    }
}

/// Linux 安装 Node.js
async fn install_nodejs_linux() -> Result<InstallResult, String> {
    // 使用 NodeSource 仓库安装
    let script = r#"
# 检测包管理器
if command -v apt-get &> /dev/null; then
    echo "检测到 apt，使用 NodeSource 仓库..."
    curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
    sudo apt-get install -y nodejs
elif command -v dnf &> /dev/null; then
    echo "检测到 dnf，使用 NodeSource 仓库..."
    curl -fsSL https://rpm.nodesource.com/setup_22.x | sudo bash -
    sudo dnf install -y nodejs
elif command -v yum &> /dev/null; then
    echo "检测到 yum，使用 NodeSource 仓库..."
    curl -fsSL https://rpm.nodesource.com/setup_22.x | sudo bash -
    sudo yum install -y nodejs
elif command -v pacman &> /dev/null; then
    echo "检测到 pacman..."
    sudo pacman -S nodejs npm --noconfirm
else
    echo "无法检测到支持的包管理器"
    exit 1
fi

# 验证安装
node --version
"#;

    match shell::run_bash_output(script) {
        Ok(output) => {
            configure_user_npm_registry();
            Ok(install_success(format!("Node.js 安装成功！{}", output)))
        }
        Err(e) => Ok(install_failure("Node.js 安装失败", e)),
    }
}

/// 安装 OpenClaw
#[command]
pub async fn install_openclaw() -> Result<InstallResult, String> {
    info!("[安装OpenClaw] 开始安装 OpenClaw...");
    let os = platform::get_os();
    info!("[安装OpenClaw] 检测到操作系统: {}", os);

    let result = match os.as_str() {
        "windows" => {
            info!("[安装OpenClaw] 使用 Windows 安装方式...");
            install_openclaw_windows().await
        }
        _ => {
            info!("[安装OpenClaw] 使用 Unix 安装方式 (托管本地安装)...");
            install_openclaw_unix().await
        }
    };

    match &result {
        Ok(r) if r.success => info!("[安装OpenClaw] ✓ 安装成功"),
        Ok(r) => warn!("[安装OpenClaw] ✗ 安装失败: {}", r.message),
        Err(e) => error!("[安装OpenClaw] ✗ 安装错误: {}", e),
    }

    result
}

/// Windows 安装 OpenClaw
async fn install_openclaw_windows() -> Result<InstallResult, String> {
    let paths = ManagedInstallPaths::new();

    if let Err(err) = ensure_managed_runtime_layout(&paths)
        .and_then(|_| ensure_windows_node_runtime(&paths))
        .and_then(|_| ensure_windows_git_runtime(&paths))
    {
        return Ok(install_failure("OpenClaw 安装失败", err));
    }

    match install_openclaw_managed(&paths) {
        Ok(output) => {
            if let Some(version) = get_openclaw_version() {
                Ok(install_success(format!(
                    "OpenClaw 已安装到应用托管目录: {}。{}",
                    version, output
                )))
            } else {
                Ok(install_failure(
                    "OpenClaw 安装失败",
                    format!(
                        "npm 安装命令已执行，但未检测到 openclaw 可执行文件。\n{}",
                        output
                    ),
                ))
            }
        }
        Err(err) => Ok(install_failure("OpenClaw 安装失败", err)),
    }
}

/// Unix 系统安装 OpenClaw
async fn install_openclaw_unix() -> Result<InstallResult, String> {
    if get_node_version().is_none() {
        return Ok(install_failure(
            "OpenClaw 安装失败",
            format!("请先安装 Node.js {}", required_node_version_hint()),
        ));
    }

    let paths = ManagedInstallPaths::new();
    match install_openclaw_managed(&paths) {
        Ok(output) => {
            if let Some(version) = get_openclaw_version() {
                Ok(install_success(format!(
                    "OpenClaw 已安装到应用托管目录: {}。{}",
                    version, output
                )))
            } else {
                Ok(install_failure(
                    "OpenClaw 安装失败",
                    format!(
                        "npm 安装命令已执行，但未检测到 openclaw 可执行文件。\n{}",
                        output
                    ),
                ))
            }
        }
        Err(err) => Ok(install_failure("OpenClaw 安装失败", err)),
    }
}

/// 初始化 OpenClaw 配置
#[command]
pub async fn init_openclaw_config() -> Result<InstallResult, String> {
    info!("[初始化配置] 开始初始化 OpenClaw 配置...");

    let config_dir = platform::get_config_dir();
    info!("[初始化配置] 配置目录: {}", config_dir);

    // 创建配置目录
    info!("[初始化配置] 创建配置目录...");
    if let Err(e) = std::fs::create_dir_all(&config_dir) {
        error!("[初始化配置] ✗ 创建配置目录失败: {}", e);
        return Ok(InstallResult {
            success: false,
            message: "创建配置目录失败".to_string(),
            error: Some(e.to_string()),
        });
    }

    // 创建子目录
    let subdirs = ["agents/main/sessions", "agents/main/agent", "credentials"];
    for subdir in subdirs {
        let path = format!("{}/{}", config_dir, subdir);
        info!("[初始化配置] 创建子目录: {}", subdir);
        if let Err(e) = std::fs::create_dir_all(&path) {
            error!("[初始化配置] ✗ 创建目录失败: {} - {}", subdir, e);
            return Ok(InstallResult {
                success: false,
                message: format!("创建目录失败: {}", subdir),
                error: Some(e.to_string()),
            });
        }
    }

    // 设置配置目录权限为 700（与 shell 脚本 chmod 700 一致）
    // 仅在 Unix 系统上执行
    #[cfg(unix)]
    {
        info!("[初始化配置] 设置目录权限为 700...");
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&config_dir) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o700);
            if let Err(e) = std::fs::set_permissions(&config_dir, perms) {
                warn!("[初始化配置] 设置权限失败: {}", e);
            } else {
                info!("[初始化配置] ✓ 权限设置成功");
            }
        }
    }

    // 设置 gateway mode 为 local
    info!("[初始化配置] 执行: openclaw config set gateway.mode local");
    let result = shell::run_openclaw(&["config", "set", "gateway.mode", "local"]);

    match result {
        Ok(output) => {
            info!("[初始化配置] ✓ 配置初始化成功");
            debug!("[初始化配置] 命令输出: {}", output);
            Ok(InstallResult {
                success: true,
                message: "配置初始化成功！".to_string(),
                error: None,
            })
        }
        Err(e) => {
            error!("[初始化配置] ✗ 配置初始化失败: {}", e);
            Ok(InstallResult {
                success: false,
                message: "配置初始化失败".to_string(),
                error: Some(e),
            })
        }
    }
}

/// 打开终端执行安装脚本（用于需要管理员权限的场景）
#[command]
pub async fn open_install_terminal(install_type: String) -> Result<String, String> {
    match install_type.as_str() {
        "nodejs" => open_nodejs_install_terminal().await,
        "openclaw" => open_openclaw_install_terminal().await,
        _ => Err(format!("未知的安装类型: {}", install_type)),
    }
}

/// 打开终端安装 Node.js
async fn open_nodejs_install_terminal() -> Result<String, String> {
    if platform::is_windows() {
        // Windows: 打开 PowerShell 执行安装
        let script = r#"
Start-Process powershell -ArgumentList '-NoExit', '-Command', '
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "    Node.js 安装向导" -ForegroundColor White
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

Write-Host "当前版本已改为应用内自动下载 npmmirror Node 运行时。" -ForegroundColor Green
Write-Host "如果自动安装失败，请直接回到 OpenClaw Manager 再点一次安装。" -ForegroundColor Yellow
Write-Host "如需手动处理，可访问镜像目录：" -ForegroundColor Yellow
Write-Host "https://npmmirror.com/mirrors/node/" -ForegroundColor Green
Start-Process "https://npmmirror.com/mirrors/node/"

Write-Host ""
Write-Host "安装完成后请重启 OpenClaw Manager" -ForegroundColor Green
Write-Host ""
Read-Host "按回车键关闭此窗口"
' -Verb RunAs
"#;
        shell::run_powershell_output(script)?;
        Ok("已打开安装终端".to_string())
    } else if platform::is_macos() {
        // macOS: 打开 Terminal.app
        let script_content = r#"#!/bin/bash
clear
echo "========================================"
echo "    Node.js 安装向导"
echo "========================================"
echo ""

echo "当前应用已改为优先使用 npmmirror 托管 Node 运行时。"
echo "如果自动安装失败，可手动下载镜像，或在你已安装的 Homebrew 中执行："
echo "  brew install node@22"
echo "  brew link --overwrite node@22"
echo ""
echo "镜像目录: https://npmmirror.com/mirrors/node/"
open "https://npmmirror.com/mirrors/node/" >/dev/null 2>&1 || true

echo ""
echo "如已手动安装完成，可运行 node --version 验证。"
node --version 2>/dev/null || true
echo ""
read -p "按回车键关闭此窗口..."
"#;

        let script_path = "/tmp/openclaw_install_nodejs.command";
        std::fs::write(script_path, script_content).map_err(|e| format!("创建脚本失败: {}", e))?;

        std::process::Command::new("chmod")
            .args(["+x", script_path])
            .output()
            .map_err(|e| format!("设置权限失败: {}", e))?;

        std::process::Command::new("open")
            .arg(script_path)
            .spawn()
            .map_err(|e| format!("启动终端失败: {}", e))?;

        Ok("已打开安装终端".to_string())
    } else {
        Err("请手动安装 Node.js: https://nodejs.org/".to_string())
    }
}

/// 打开终端安装 OpenClaw
async fn open_openclaw_install_terminal() -> Result<String, String> {
    if platform::is_windows() {
        let script = r#"
Start-Process powershell -ArgumentList '-NoExit', '-Command', '
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "    OpenClaw 安装向导" -ForegroundColor White
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

Write-Host "当前版本已改为应用内自动安装 OpenClaw，并自动配置 npmmirror/Git 代理。" -ForegroundColor Green
Write-Host "这里提供手工兜底路径：" -ForegroundColor Yellow

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Host "未检测到 Git，正在尝试安装 Git for Windows..." -ForegroundColor Yellow
    if (Get-Command winget -ErrorAction SilentlyContinue) {
        winget install --id Git.Git --accept-source-agreements --accept-package-agreements
    } else {
        Write-Host "请先安装 Git: https://git-scm.com/download/win" -ForegroundColor Red
        Start-Process "https://git-scm.com/download/win"
    }
}

Write-Host "配置 npm 国内镜像..." -ForegroundColor Yellow
npm config set registry https://registry.npmmirror.com --location user

Write-Host "开始安装 OpenClaw..." -ForegroundColor Yellow
npm install -g openclaw@latest --registry https://registry.npmmirror.com

Write-Host ""
Write-Host "如安装成功，将执行基础初始化..." -ForegroundColor Green
openclaw config set gateway.mode local 2>$null
openclaw --version
Write-Host ""
Read-Host "按回车键关闭此窗口"
'
"#;
        shell::run_powershell_output(script)?;
        Ok("已打开安装终端".to_string())
    } else if platform::is_macos() {
        let script_content = r#"#!/bin/bash
clear
echo "========================================"
echo "    OpenClaw 安装向导"
echo "========================================"
echo ""

echo "当前版本已改为应用内托管安装 OpenClaw，并优先使用 npmmirror。"
echo "如果自动安装失败，请确认："
echo "1. Node.js 已安装且版本 >= 22"
echo "2. git 可用（macOS 可执行 xcode-select --install）"
echo ""
echo "如需手动安装，可执行："
echo "npm config set registry https://registry.npmmirror.com --location user"
echo "npm install -g openclaw@latest --registry https://registry.npmmirror.com"

echo ""
echo "初始化配置..."
openclaw config set gateway.mode local 2>/dev/null || true

mkdir -p ~/.openclaw/agents/main/sessions
mkdir -p ~/.openclaw/agents/main/agent
mkdir -p ~/.openclaw/credentials

echo ""
echo "安装完成！"
openclaw --version
echo ""
read -p "按回车键关闭此窗口..."
"#;

        let script_path = "/tmp/openclaw_install_openclaw.command";
        std::fs::write(script_path, script_content).map_err(|e| format!("创建脚本失败: {}", e))?;

        std::process::Command::new("chmod")
            .args(["+x", script_path])
            .output()
            .map_err(|e| format!("设置权限失败: {}", e))?;

        std::process::Command::new("open")
            .arg(script_path)
            .spawn()
            .map_err(|e| format!("启动终端失败: {}", e))?;

        Ok("已打开安装终端".to_string())
    } else {
        // Linux
        let script_content = r#"#!/bin/bash
clear
echo "========================================"
echo "    OpenClaw 安装向导"
echo "========================================"
echo ""

echo "当前版本已改为应用内托管安装 OpenClaw，并优先使用 npmmirror。"
echo "如果自动安装失败，请确认 Node.js 与 git 已安装。"
echo ""
echo "如需手动安装，可执行："
echo "npm config set registry https://registry.npmmirror.com --location user"
echo "npm install -g openclaw@latest --registry https://registry.npmmirror.com"

echo ""
echo "初始化配置..."
openclaw config set gateway.mode local 2>/dev/null || true

mkdir -p ~/.openclaw/agents/main/sessions
mkdir -p ~/.openclaw/agents/main/agent
mkdir -p ~/.openclaw/credentials

echo ""
echo "安装完成！"
openclaw --version
echo ""
read -p "按回车键关闭..."
"#;

        let script_path = "/tmp/openclaw_install_openclaw.sh";
        std::fs::write(script_path, script_content).map_err(|e| format!("创建脚本失败: {}", e))?;

        std::process::Command::new("chmod")
            .args(["+x", script_path])
            .output()
            .map_err(|e| format!("设置权限失败: {}", e))?;

        // 尝试不同的终端
        let terminals = ["gnome-terminal", "xfce4-terminal", "konsole", "xterm"];
        for term in terminals {
            if std::process::Command::new(term)
                .args(["--", script_path])
                .spawn()
                .is_ok()
            {
                return Ok("已打开安装终端".to_string());
            }
        }

        Err("无法启动终端，请手动运行: npm install -g openclaw@latest --registry https://registry.npmmirror.com".to_string())
    }
}

/// 卸载 OpenClaw
#[command]
pub async fn uninstall_openclaw() -> Result<InstallResult, String> {
    info!("[卸载OpenClaw] 开始卸载 OpenClaw...");
    let os = platform::get_os();
    info!("[卸载OpenClaw] 检测到操作系统: {}", os);

    // 先停止服务
    info!("[卸载OpenClaw] 尝试停止服务...");
    let _ = shell::run_openclaw(&["gateway", "stop"]);
    std::thread::sleep(std::time::Duration::from_millis(500));

    let result = match os.as_str() {
        "windows" => {
            info!("[卸载OpenClaw] 使用 Windows 卸载方式...");
            uninstall_openclaw_windows().await
        }
        _ => {
            info!("[卸载OpenClaw] 使用 Unix 卸载方式 (npm)...");
            uninstall_openclaw_unix().await
        }
    };

    match &result {
        Ok(r) if r.success => info!("[卸载OpenClaw] ✓ 卸载成功"),
        Ok(r) => warn!("[卸载OpenClaw] ✗ 卸载失败: {}", r.message),
        Err(e) => error!("[卸载OpenClaw] ✗ 卸载错误: {}", e),
    }

    result
}

/// Windows 卸载 OpenClaw
async fn uninstall_openclaw_windows() -> Result<InstallResult, String> {
    let paths = ManagedInstallPaths::new();
    let result = if shell::get_managed_openclaw_path().is_some() {
        uninstall_openclaw_managed(&paths)
    } else {
        run_npm_command(&["uninstall", "-g", "openclaw"], None, &[])
    };

    match result {
        Ok(output) => {
            std::thread::sleep(Duration::from_millis(500));
            if get_openclaw_version().is_none() {
                Ok(install_success(format!("OpenClaw 已成功卸载。{}", output)))
            } else {
                Ok(install_failure(
                    "OpenClaw 卸载未完成",
                    format!("卸载命令已执行，但仍检测到 openclaw。\n{}", output),
                ))
            }
        }
        Err(err) => Ok(install_failure("OpenClaw 卸载失败", err)),
    }
}

/// Unix 系统卸载 OpenClaw
async fn uninstall_openclaw_unix() -> Result<InstallResult, String> {
    let paths = ManagedInstallPaths::new();
    let result = if shell::get_managed_openclaw_path().is_some() {
        uninstall_openclaw_managed(&paths)
    } else {
        run_npm_command(&["uninstall", "-g", "openclaw"], None, &[])
    };

    match result {
        Ok(output) => {
            std::thread::sleep(Duration::from_millis(500));
            if get_openclaw_version().is_none() {
                Ok(install_success(format!("OpenClaw 已成功卸载。{}", output)))
            } else {
                Ok(install_failure(
                    "OpenClaw 卸载未完成",
                    format!("卸载命令已执行，但仍检测到 openclaw。\n{}", output),
                ))
            }
        }
        Err(err) => Ok(install_failure("OpenClaw 卸载失败", err)),
    }
}

/// 版本更新信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    /// 是否有更新可用
    pub update_available: bool,
    /// 当前版本
    pub current_version: Option<String>,
    /// 最新版本
    pub latest_version: Option<String>,
    /// 错误信息
    pub error: Option<String>,
}

/// 检查 OpenClaw 更新
#[command]
pub async fn check_openclaw_update() -> Result<UpdateInfo, String> {
    info!("[版本检查] 开始检查 OpenClaw 更新...");

    // 获取当前版本
    let current_version = get_openclaw_version();
    info!("[版本检查] 当前版本: {:?}", current_version);

    if current_version.is_none() {
        info!("[版本检查] OpenClaw 未安装");
        return Ok(UpdateInfo {
            update_available: false,
            current_version: None,
            latest_version: None,
            error: Some("OpenClaw 未安装".to_string()),
        });
    }

    // 获取最新版本
    let latest_version = get_latest_openclaw_version();
    info!("[版本检查] 最新版本: {:?}", latest_version);

    if latest_version.is_none() {
        return Ok(UpdateInfo {
            update_available: false,
            current_version,
            latest_version: None,
            error: Some("无法获取最新版本信息".to_string()),
        });
    }

    // 比较版本
    let current = current_version.clone().unwrap();
    let latest = latest_version.clone().unwrap();
    let update_available = compare_versions(&current, &latest);

    info!("[版本检查] 是否有更新: {}", update_available);

    Ok(UpdateInfo {
        update_available,
        current_version,
        latest_version,
        error: None,
    })
}

/// 获取 npm registry 上的最新版本
fn get_latest_openclaw_version() -> Option<String> {
    let result = run_npm_command(
        &[
            "view",
            "openclaw",
            "version",
            "--registry",
            NPM_REGISTRY_MIRROR,
        ],
        None,
        &[("npm_config_registry", NPM_REGISTRY_MIRROR.to_string())],
    );

    match result {
        Ok(version) => {
            let v = version.trim().to_string();
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        }
        Err(e) => {
            warn!("[版本检查] 获取最新版本失败: {}", e);
            None
        }
    }
}

/// 比较版本号，返回是否有更新可用
/// current: 当前版本 (如 "1.0.0" 或 "v1.0.0")
/// latest: 最新版本 (如 "1.0.1")
fn compare_versions(current: &str, latest: &str) -> bool {
    // 移除可能的 'v' 前缀和空白
    let current = current.trim().trim_start_matches('v');
    let latest = latest.trim().trim_start_matches('v');

    // 分割版本号
    let current_parts: Vec<u32> = current.split('.').filter_map(|s| s.parse().ok()).collect();
    let latest_parts: Vec<u32> = latest.split('.').filter_map(|s| s.parse().ok()).collect();

    // 比较每个部分
    for i in 0..3 {
        let c = current_parts.get(i).unwrap_or(&0);
        let l = latest_parts.get(i).unwrap_or(&0);
        if l > c {
            return true;
        } else if l < c {
            return false;
        }
    }

    false
}

/// 更新 OpenClaw
#[command]
pub async fn update_openclaw() -> Result<InstallResult, String> {
    info!("[更新OpenClaw] 开始更新 OpenClaw...");
    let os = platform::get_os();

    // 先停止服务
    info!("[更新OpenClaw] 尝试停止服务...");
    let _ = shell::run_openclaw(&["gateway", "stop"]);
    std::thread::sleep(std::time::Duration::from_millis(500));

    let result = match os.as_str() {
        "windows" => {
            info!("[更新OpenClaw] 使用 Windows 更新方式...");
            update_openclaw_windows().await
        }
        _ => {
            info!("[更新OpenClaw] 使用 Unix 更新方式 (托管本地安装)...");
            update_openclaw_unix().await
        }
    };

    match &result {
        Ok(r) if r.success => info!("[更新OpenClaw] ✓ 更新成功"),
        Ok(r) => warn!("[更新OpenClaw] ✗ 更新失败: {}", r.message),
        Err(e) => error!("[更新OpenClaw] ✗ 更新错误: {}", e),
    }

    result
}

/// Windows 更新 OpenClaw
async fn update_openclaw_windows() -> Result<InstallResult, String> {
    let paths = ManagedInstallPaths::new();
    if let Err(err) = ensure_managed_runtime_layout(&paths)
        .and_then(|_| ensure_windows_node_runtime(&paths))
        .and_then(|_| ensure_windows_git_runtime(&paths))
    {
        return Ok(install_failure("OpenClaw 更新失败", err));
    }

    match update_openclaw_managed(&paths) {
        Ok(output) => {
            let new_version = get_openclaw_version().unwrap_or_else(|| "最新版本".to_string());
            Ok(install_success(format!(
                "OpenClaw 已更新到 {}。{}",
                new_version, output
            )))
        }
        Err(err) => Ok(install_failure("OpenClaw 更新失败", err)),
    }
}

/// Unix 系统更新 OpenClaw
async fn update_openclaw_unix() -> Result<InstallResult, String> {
    let paths = ManagedInstallPaths::new();
    match update_openclaw_managed(&paths) {
        Ok(output) => {
            let new_version = get_openclaw_version().unwrap_or_else(|| "最新版本".to_string());
            Ok(install_success(format!(
                "OpenClaw 已更新到 {}。{}",
                new_version, output
            )))
        }
        Err(err) => Ok(install_failure("OpenClaw 更新失败", err)),
    }
}
