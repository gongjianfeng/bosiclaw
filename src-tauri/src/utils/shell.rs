use crate::utils::{file, platform};
use log::{debug, info, warn};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Windows CREATE_NO_WINDOW 标志，用于隐藏控制台窗口
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone)]
enum NodeRuntime {
    Bundled { root: PathBuf, path: PathBuf },
    System(PathBuf),
}

#[derive(Debug, Clone)]
struct BundledOpenClawRuntime {
    runtime_root: PathBuf,
    node_root: PathBuf,
    node_path: PathBuf,
    package_root: PathBuf,
    entry_path: PathBuf,
}

#[derive(Debug, Clone)]
struct SystemOpenClawRuntime {
    executable_path: PathBuf,
}

#[derive(Debug, Clone)]
enum OpenClawRuntime {
    Bundled(BundledOpenClawRuntime),
    System(SystemOpenClawRuntime),
}

impl NodeRuntime {
    fn path(&self) -> &Path {
        match self {
            Self::Bundled { path, .. } => path,
            Self::System(path) => path,
        }
    }
}

impl OpenClawRuntime {
    fn mode(&self) -> &'static str {
        match self {
            Self::Bundled(_) => "bundled",
            Self::System(_) => "system",
        }
    }

    fn display_path(&self) -> String {
        match self {
            Self::Bundled(runtime) => runtime.entry_path.display().to_string(),
            Self::System(runtime) => runtime.executable_path.display().to_string(),
        }
    }

    fn runtime_root(&self) -> Option<&Path> {
        match self {
            Self::Bundled(runtime) => Some(&runtime.runtime_root),
            Self::System(_) => None,
        }
    }
}

fn apply_common_env(command: &mut Command) {
    command.env("PATH", get_extended_path());

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
}

fn output_to_string(result: io::Result<Output>) -> Result<String, String> {
    match result {
        Ok(output) => {
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                if stderr.is_empty() {
                    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if stdout.is_empty() {
                        Err(format!(
                            "Command failed with exit code: {:?}",
                            output.status.code()
                        ))
                    } else {
                        Err(stdout)
                    }
                } else {
                    Err(stderr)
                }
            }
        }
        Err(error) => Err(error.to_string()),
    }
}

fn normalize_lines(output: &str) -> Vec<String> {
    output
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

fn first_existing_path(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    for candidate in candidates {
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn ensure_executable(path: &Path) {
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        let mode = permissions.mode();
        if mode & 0o111 == 0 {
            permissions.set_mode(mode | 0o755);
            let _ = std::fs::set_permissions(path, permissions);
        }
    }
}

#[cfg(not(unix))]
fn ensure_executable(_path: &Path) {}

fn resolve_bundled_node_runtime() -> Option<NodeRuntime> {
    let runtime_root = platform::get_bundled_runtime_dir()?;
    let node_root_candidates = [
        runtime_root.join("tools").join("node"),
        runtime_root.join("node"),
        runtime_root.join("nodejs"),
    ];

    for node_root in node_root_candidates {
        if !node_root.exists() {
            continue;
        }

        let node_path = if platform::is_windows() {
            first_existing_path([
                node_root.join("node.exe"),
                node_root.join("bin").join("node.exe"),
            ])
        } else {
            first_existing_path([node_root.join("bin").join("node"), node_root.join("node")])
        };

        if let Some(node_path) = node_path {
            ensure_executable(&node_path);
            return Some(NodeRuntime::Bundled {
                root: node_root,
                path: node_path,
            });
        }
    }

    None
}

fn resolve_node_from_path() -> Option<PathBuf> {
    if platform::is_windows() {
        if let Ok(output) = run_cmd_output("where node") {
            for line in normalize_lines(&output) {
                let path = PathBuf::from(line);
                if path.exists() {
                    return Some(path);
                }
            }
        }
    } else if let Ok(output) = run_command_output("which", &["node"]) {
        for line in normalize_lines(&output) {
            let path = PathBuf::from(line);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn resolve_system_node_runtime() -> Option<NodeRuntime> {
    if let Some(path) = resolve_node_from_path() {
        return Some(NodeRuntime::System(path));
    }

    let candidate_paths = if platform::is_windows() {
        get_windows_node_paths()
    } else {
        get_unix_node_paths()
    };

    first_existing_path(candidate_paths.into_iter().map(PathBuf::from)).map(NodeRuntime::System)
}

fn resolve_node_runtime() -> Option<NodeRuntime> {
    resolve_bundled_node_runtime().or_else(resolve_system_node_runtime)
}

fn resolve_bundled_openclaw_runtime() -> Option<BundledOpenClawRuntime> {
    let runtime_root = platform::get_bundled_runtime_dir()?;
    let node_runtime = resolve_bundled_node_runtime()?;
    let (node_root, node_path) = match node_runtime {
        NodeRuntime::Bundled { root, path } => (root, path),
        NodeRuntime::System(_) => return None,
    };

    let package_root_candidates = [
        runtime_root
            .join("lib")
            .join("node_modules")
            .join("openclaw"),
        runtime_root.join("openclaw"),
        runtime_root.join("packages").join("openclaw"),
    ];

    for package_root in package_root_candidates {
        if !package_root.exists() {
            continue;
        }

        let entry_path = first_existing_path([
            package_root.join("openclaw.mjs"),
            package_root.join("dist").join("entry.js"),
        ]);

        if let Some(entry_path) = entry_path {
            ensure_executable(&entry_path);
            return Some(BundledOpenClawRuntime {
                runtime_root: runtime_root.clone(),
                node_root: node_root.clone(),
                node_path: node_path.clone(),
                package_root,
                entry_path,
            });
        }
    }

    None
}

fn resolve_system_openclaw_runtime() -> Option<OpenClawRuntime> {
    let path = if platform::is_windows() {
        first_existing_path(get_windows_openclaw_paths().into_iter().map(PathBuf::from))
    } else {
        first_existing_path(get_unix_openclaw_paths().into_iter().map(PathBuf::from))
    };

    if let Some(path) = path {
        return Some(OpenClawRuntime::System(SystemOpenClawRuntime {
            executable_path: path,
        }));
    }

    if command_exists("openclaw") {
        return Some(OpenClawRuntime::System(SystemOpenClawRuntime {
            executable_path: PathBuf::from("openclaw"),
        }));
    }

    if !platform::is_windows() {
        if let Ok(path) =
            run_bash_output("source ~/.zshrc 2>/dev/null || source ~/.bashrc 2>/dev/null; which openclaw 2>/dev/null")
        {
            for line in normalize_lines(&path) {
                let candidate = PathBuf::from(line);
                if candidate.exists() {
                    return Some(OpenClawRuntime::System(SystemOpenClawRuntime {
                        executable_path: candidate,
                    }));
                }
            }
        }
    }

    None
}

fn resolve_openclaw_runtime() -> Option<OpenClawRuntime> {
    resolve_bundled_openclaw_runtime()
        .map(OpenClawRuntime::Bundled)
        .or_else(resolve_system_openclaw_runtime)
}

fn build_openclaw_command(runtime: &OpenClawRuntime, args: &[&str]) -> Command {
    let mut command = match runtime {
        OpenClawRuntime::Bundled(runtime) => {
            let mut command = Command::new(&runtime.node_path);
            command
                .arg(&runtime.entry_path)
                .current_dir(&runtime.package_root);
            command
        }
        OpenClawRuntime::System(runtime) => {
            let executable = runtime.executable_path.to_string_lossy().to_string();
            if executable.ends_with(".cmd") {
                let mut command = Command::new("cmd");
                command.arg("/c").arg(&runtime.executable_path);
                command
            } else {
                Command::new(&runtime.executable_path)
            }
        }
    };

    command.args(args);
    apply_common_env(&mut command);
    command
}

/// 获取扩展的 PATH 环境变量
/// GUI 应用启动时可能没有继承用户 shell 的 PATH，需要手动添加常见路径
pub fn get_extended_path() -> String {
    let mut paths = Vec::<PathBuf>::new();

    if let Some(NodeRuntime::Bundled { path, .. }) = resolve_bundled_node_runtime() {
        if let Some(parent) = path.parent() {
            paths.push(parent.to_path_buf());
        }
    }

    if !platform::is_windows() {
        paths.push(PathBuf::from("/opt/homebrew/bin"));
        paths.push(PathBuf::from("/usr/local/bin"));
        paths.push(PathBuf::from("/usr/bin"));
        paths.push(PathBuf::from("/bin"));
    }

    if let Some(home) = dirs::home_dir() {
        let nvm_default = home.join(".nvm").join("alias").join("default");
        if let Ok(version) = std::fs::read_to_string(&nvm_default) {
            let version = version.trim();
            if !version.is_empty() {
                paths.push(
                    home.join(".nvm")
                        .join("versions")
                        .join("node")
                        .join(format!("v{}", version))
                        .join("bin"),
                );
            }
        }

        for version in ["v22.22.0", "v22.12.0", "v22.11.0", "v22.0.0", "v23.0.0"] {
            let nvm_bin = home
                .join(".nvm")
                .join("versions")
                .join("node")
                .join(version)
                .join("bin");
            if nvm_bin.exists() {
                paths.push(nvm_bin);
                break;
            }
        }

        if platform::is_windows() {
            paths.push(home.join("AppData").join("Roaming").join("npm"));
            paths.push(home.join("AppData").join("Local").join("Volta").join("bin"));
        } else {
            paths.push(
                home.join(".fnm")
                    .join("aliases")
                    .join("default")
                    .join("bin"),
            );
            paths.push(home.join(".volta").join("bin"));
            paths.push(home.join(".asdf").join("shims"));
            paths.push(home.join(".local").join("share").join("mise").join("shims"));
            paths.push(home.join(".npm-global").join("bin"));
            paths.push(home.join(".pnpm").join("bin"));
            paths.push(home.join("Library").join("pnpm"));
        }
    }

    if let Some(current_path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&current_path));
    }

    let joined =
        std::env::join_paths(paths.iter()).map(|value| value.to_string_lossy().to_string());

    joined.unwrap_or_else(|_| {
        let separator = if platform::is_windows() { ';' } else { ':' };
        paths
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<String>>()
            .join(&separator.to_string())
    })
}

/// 执行 Shell 命令（带扩展 PATH）
pub fn run_command(cmd: &str, args: &[&str]) -> io::Result<Output> {
    let mut command = Command::new(cmd);
    command.args(args);
    apply_common_env(&mut command);
    command.output()
}

/// 执行 Shell 命令并获取输出字符串
pub fn run_command_output(cmd: &str, args: &[&str]) -> Result<String, String> {
    output_to_string(run_command(cmd, args))
}

/// 执行 Bash 命令（带扩展 PATH）
pub fn run_bash(script: &str) -> io::Result<Output> {
    let mut command = Command::new("bash");
    command.arg("-c").arg(script);
    apply_common_env(&mut command);
    command.output()
}

/// 执行 Bash 命令并获取输出
pub fn run_bash_output(script: &str) -> Result<String, String> {
    output_to_string(run_bash(script))
}

/// 执行 cmd.exe 命令（Windows）- 避免 PowerShell 执行策略问题
pub fn run_cmd(script: &str) -> io::Result<Output> {
    let mut cmd = Command::new("cmd");
    cmd.args(["/c", script]);
    apply_common_env(&mut cmd);
    cmd.output()
}

/// 执行 cmd.exe 命令并获取输出（Windows）
pub fn run_cmd_output(script: &str) -> Result<String, String> {
    output_to_string(run_cmd(script))
}

/// 执行 PowerShell 命令（Windows）- 仅在需要 PowerShell 特定功能时使用
/// 注意：某些 Windows 系统的 PowerShell 执行策略可能禁止运行脚本
pub fn run_powershell(script: &str) -> io::Result<Output> {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    apply_common_env(&mut cmd);
    cmd.output()
}

/// 执行 PowerShell 命令并获取输出（Windows）
pub fn run_powershell_output(script: &str) -> Result<String, String> {
    output_to_string(run_powershell(script))
}

/// 跨平台执行脚本命令
/// Windows 上使用 cmd.exe（避免 PowerShell 执行策略问题）
pub fn run_script_output(script: &str) -> Result<String, String> {
    if platform::is_windows() {
        run_cmd_output(script)
    } else {
        run_bash_output(script)
    }
}

/// 后台执行命令（不等待结果）
pub fn spawn_background(script: &str) -> io::Result<()> {
    if platform::is_windows() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", script]);
        apply_common_env(&mut cmd);
        cmd.spawn()?;
    } else {
        let mut command = Command::new("bash");
        command.arg("-c").arg(script);
        apply_common_env(&mut command);
        command.spawn()?;
    }
    Ok(())
}

/// 获取当前 OpenClaw 运行时模式
pub fn get_runtime_mode() -> String {
    resolve_openclaw_runtime()
        .map(|runtime| runtime.mode().to_string())
        .unwrap_or_else(|| "missing".to_string())
}

/// 获取内置运行时根目录
pub fn get_runtime_root() -> Option<String> {
    resolve_openclaw_runtime().and_then(|runtime| {
        runtime
            .runtime_root()
            .map(|path| path.display().to_string())
    })
}

/// 是否使用内置运行时
pub fn is_bundled_runtime_active() -> bool {
    matches!(
        resolve_openclaw_runtime(),
        Some(OpenClawRuntime::Bundled(_))
    )
}

/// 获取 Node.js 版本
pub fn get_node_version() -> Option<String> {
    let runtime = resolve_node_runtime()?;
    let version = output_to_string(Command::new(runtime.path()).arg("--version").output()).ok()?;
    if version.starts_with('v') {
        Some(version)
    } else {
        None
    }
}

/// 获取 openclaw 可执行文件路径
/// 优先返回内置运行时入口，其次回退到系统安装
pub fn get_openclaw_path() -> Option<String> {
    let runtime = resolve_openclaw_runtime()?;
    let path = runtime.display_path();
    info!("[Shell] 使用 {} 运行时: {}", runtime.mode(), path);
    Some(path)
}

/// 获取 Unix 系统上可能的 Node.js 路径
fn get_unix_node_paths() -> Vec<String> {
    let mut paths = Vec::new();
    paths.push("/opt/homebrew/bin/node".to_string());
    paths.push("/usr/local/bin/node".to_string());
    paths.push("/usr/bin/node".to_string());

    if let Some(home) = dirs::home_dir() {
        let home_str = home.display().to_string();
        paths.push(format!("{}/.nvm/versions/node/v22.0.0/bin/node", home_str));
        paths.push(format!("{}/.nvm/versions/node/v22.1.0/bin/node", home_str));
        paths.push(format!("{}/.nvm/versions/node/v22.2.0/bin/node", home_str));
        paths.push(format!("{}/.nvm/versions/node/v22.11.0/bin/node", home_str));
        paths.push(format!("{}/.nvm/versions/node/v22.12.0/bin/node", home_str));
        paths.push(format!("{}/.nvm/versions/node/v23.0.0/bin/node", home_str));

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

        paths.push(format!("{}/.fnm/aliases/default/bin/node", home_str));
        paths.push(format!("{}/.volta/bin/node", home_str));
        paths.push(format!("{}/.asdf/shims/node", home_str));
        paths.push(format!("{}/.local/share/mise/shims/node", home_str));
    }

    paths
}

/// 获取 Windows 系统上可能的 Node.js 路径
fn get_windows_node_paths() -> Vec<String> {
    let mut paths = Vec::new();
    paths.push("C:\\Program Files\\nodejs\\node.exe".to_string());
    paths.push("C:\\Program Files (x86)\\nodejs\\node.exe".to_string());
    paths.push("C:\\nvm4w\\nodejs\\node.exe".to_string());

    if let Some(home) = dirs::home_dir() {
        let home_str = home.display().to_string();
        paths.push(format!(
            "{}\\AppData\\Roaming\\nvm\\current\\node.exe",
            home_str
        ));
        paths.push(format!(
            "{}\\AppData\\Roaming\\fnm\\aliases\\default\\node.exe",
            home_str
        ));
        paths.push(format!(
            "{}\\AppData\\Local\\fnm\\aliases\\default\\node.exe",
            home_str
        ));
        paths.push(format!("{}\\.fnm\\aliases\\default\\node.exe", home_str));
        paths.push(format!(
            "{}\\AppData\\Local\\Volta\\bin\\node.exe",
            home_str
        ));
        paths.push(format!(
            "{}\\scoop\\apps\\nodejs\\current\\node.exe",
            home_str
        ));
        paths.push(format!(
            "{}\\scoop\\apps\\nodejs-lts\\current\\node.exe",
            home_str
        ));
        paths.push("C:\\ProgramData\\chocolatey\\lib\\nodejs\\tools\\node.exe".to_string());
    }

    if let Ok(program_files) = std::env::var("ProgramFiles") {
        paths.push(format!("{}\\nodejs\\node.exe", program_files));
    }
    if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
        paths.push(format!("{}\\nodejs\\node.exe", program_files_x86));
    }

    if let Ok(nvm_symlink) = std::env::var("NVM_SYMLINK") {
        paths.insert(0, format!("{}\\node.exe", nvm_symlink));
    }

    if let Ok(nvm_home) = std::env::var("NVM_HOME") {
        let settings_path = format!("{}\\settings.txt", nvm_home);
        if let Ok(content) = std::fs::read_to_string(&settings_path) {
            for line in content.lines() {
                if let Some(version) = line.strip_prefix("current:") {
                    let version = version.trim();
                    if !version.is_empty() {
                        paths.insert(0, format!("{}\\v{}\\node.exe", nvm_home, version));
                    }
                }
            }
        }
    }

    paths
}

/// 获取 Unix 系统上可能的 openclaw 安装路径
fn get_unix_openclaw_paths() -> Vec<String> {
    let mut paths = Vec::new();
    paths.push("/usr/local/bin/openclaw".to_string());
    paths.push("/opt/homebrew/bin/openclaw".to_string());
    paths.push("/usr/bin/openclaw".to_string());

    if let Some(home) = dirs::home_dir() {
        let home_str = home.display().to_string();
        paths.push(format!("{}/.npm-global/bin/openclaw", home_str));

        for version in [
            "v22.0.0", "v22.1.0", "v22.2.0", "v22.11.0", "v22.12.0", "v23.0.0",
        ] {
            paths.push(format!(
                "{}/.nvm/versions/node/{}/bin/openclaw",
                home_str, version
            ));
        }

        let nvm_default = format!("{}/.nvm/alias/default", home_str);
        if let Ok(version) = std::fs::read_to_string(&nvm_default) {
            let version = version.trim();
            if !version.is_empty() {
                paths.insert(
                    0,
                    format!("{}/.nvm/versions/node/v{}/bin/openclaw", home_str, version),
                );
            }
        }

        paths.push(format!("{}/.fnm/aliases/default/bin/openclaw", home_str));
        paths.push(format!("{}/.volta/bin/openclaw", home_str));
        paths.push(format!("{}/.pnpm/bin/openclaw", home_str));
        paths.push(format!("{}/Library/pnpm/openclaw", home_str));
        paths.push(format!("{}/.asdf/shims/openclaw", home_str));
        paths.push(format!("{}/.local/share/mise/shims/openclaw", home_str));
        paths.push(format!("{}/.yarn/bin/openclaw", home_str));
        paths.push(format!(
            "{}/.config/yarn/global/node_modules/.bin/openclaw",
            home_str
        ));
    }

    paths
}

/// 获取 Windows 上可能的 openclaw 安装路径
fn get_windows_openclaw_paths() -> Vec<String> {
    let mut paths = Vec::new();
    paths.push("C:\\nvm4w\\nodejs\\openclaw.cmd".to_string());

    if let Some(home) = dirs::home_dir() {
        paths.push(format!(
            "{}\\AppData\\Roaming\\npm\\openclaw.cmd",
            home.display()
        ));
    }

    paths.push("C:\\Program Files\\nodejs\\openclaw.cmd".to_string());
    paths
}

/// 执行 openclaw 命令并获取输出
pub fn run_openclaw(args: &[&str]) -> Result<String, String> {
    debug!("[Shell] 执行 openclaw 命令: {:?}", args);

    let runtime = resolve_openclaw_runtime().ok_or_else(|| {
        warn!("[Shell] 找不到 openclaw 命令");
        "找不到 openclaw 命令，请先准备内置运行时或通过 npm install -g openclaw 安装".to_string()
    })?;

    let output = build_openclaw_command(&runtime, args).output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            debug!("[Shell] 运行时模式: {}", runtime.mode());
            debug!("[Shell] 命令退出码: {:?}", out.status.code());
            if out.status.success() {
                Ok(stdout)
            } else {
                Err(format!("{}\n{}", stdout, stderr).trim().to_string())
            }
        }
        Err(error) => Err(format!("执行 openclaw 失败: {}", error)),
    }
}

/// 从 ~/.openclaw/env 文件读取所有环境变量
/// 与 shell 脚本 `source ~/.openclaw/env` 行为一致
fn load_openclaw_env_vars() -> HashMap<String, String> {
    let mut env_vars = HashMap::new();
    let env_path = platform::get_env_file_path();

    if let Ok(content) = file::read_file(&env_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some((key, value)) = line.split_once('=') {
                env_vars.insert(
                    key.trim().to_string(),
                    value
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string(),
                );
            }
        }
    }

    env_vars
}

/// 后台启动 openclaw gateway
/// 与 shell 脚本行为一致：先加载 env 文件，再启动 gateway
pub fn spawn_openclaw_gateway() -> io::Result<()> {
    info!("[Shell] 后台启动 openclaw gateway...");

    let runtime = resolve_openclaw_runtime().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "找不到 openclaw 命令，请先准备内置运行时或通过 npm install -g openclaw 安装",
        )
    })?;

    let mut command = build_openclaw_command(&runtime, &["gateway", "--port", "18789"]);

    for (key, value) in load_openclaw_env_vars() {
        command.env(key, value);
    }

    if let OpenClawRuntime::Bundled(runtime) = &runtime {
        command.env("BOSICLAW_BUNDLED_RUNTIME_ROOT", &runtime.runtime_root);
        command.env("BOSICLAW_BUNDLED_NODE_ROOT", &runtime.node_root);
    }

    let logs_dir = format!("{}/logs", platform::get_config_dir());
    let _ = std::fs::create_dir_all(&logs_dir);

    let stdout_log_path = format!("{}/gateway.log", logs_dir);
    let stderr_log_path = format!("{}/gateway.err.log", logs_dir);

    if let Ok(stdout_file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stdout_log_path)
    {
        command.stdout(std::process::Stdio::from(stdout_file));
    }
    if let Ok(stderr_file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_log_path)
    {
        command.stderr(std::process::Stdio::from(stderr_file));
    }

    command
        .spawn()
        .map(|child| {
            info!("[Shell] ✓ Gateway 进程已启动, PID: {}", child.id());
        })
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("启动失败 (运行时: {}): {}", runtime.display_path(), error),
            )
        })
}

/// 检查命令是否存在
pub fn command_exists(cmd: &str) -> bool {
    if platform::is_windows() {
        let mut command = Command::new("where");
        command.arg(cmd);
        apply_common_env(&mut command);
        command
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    } else {
        let mut command = Command::new("which");
        command.arg(cmd);
        apply_common_env(&mut command);
        command
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}
