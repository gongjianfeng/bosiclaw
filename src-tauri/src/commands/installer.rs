use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use crate::utils::{platform, shell};
use serde::{Deserialize, Serialize};
use tauri::command;
use log::{info, warn, error, debug};

const DEFAULT_NPM_REGISTRY: &str = "https://registry.npmmirror.com";
const DEFAULT_NODE_DIST_MIRROR: &str = "https://mirrors.tuna.tsinghua.edu.cn/nodejs-release/";
const DEFAULT_NODE_DOWNLOAD_URL: &str = "https://mirrors.tuna.tsinghua.edu.cn/nodejs-release/";
const DEFAULT_HOMEBREW_BREW_GIT_REMOTE: &str = "https://mirrors.tuna.tsinghua.edu.cn/git/homebrew/brew.git";
const DEFAULT_HOMEBREW_CORE_GIT_REMOTE: &str = "https://mirrors.tuna.tsinghua.edu.cn/git/homebrew/homebrew-core.git";
const DEFAULT_HOMEBREW_INSTALL_GIT_REMOTE: &str = "https://mirrors.tuna.tsinghua.edu.cn/git/homebrew/install.git";
const DEFAULT_HOMEBREW_BOTTLE_DOMAIN: &str = "https://mirrors.tuna.tsinghua.edu.cn/homebrew-bottles";
const DEFAULT_HOMEBREW_API_DOMAIN: &str = "https://mirrors.tuna.tsinghua.edu.cn/homebrew-bottles/api";

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

fn unix_npm_mirror_exports() -> String {
    format!(
        r#"
OPENCLAW_NPM_REGISTRY="${{OPENCLAW_NPM_REGISTRY:-{registry}}}"
export NPM_CONFIG_REGISTRY="$OPENCLAW_NPM_REGISTRY"
export npm_config_registry="$OPENCLAW_NPM_REGISTRY"
"#,
        registry = DEFAULT_NPM_REGISTRY,
    )
}

fn windows_npm_mirror_exports() -> String {
    format!(
        r#"
if (-not $env:OPENCLAW_NPM_REGISTRY) {{ $env:OPENCLAW_NPM_REGISTRY = "{registry}" }}
$env:NPM_CONFIG_REGISTRY = $env:OPENCLAW_NPM_REGISTRY
$env:npm_config_registry = $env:OPENCLAW_NPM_REGISTRY
"#,
        registry = DEFAULT_NPM_REGISTRY,
    )
}

fn windows_node_mirror_exports() -> String {
    format!(
        r#"
if (-not $env:OPENCLAW_NODE_DIST_MIRROR) {{ $env:OPENCLAW_NODE_DIST_MIRROR = "{dist_mirror}" }}
if (-not $env:OPENCLAW_NODE_DOWNLOAD_URL) {{ $env:OPENCLAW_NODE_DOWNLOAD_URL = "{download_url}" }}
"#,
        dist_mirror = DEFAULT_NODE_DIST_MIRROR,
        download_url = DEFAULT_NODE_DOWNLOAD_URL,
    )
}

fn windows_command_bootstrap_script() -> String {
    r#"
function Add-PathEntryIfExists {
    param([string]$PathEntry)
    if ([string]::IsNullOrWhiteSpace($PathEntry)) { return }
    if (-not (Test-Path $PathEntry)) { return }

    $pathEntries = @($env:Path -split ';' | Where-Object { $_ })
    if ($pathEntries -notcontains $PathEntry) {
        $env:Path = "$PathEntry;$env:Path"
    }
}

function Resolve-FirstCommandPath {
    param(
        [Parameter(Mandatory=$true)][string]$CommandName,
        [string[]]$CandidatePaths = @()
    )

    $command = Get-Command $CommandName -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) {
        if ($command.Source) { return $command.Source }
        if ($command.Path) { return $command.Path }
        if ($command.Definition) { return $command.Definition }
    }

    foreach ($candidate in $CandidatePaths) {
        if (-not [string]::IsNullOrWhiteSpace($candidate) -and (Test-Path $candidate)) {
            return $candidate
        }
    }

    return $null
}

function Assert-LastExitCode {
    param([string]$Message)
    if ($LASTEXITCODE -ne 0) {
        throw "$Message (exit code: $LASTEXITCODE)"
    }
}

$pathEntries = @(
    "$env:APPDATA\npm",
    "$env:USERPROFILE\AppData\Roaming\npm",
    "$env:LOCALAPPDATA\fnm",
    "$env:LOCALAPPDATA\fnm_multishells",
    "$env:USERPROFILE\.fnm",
    "C:\nvm4w\nodejs",
    "$env:ProgramFiles\nodejs",
    "${env:ProgramFiles(x86)}\nodejs",
    "$env:ProgramFiles\Git\cmd",
    "$env:ProgramFiles\Git\bin",
    "${env:ProgramFiles(x86)}\Git\cmd",
    "${env:ProgramFiles(x86)}\Git\bin",
    "$env:USERPROFILE\scoop\shims"
)
foreach ($pathEntry in $pathEntries) {
    Add-PathEntryIfExists $pathEntry
}
"#
    .to_string()
}

fn windows_openclaw_resolver_script() -> String {
    r#"
$openclawCmd = Resolve-FirstCommandPath -CommandName 'openclaw' -CandidatePaths @(
    "$env:APPDATA\npm\openclaw.cmd",
    "$env:USERPROFILE\AppData\Roaming\npm\openclaw.cmd",
    "$env:ProgramFiles\nodejs\openclaw.cmd",
    "${env:ProgramFiles(x86)}\nodejs\openclaw.cmd",
    "C:\nvm4w\nodejs\openclaw.cmd"
)
if ($openclawCmd) {
    Add-PathEntryIfExists (Split-Path -Parent $openclawCmd)
}
"#
    .to_string()
}

fn windows_git_installer_script() -> String {
    r#"
$gitCandidatePaths = @(
    "$env:ProgramFiles\Git\cmd\git.exe",
    "$env:ProgramFiles\Git\bin\git.exe",
    "${env:ProgramFiles(x86)}\Git\cmd\git.exe",
    "${env:ProgramFiles(x86)}\Git\bin\git.exe",
    "$env:USERPROFILE\scoop\shims\git.exe"
)

$gitCmd = Resolve-FirstCommandPath -CommandName 'git' -CandidatePaths $gitCandidatePaths
if (-not $gitCmd) {
    Write-Host "未检测到 Git，正在尝试自动安装..." -ForegroundColor Yellow
    $gitInstalled = $false

    $winget = Get-Command winget -ErrorAction SilentlyContinue
    if ($winget) {
        Write-Host "尝试使用 winget 安装 Git for Windows..." -ForegroundColor Yellow
        & winget install --id Git.Git --accept-source-agreements --accept-package-agreements --disable-interactivity
        $gitInstalled = ($LASTEXITCODE -eq 0)
    }

    if (-not $gitInstalled) {
        $choco = Get-Command choco -ErrorAction SilentlyContinue
        if ($choco) {
            Write-Host "尝试使用 Chocolatey 安装 Git..." -ForegroundColor Yellow
            & choco install git -y
            $gitInstalled = ($LASTEXITCODE -eq 0)
        }
    }

    if (-not $gitInstalled) {
        $scoop = Get-Command scoop -ErrorAction SilentlyContinue
        if ($scoop) {
            Write-Host "尝试使用 Scoop 安装 Git..." -ForegroundColor Yellow
            & scoop install git
            $gitInstalled = ($LASTEXITCODE -eq 0)
        }
    }

    $gitCmd = Resolve-FirstCommandPath -CommandName 'git' -CandidatePaths $gitCandidatePaths
}

if ($gitCmd) {
    Add-PathEntryIfExists (Split-Path -Parent $gitCmd)
} else {
    throw "自动安装 Git 失败。请手动安装 Git for Windows: https://git-scm.com/download/win"
}
"#
    .to_string()
}

fn encode_powershell_script(script: &str) -> String {
    let utf16le_bytes: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    BASE64_STANDARD.encode(utf16le_bytes)
}

fn windows_powershell_start_command(encoded_command: &str, run_as_admin: bool) -> String {
    let verb = if run_as_admin { " -Verb RunAs" } else { "" };

    format!(
        "Start-Process -FilePath 'powershell.exe' -ArgumentList '-NoExit','-ExecutionPolicy','Bypass','-EncodedCommand','{encoded_command}'{verb}",
        encoded_command = encoded_command,
        verb = verb,
    )
}

fn open_windows_powershell_terminal(script_content: &str, run_as_admin: bool) -> Result<(), String> {
    let encoded_command = encode_powershell_script(script_content);
    let launch_script = windows_powershell_start_command(&encoded_command, run_as_admin);
    shell::run_powershell_output(&launch_script)
        .map(|_| ())
        .map_err(|e| format!("启动终端失败: {}", e))
}

fn unix_homebrew_mirror_exports() -> String {
    format!(
        r#"
OPENCLAW_HOMEBREW_BREW_GIT_REMOTE="${{OPENCLAW_HOMEBREW_BREW_GIT_REMOTE:-{brew_git}}}"
OPENCLAW_HOMEBREW_CORE_GIT_REMOTE="${{OPENCLAW_HOMEBREW_CORE_GIT_REMOTE:-{core_git}}}"
OPENCLAW_HOMEBREW_INSTALL_GIT_REMOTE="${{OPENCLAW_HOMEBREW_INSTALL_GIT_REMOTE:-{install_git}}}"
OPENCLAW_HOMEBREW_BOTTLE_DOMAIN="${{OPENCLAW_HOMEBREW_BOTTLE_DOMAIN:-{bottle_domain}}}"
OPENCLAW_HOMEBREW_API_DOMAIN="${{OPENCLAW_HOMEBREW_API_DOMAIN:-{api_domain}}}"
export HOMEBREW_BREW_GIT_REMOTE="$OPENCLAW_HOMEBREW_BREW_GIT_REMOTE"
export HOMEBREW_CORE_GIT_REMOTE="$OPENCLAW_HOMEBREW_CORE_GIT_REMOTE"
export HOMEBREW_BOTTLE_DOMAIN="$OPENCLAW_HOMEBREW_BOTTLE_DOMAIN"
export HOMEBREW_API_DOMAIN="$OPENCLAW_HOMEBREW_API_DOMAIN"
"#,
        brew_git = DEFAULT_HOMEBREW_BREW_GIT_REMOTE,
        core_git = DEFAULT_HOMEBREW_CORE_GIT_REMOTE,
        install_git = DEFAULT_HOMEBREW_INSTALL_GIT_REMOTE,
        bottle_domain = DEFAULT_HOMEBREW_BOTTLE_DOMAIN,
        api_domain = DEFAULT_HOMEBREW_API_DOMAIN,
    )
}

fn macos_node_install_script_body() -> String {
    format!(
        r#"
set -e
{homebrew_mirror_exports}

ensure_brew_env() {{
    if [[ -f /opt/homebrew/bin/brew ]]; then
        eval "$(/opt/homebrew/bin/brew shellenv)"
    elif [[ -f /usr/local/bin/brew ]]; then
        eval "$(/usr/local/bin/brew shellenv)"
    fi
}}

if ! command -v brew &> /dev/null; then
    echo "正在通过国内镜像安装 Homebrew..."
    install_dir="$(mktemp -d /tmp/openclaw-homebrew-install.XXXXXX)"
    cleanup() {{
        rm -rf "$install_dir"
    }}
    trap cleanup EXIT

    if git clone --depth=1 "$OPENCLAW_HOMEBREW_INSTALL_GIT_REMOTE" "$install_dir"; then
        NONINTERACTIVE=1 /bin/bash "$install_dir/install.sh"
    else
        echo "镜像安装脚本拉取失败，回退官方源..."
        rm -rf "$install_dir"
        install_dir="$(mktemp -d /tmp/openclaw-homebrew-install.XXXXXX)"
        git clone --depth=1 https://github.com/Homebrew/install.git "$install_dir"
        NONINTERACTIVE=1 /bin/bash "$install_dir/install.sh"
    fi
fi

ensure_brew_env

echo "正在安装 Node.js 22..."
if ! brew install node@22; then
    echo "镜像源安装失败，回退官方 Homebrew 源重试..."
    unset HOMEBREW_BREW_GIT_REMOTE HOMEBREW_CORE_GIT_REMOTE HOMEBREW_BOTTLE_DOMAIN HOMEBREW_API_DOMAIN
    brew install node@22
fi

brew link --overwrite node@22
node --version
"#,
        homebrew_mirror_exports = unix_homebrew_mirror_exports(),
    )
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
    info!("[环境检查] Node.js: installed={}, version={:?}, version_ok={}", 
        node_installed, node_version, node_version_ok);
    
    // 检查 OpenClaw
    info!("[环境检查] 检查 OpenClaw...");
    let openclaw_version = get_openclaw_version();
    let openclaw_installed = openclaw_version.is_some();
    info!("[环境检查] OpenClaw: installed={}, version={:?}", 
        openclaw_installed, openclaw_version);
    
    // 检查配置目录
    let config_dir = platform::get_config_dir();
    let config_dir_exists = std::path::Path::new(&config_dir).exists();
    info!("[环境检查] 配置目录: {}, exists={}", config_dir, config_dir_exists);
    
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
    
    // Homebrew (macOS)
    paths.push("/opt/homebrew/bin/node".to_string()); // Apple Silicon
    paths.push("/usr/local/bin/node".to_string());     // Intel Mac
    
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
                paths.insert(0, format!("{}/.nvm/versions/node/v{}/bin/node", home_str, version));
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
    
    // 1. 标准安装路径 (Program Files)
    paths.push("C:\\Program Files\\nodejs\\node.exe".to_string());
    paths.push("C:\\Program Files (x86)\\nodejs\\node.exe".to_string());
    
    // 2. nvm for Windows (nvm4w) - 常见安装位置
    paths.push("C:\\nvm4w\\nodejs\\node.exe".to_string());
    
    // 3. 用户目录下的各种安装
    if let Some(home) = dirs::home_dir() {
        let home_str = home.display().to_string();
        
        // nvm for Windows 用户安装
        paths.push(format!("{}\\AppData\\Roaming\\nvm\\current\\node.exe", home_str));
        
        // fnm (Fast Node Manager) for Windows
        paths.push(format!("{}\\AppData\\Roaming\\fnm\\aliases\\default\\node.exe", home_str));
        paths.push(format!("{}\\AppData\\Local\\fnm\\aliases\\default\\node.exe", home_str));
        paths.push(format!("{}\\.fnm\\aliases\\default\\node.exe", home_str));
        
        // volta
        paths.push(format!("{}\\AppData\\Local\\Volta\\bin\\node.exe", home_str));
        // volta 通过 shim 调用，检查 bin 目录即可
        
        // scoop 安装
        paths.push(format!("{}\\scoop\\apps\\nodejs\\current\\node.exe", home_str));
        paths.push(format!("{}\\scoop\\apps\\nodejs-lts\\current\\node.exe", home_str));
        
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
        let major = v.trim_start_matches('v')
            .split('.')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        major >= 22
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
        },
        "macos" => {
            info!("[安装Node.js] 使用 macOS 安装方式 (Homebrew)...");
            install_nodejs_macos().await
        },
        "linux" => {
            info!("[安装Node.js] 使用 Linux 安装方式...");
            install_nodejs_linux().await
        },
        _ => {
            error!("[安装Node.js] 不支持的操作系统: {}", os);
            Ok(InstallResult {
                success: false,
                message: "不支持的操作系统".to_string(),
                error: Some(format!("不支持的操作系统: {}", os)),
            })
        },
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
    // 使用 winget 安装 Node.js（Windows 10/11 自带）
    let script = [
        r#"
$ErrorActionPreference = 'Stop'
"#,
        &windows_node_mirror_exports(),
        r#"

# 检查是否已安装
$nodeVersion = node --version 2>$null
if ($nodeVersion) {
    Write-Host "Node.js 已安装: $nodeVersion"
    exit 0
}

# 优先使用 winget
$hasWinget = Get-Command winget -ErrorAction SilentlyContinue
if ($hasWinget) {
    Write-Host "使用 winget 安装 Node.js..."
    winget install --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements
    if ($LASTEXITCODE -eq 0) {
        Write-Host "Node.js 安装成功！"
        exit 0
    }
}

# 备用方案：使用 fnm (Fast Node Manager)
Write-Host "尝试使用 fnm 安装 Node.js..."
$fnmInstallScript = "irm https://fnm.vercel.app/install.ps1 | iex"
Invoke-Expression $fnmInstallScript

# 配置 fnm 环境
$env:FNM_DIR = "$env:USERPROFILE\.fnm"
$env:Path = "$env:FNM_DIR;$env:Path"
$env:FNM_NODE_DIST_MIRROR = $env:OPENCLAW_NODE_DIST_MIRROR

# 安装 Node.js 22
fnm install 22
fnm default 22
fnm use 22

# 验证安装
$nodeVersion = node --version 2>$null
if ($nodeVersion) {
    Write-Host "Node.js 安装成功: $nodeVersion"
    exit 0
} else {
    Write-Host "Node.js 安装失败"
    exit 1
}
"#,
    ]
    .concat();
    
    match shell::run_powershell_output(&script) {
        Ok(output) => {
            // 验证安装
            if get_node_version().is_some() {
                Ok(InstallResult {
                    success: true,
                    message: "Node.js 安装成功！请重启应用以使环境变量生效。".to_string(),
                    error: None,
                })
            } else {
                Ok(InstallResult {
                    success: false,
                    message: "安装后需要重启应用".to_string(),
                    error: Some(output),
                })
            }
        }
        Err(e) => Ok(InstallResult {
            success: false,
            message: "Node.js 安装失败".to_string(),
            error: Some(e),
        }),
    }
}

/// macOS 安装 Node.js
async fn install_nodejs_macos() -> Result<InstallResult, String> {
    let script = macos_node_install_script_body();
    
    match shell::run_bash_output(&script) {
        Ok(output) => Ok(InstallResult {
            success: true,
            message: format!("Node.js 安装成功！{}", output),
            error: None,
        }),
        Err(e) => Ok(InstallResult {
            success: false,
            message: "Node.js 安装失败".to_string(),
            error: Some(e),
        }),
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
        Ok(output) => Ok(InstallResult {
            success: true,
            message: format!("Node.js 安装成功！{}", output),
            error: None,
        }),
        Err(e) => Ok(InstallResult {
            success: false,
            message: "Node.js 安装失败".to_string(),
            error: Some(e),
        }),
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
        },
        _ => {
            info!("[安装OpenClaw] 使用 Unix 安装方式 (npm)...");
            install_openclaw_unix().await
        },
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
    let script = [
        r#"
$ErrorActionPreference = 'Stop'
"#,
        &windows_npm_mirror_exports(),
        &windows_command_bootstrap_script(),
        &windows_git_installer_script(),
        r#"

# 检查 Node.js
$nodeVersion = node --version 2>$null
if (-not $nodeVersion) {
    Write-Host "错误：请先安装 Node.js"
    exit 1
}

Write-Host "使用 npm 安装 OpenClaw..."
& npm install -g openclaw@latest "--registry=$env:OPENCLAW_NPM_REGISTRY"
Assert-LastExitCode "OpenClaw 安装失败"
"#,
        &windows_openclaw_resolver_script(),
        r#"

# 验证安装
$openclawVersion = if ($openclawCmd) { & $openclawCmd --version 2>$null } else { $null }
if ($openclawCmd) {
    Assert-LastExitCode "OpenClaw 版本检查失败"
}
if ($openclawVersion) {
    Write-Host "OpenClaw 安装成功: $openclawVersion"
    exit 0
} else {
    Write-Host "OpenClaw 安装完成后未找到可执行文件，请重启终端后重试"
    exit 1
}
"#,
    ]
    .concat();
    
    match shell::run_powershell_output(&script) {
        Ok(output) => {
            if get_openclaw_version().is_some() {
                Ok(InstallResult {
                    success: true,
                    message: "OpenClaw 安装成功！".to_string(),
                    error: None,
                })
            } else {
                Ok(InstallResult {
                    success: false,
                    message: "安装后需要重启应用".to_string(),
                    error: Some(output),
                })
            }
        }
        Err(e) => Ok(InstallResult {
            success: false,
            message: "OpenClaw 安装失败".to_string(),
            error: Some(e),
        }),
    }
}

/// Unix 系统安装 OpenClaw
async fn install_openclaw_unix() -> Result<InstallResult, String> {
    let script = format!(r#"
set -e
{npm_mirror_exports}

# 检查 Node.js
if ! command -v node &> /dev/null; then
    echo "错误：请先安装 Node.js"
    exit 1
fi

echo "使用 npm 安装 OpenClaw..."
npm install -g openclaw@latest --registry "$OPENCLAW_NPM_REGISTRY"

# 验证安装
openclaw --version
"#,
        npm_mirror_exports = unix_npm_mirror_exports(),
    );
    
    match shell::run_bash_output(&script) {
        Ok(output) => Ok(InstallResult {
            success: true,
            message: format!("OpenClaw 安装成功！{}", output),
            error: None,
        }),
        Err(e) => Ok(InstallResult {
            success: false,
            message: "OpenClaw 安装失败".to_string(),
            error: Some(e),
        }),
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
        },
        Err(e) => {
            error!("[初始化配置] ✗ 配置初始化失败: {}", e);
            Ok(InstallResult {
                success: false,
                message: "配置初始化失败".to_string(),
                error: Some(e),
            })
        },
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
        let script_content = [
            r#"
$ErrorActionPreference = 'Stop'
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "    Node.js 安装向导" -ForegroundColor White
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""
"#,
            &windows_node_mirror_exports(),
            &windows_command_bootstrap_script(),
            r#"

# 检查 winget
$hasWinget = Get-Command winget -ErrorAction SilentlyContinue
if ($hasWinget) {
    Write-Host "正在使用 winget 安装 Node.js 22..." -ForegroundColor Yellow
    & winget install --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements
    Assert-LastExitCode "Node.js 安装失败"
} else {
    Write-Host "请优先从以下国内镜像下载 Node.js:" -ForegroundColor Yellow
    Write-Host $env:OPENCLAW_NODE_DOWNLOAD_URL -ForegroundColor Green
    Write-Host ""
    Start-Process $env:OPENCLAW_NODE_DOWNLOAD_URL
}

Write-Host ""
Write-Host "安装完成后请重启 OpenClaw Manager" -ForegroundColor Green
Write-Host ""
        Read-Host "按回车键关闭此窗口"
"#,
        ]
        .concat();
        open_windows_powershell_terminal(&script_content, true)?;
        Ok("已打开安装终端".to_string())
    } else if platform::is_macos() {
        // macOS: 打开 Terminal.app
        let script_content = format!(r#"#!/bin/bash
clear
echo "========================================"
echo "    Node.js 安装向导"
echo "========================================"
echo ""
{script_body}
echo ""
read -p "按回车键关闭此窗口..."
"#,
            script_body = macos_node_install_script_body(),
        );
        
        let script_path = "/tmp/openclaw_install_nodejs.command";
        std::fs::write(script_path, script_content)
            .map_err(|e| format!("创建脚本失败: {}", e))?;
        
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
        Err(format!("请手动安装 Node.js，可优先使用国内镜像: {}", DEFAULT_NODE_DOWNLOAD_URL))
    }
}

/// 打开终端安装 OpenClaw
async fn open_openclaw_install_terminal() -> Result<String, String> {
    if platform::is_windows() {
        let script_content = [
            r#"
$ErrorActionPreference = 'Stop'
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "    OpenClaw 安装向导" -ForegroundColor White
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""
"#,
            &windows_npm_mirror_exports(),
            &windows_command_bootstrap_script(),
            &windows_git_installer_script(),
            r#"

Write-Host "正在安装 OpenClaw..." -ForegroundColor Yellow
& npm install -g openclaw@latest "--registry=$env:OPENCLAW_NPM_REGISTRY"
Assert-LastExitCode "OpenClaw 安装失败"
"#,
            &windows_openclaw_resolver_script(),
            r#"

if (-not $openclawCmd) {
    throw "OpenClaw 安装完成后未找到可执行文件。请重启终端后再试，或检查 %APPDATA%\npm 是否在 PATH 中。"
}

Write-Host ""
Write-Host "初始化配置..."
& $openclawCmd config set gateway.mode local
Assert-LastExitCode "OpenClaw 初始化配置失败"

Write-Host ""
Write-Host "安装完成！" -ForegroundColor Green
$openclawVersion = & $openclawCmd --version
Assert-LastExitCode "OpenClaw 版本检查失败"
Write-Host $openclawVersion
Write-Host ""
        Read-Host "按回车键关闭此窗口"
"#,
        ]
        .concat();
        open_windows_powershell_terminal(&script_content, true)?;
        Ok("已打开安装终端".to_string())
    } else if platform::is_macos() {
        let script_content = format!(r#"#!/bin/bash
clear
echo "========================================"
echo "    OpenClaw 安装向导"
echo "========================================"
echo ""
set -e
{npm_mirror_exports}

echo "正在安装 OpenClaw..."
npm install -g openclaw@latest --registry "$OPENCLAW_NPM_REGISTRY"

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
"#,
            npm_mirror_exports = unix_npm_mirror_exports(),
        );
        
        let script_path = "/tmp/openclaw_install_openclaw.command";
        std::fs::write(script_path, script_content)
            .map_err(|e| format!("创建脚本失败: {}", e))?;
        
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
        let script_content = format!(r#"#!/bin/bash
clear
echo "========================================"
echo "    OpenClaw 安装向导"
echo "========================================"
echo ""
set -e
{npm_mirror_exports}

echo "正在安装 OpenClaw..."
npm install -g openclaw@latest --registry "$OPENCLAW_NPM_REGISTRY"

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
"#,
            npm_mirror_exports = unix_npm_mirror_exports(),
        );
        
        let script_path = "/tmp/openclaw_install_openclaw.sh";
        std::fs::write(script_path, script_content)
            .map_err(|e| format!("创建脚本失败: {}", e))?;
        
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
        
        Err(format!(
            "无法启动终端，请手动运行: npm install -g openclaw@latest --registry {}",
            DEFAULT_NPM_REGISTRY
        ))
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
        },
        _ => {
            info!("[卸载OpenClaw] 使用 Unix 卸载方式 (npm)...");
            uninstall_openclaw_unix().await
        },
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
    // 使用 cmd.exe 执行 npm uninstall，避免 PowerShell 执行策略问题
    info!("[卸载OpenClaw] 执行 npm uninstall -g openclaw...");
    
    match shell::run_cmd_output("npm uninstall -g openclaw") {
        Ok(output) => {
            info!("[卸载OpenClaw] npm 输出: {}", output);
            
            // 验证卸载是否成功
            std::thread::sleep(std::time::Duration::from_millis(500));
            if get_openclaw_version().is_none() {
                Ok(InstallResult {
                    success: true,
                    message: "OpenClaw 已成功卸载！".to_string(),
                    error: None,
                })
            } else {
                Ok(InstallResult {
                    success: false,
                    message: "卸载命令已执行，但 OpenClaw 仍然存在，请尝试手动卸载".to_string(),
                    error: Some(output),
                })
            }
        }
        Err(e) => {
            warn!("[卸载OpenClaw] npm uninstall 失败: {}", e);
            Ok(InstallResult {
                success: false,
                message: "OpenClaw 卸载失败".to_string(),
                error: Some(e),
            })
        }
    }
}

/// Unix 系统卸载 OpenClaw
async fn uninstall_openclaw_unix() -> Result<InstallResult, String> {
    let script = r#"
echo "卸载 OpenClaw..."
npm uninstall -g openclaw

# 验证卸载
if command -v openclaw &> /dev/null; then
    echo "警告：openclaw 命令仍然存在"
    exit 1
else
    echo "OpenClaw 已成功卸载"
    exit 0
fi
"#;
    
    match shell::run_bash_output(&script) {
        Ok(output) => Ok(InstallResult {
            success: true,
            message: format!("OpenClaw 已成功卸载！{}", output),
            error: None,
        }),
        Err(e) => Ok(InstallResult {
            success: false,
            message: "OpenClaw 卸载失败".to_string(),
            error: Some(e),
        }),
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
    // 使用 npm view 获取最新版本
    let result = if platform::is_windows() {
        shell::run_cmd_output(&format!(
            "set NPM_CONFIG_REGISTRY={} && set npm_config_registry={} && npm view openclaw version --registry {}",
            DEFAULT_NPM_REGISTRY,
            DEFAULT_NPM_REGISTRY,
            DEFAULT_NPM_REGISTRY
        ))
    } else {
        shell::run_bash_output(&format!(
            r#"
set -e
{npm_mirror_exports}
npm view openclaw version --registry "$OPENCLAW_NPM_REGISTRY" 2>/dev/null
"#,
            npm_mirror_exports = unix_npm_mirror_exports(),
        ))
    };
    
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
    let current_parts: Vec<u32> = current
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    let latest_parts: Vec<u32> = latest
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    
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
        },
        _ => {
            info!("[更新OpenClaw] 使用 Unix 更新方式 (npm)...");
            update_openclaw_unix().await
        },
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
    info!("[更新OpenClaw] 执行 npm install -g openclaw@latest...");
    
    match shell::run_cmd_output(&format!(
        "set NPM_CONFIG_REGISTRY={registry} && set npm_config_registry={registry} && npm install -g openclaw@latest --registry {registry}",
        registry = DEFAULT_NPM_REGISTRY
    )) {
        Ok(output) => {
            info!("[更新OpenClaw] npm 输出: {}", output);
            
            // 获取新版本
            let new_version = get_openclaw_version();
            
            Ok(InstallResult {
                success: true,
                message: format!("OpenClaw 已更新到 {}", new_version.unwrap_or("最新版本".to_string())),
                error: None,
            })
        }
        Err(e) => {
            warn!("[更新OpenClaw] npm install 失败: {}", e);
            Ok(InstallResult {
                success: false,
                message: "OpenClaw 更新失败".to_string(),
                error: Some(e),
            })
        }
    }
}

/// Unix 系统更新 OpenClaw
async fn update_openclaw_unix() -> Result<InstallResult, String> {
    let script = format!(r#"
set -e
{npm_mirror_exports}

echo "更新 OpenClaw..."
npm install -g openclaw@latest --registry "$OPENCLAW_NPM_REGISTRY"

# 验证更新
openclaw --version
"#,
        npm_mirror_exports = unix_npm_mirror_exports(),
    );
    
    match shell::run_bash_output(&script) {
        Ok(output) => Ok(InstallResult {
            success: true,
            message: format!("OpenClaw 已更新！{}", output),
            error: None,
        }),
        Err(e) => Ok(InstallResult {
            success: false,
            message: "OpenClaw 更新失败".to_string(),
            error: Some(e),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_versions_handles_v_prefix_and_patch_updates() {
        assert!(compare_versions("v1.2.3", "1.2.4"));
        assert!(compare_versions("1.2.3", "1.3.0"));
        assert!(!compare_versions("1.2.3", "1.2.3"));
        assert!(!compare_versions("1.3.0", "1.2.9"));
    }

    #[test]
    fn unix_npm_mirror_exports_include_expected_defaults() {
        let exports = unix_npm_mirror_exports();
        assert!(exports.contains(DEFAULT_NPM_REGISTRY));
        assert!(exports.contains("npm_config_registry"));
        assert!(!exports.contains("npm_config_disturl"));
    }

    #[test]
    fn macos_node_install_script_contains_cn_mirror_fallbacks() {
        let script = macos_node_install_script_body();
        assert!(script.contains(DEFAULT_HOMEBREW_INSTALL_GIT_REMOTE));
        assert!(script.contains(DEFAULT_HOMEBREW_BOTTLE_DOMAIN));
        assert!(script.contains("brew install node@22"));
        assert!(script.contains("回退官方 Homebrew 源重试"));
    }

    #[test]
    fn encode_powershell_script_uses_utf16le_base64() {
        let script = "Write-Host \"你好\"";
        let encoded = encode_powershell_script(script);
        let decoded = BASE64_STANDARD.decode(encoded).unwrap();
        let utf16: Vec<u16> = decoded
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        assert_eq!(String::from_utf16(&utf16).unwrap(), script);
    }

    #[test]
    fn windows_start_command_uses_encoded_command_and_runas() {
        let command = windows_powershell_start_command("QUJDRA==", true);
        assert!(command.contains("Start-Process -FilePath 'powershell.exe'"));
        assert!(command.contains("'-EncodedCommand','QUJDRA=='"));
        assert!(command.contains("-Verb RunAs"));
    }

    #[test]
    fn windows_start_command_without_runas_keeps_encoded_payload() {
        let command = windows_powershell_start_command("LQBuAG8AZABlAA==", false);
        assert!(command.contains("'-EncodedCommand','LQBuAG8AZABlAA=='"));
        assert!(!command.contains("-Verb RunAs"));
    }

    #[test]
    fn windows_command_bootstrap_includes_common_cli_paths() {
        let script = windows_command_bootstrap_script();
        assert!(script.contains("$env:APPDATA\\npm"));
        assert!(script.contains("$env:ProgramFiles\\Git\\cmd"));
        assert!(script.contains("function Assert-LastExitCode"));
    }

    #[test]
    fn windows_openclaw_resolver_checks_common_global_install_locations() {
        let script = windows_openclaw_resolver_script();
        assert!(script.contains("$env:APPDATA\\npm\\openclaw.cmd"));
        assert!(script.contains("C:\\nvm4w\\nodejs\\openclaw.cmd"));
    }

    #[test]
    fn windows_git_installer_script_tries_known_package_managers() {
        let script = windows_git_installer_script();
        assert!(script.contains("winget install --id Git.Git"));
        assert!(script.contains("& choco install git -y"));
        assert!(script.contains("& scoop install git"));
        assert!(script.contains("https://git-scm.com/download/win"));
    }
}
