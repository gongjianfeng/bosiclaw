use crate::commands::config;
use crate::utils::{platform, shell};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use tauri::command;

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
    /// 运行时模式：bundled / system / missing
    pub runtime_mode: String,
    /// 内置运行时根目录
    pub runtime_root: Option<String>,
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

/// 检查环境状态
#[command]
pub async fn check_environment() -> Result<EnvironmentStatus, String> {
    info!("[环境检查] 开始检查系统环境...");

    let os = platform::get_os();
    let runtime_mode = shell::get_runtime_mode();
    let runtime_root = shell::get_runtime_root();
    info!("[环境检查] 操作系统: {}", os);
    info!(
        "[环境检查] 运行时模式: {}, root={:?}",
        runtime_mode, runtime_root
    );

    // 检查 Node.js
    info!("[环境检查] 检查 Node.js...");
    let node_version = shell::get_node_version();
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
        runtime_mode,
        runtime_root,
    })
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
        let major = v
            .trim_start_matches('v')
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

    if shell::is_bundled_runtime_active() {
        return Ok(InstallResult {
            success: true,
            message: "当前版本已内置 Node.js，无需额外安装".to_string(),
            error: None,
        });
    }

    let os = platform::get_os();
    info!("[安装Node.js] 检测到操作系统: {}", os);

    let result = match os.as_str() {
        "windows" => {
            info!("[安装Node.js] 使用 Windows 安装方式...");
            install_nodejs_windows().await
        }
        "macos" => {
            info!("[安装Node.js] 使用 macOS 安装方式 (Homebrew)...");
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
    // 使用 winget 安装 Node.js（Windows 10/11 自带）
    let script = r#"
$ErrorActionPreference = 'Stop'

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
"#;

    match shell::run_powershell_output(script) {
        Ok(output) => {
            // 验证安装
            if shell::get_node_version().is_some() {
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
    // 使用 Homebrew 安装
    let script = r#"
# 检查 Homebrew
if ! command -v brew &> /dev/null; then
    echo "安装 Homebrew..."
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    
    # 配置 PATH
    if [[ -f /opt/homebrew/bin/brew ]]; then
        eval "$(/opt/homebrew/bin/brew shellenv)"
    elif [[ -f /usr/local/bin/brew ]]; then
        eval "$(/usr/local/bin/brew shellenv)"
    fi
fi

echo "安装 Node.js 22..."
brew install node@22
brew link --overwrite node@22

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

    if shell::is_bundled_runtime_active() {
        return Ok(InstallResult {
            success: true,
            message: "当前版本已内置 OpenClaw，无需额外安装".to_string(),
            error: None,
        });
    }

    let os = platform::get_os();
    info!("[安装OpenClaw] 检测到操作系统: {}", os);

    let result = match os.as_str() {
        "windows" => {
            info!("[安装OpenClaw] 使用 Windows 安装方式...");
            install_openclaw_windows().await
        }
        _ => {
            info!("[安装OpenClaw] 使用 Unix 安装方式 (npm)...");
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
    let script = r#"
$ErrorActionPreference = 'Stop'

# 检查 Node.js
$nodeVersion = node --version 2>$null
if (-not $nodeVersion) {
    Write-Host "错误：请先安装 Node.js"
    exit 1
}

Write-Host "使用 npm 安装 OpenClaw..."
npm install -g openclaw@latest --unsafe-perm

# 验证安装
$openclawVersion = openclaw --version 2>$null
if ($openclawVersion) {
    Write-Host "OpenClaw 安装成功: $openclawVersion"
    exit 0
} else {
    Write-Host "OpenClaw 安装失败"
    exit 1
}
"#;

    match shell::run_powershell_output(script) {
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
    let script = r#"
# 检查 Node.js
if ! command -v node &> /dev/null; then
    echo "错误：请先安装 Node.js"
    exit 1
fi

echo "使用 npm 安装 OpenClaw..."
npm install -g openclaw@latest --unsafe-perm

# 验证安装
openclaw --version
"#;

    match shell::run_bash_output(script) {
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
    match config::ensure_local_gateway_config() {
        Ok(token) => {
            info!("[初始化配置] ✓ 配置初始化成功，token 前缀: {}...", &token[..8.min(token.len())]);
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
    if shell::is_bundled_runtime_active() {
        return Err("当前版本使用内置运行时，无需打开安装终端".to_string());
    }

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

# 检查 winget
$hasWinget = Get-Command winget -ErrorAction SilentlyContinue
if ($hasWinget) {
    Write-Host "正在使用 winget 安装 Node.js 22..." -ForegroundColor Yellow
    winget install --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements
} else {
    Write-Host "请从以下地址下载安装 Node.js:" -ForegroundColor Yellow
    Write-Host "https://nodejs.org/en/download" -ForegroundColor Green
    Write-Host ""
    Start-Process "https://nodejs.org/en/download"
}

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

# 检查 Homebrew
if ! command -v brew &> /dev/null; then
    echo "正在安装 Homebrew..."
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    
    if [[ -f /opt/homebrew/bin/brew ]]; then
        eval "$(/opt/homebrew/bin/brew shellenv)"
    elif [[ -f /usr/local/bin/brew ]]; then
        eval "$(/usr/local/bin/brew shellenv)"
    fi
fi

echo "正在安装 Node.js 22..."
brew install node@22
brew link --overwrite node@22

echo ""
echo "安装完成！"
node --version
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

Write-Host "正在安装 OpenClaw..." -ForegroundColor Yellow
npm install -g openclaw@latest

Write-Host ""
Write-Host "初始化配置..."
openclaw config set gateway.mode local

Write-Host ""
Write-Host "安装完成！" -ForegroundColor Green
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

echo "正在安装 OpenClaw..."
npm install -g openclaw@latest

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

echo "正在安装 OpenClaw..."
npm install -g openclaw@latest

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

        Err("无法启动终端，请手动运行: npm install -g openclaw".to_string())
    }
}

/// 卸载 OpenClaw
#[command]
pub async fn uninstall_openclaw() -> Result<InstallResult, String> {
    info!("[卸载OpenClaw] 开始卸载 OpenClaw...");

    if shell::is_bundled_runtime_active() {
        return Ok(InstallResult {
            success: false,
            message: "当前版本使用内置 OpenClaw，不支持应用内卸载".to_string(),
            error: Some("请通过重新打包或发布新版本应用来移除内置 OpenClaw".to_string()),
        });
    }

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

    match shell::run_bash_output(script) {
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

    if shell::is_bundled_runtime_active() {
        let current_version = get_openclaw_version();
        return Ok(UpdateInfo {
            update_available: false,
            current_version,
            latest_version: None,
            error: Some("当前版本使用内置 OpenClaw，请通过重新打包应用更新".to_string()),
        });
    }

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
        shell::run_cmd_output("npm view openclaw version")
    } else {
        shell::run_bash_output("npm view openclaw version 2>/dev/null")
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

    if shell::is_bundled_runtime_active() {
        return Ok(InstallResult {
            success: false,
            message: "当前版本使用内置 OpenClaw，不支持应用内更新".to_string(),
            error: Some("请重新构建并发布包含新 OpenClaw 版本的应用".to_string()),
        });
    }

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
            info!("[更新OpenClaw] 使用 Unix 更新方式 (npm)...");
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
    info!("[更新OpenClaw] 执行 npm install -g openclaw@latest...");

    match shell::run_cmd_output("npm install -g openclaw@latest") {
        Ok(output) => {
            info!("[更新OpenClaw] npm 输出: {}", output);

            // 获取新版本
            let new_version = get_openclaw_version();

            Ok(InstallResult {
                success: true,
                message: format!(
                    "OpenClaw 已更新到 {}",
                    new_version.unwrap_or("最新版本".to_string())
                ),
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
    let script = r#"
echo "更新 OpenClaw..."
npm install -g openclaw@latest

# 验证更新
openclaw --version
"#;

    match shell::run_bash_output(script) {
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
