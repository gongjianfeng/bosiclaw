use std::path::{Path, PathBuf};
use std::fs;
use log::{info, debug};
use crate::utils::platform;

/// 获取 runtime 目录路径: ~/.openclaw/runtime/
fn get_runtime_dir() -> PathBuf {
    let config_dir = platform::get_config_dir();
    PathBuf::from(config_dir).join("runtime")
}

/// 获取 runtime 中 openclaw 的安装目录: ~/.openclaw/runtime/openclaw/
fn get_runtime_openclaw_dir() -> PathBuf {
    get_runtime_dir().join("openclaw")
}

/// 获取版本标记文件路径: ~/.openclaw/runtime/.version
fn get_version_file() -> PathBuf {
    get_runtime_dir().join(".version")
}

/// 获取 bundled Node.js sidecar 的路径
/// Tauri sidecar 在打包后位于可执行文件同目录（macOS 为 .app/Contents/MacOS/）
pub fn get_bundled_node_path() -> Option<PathBuf> {
    let exe_path = std::env::current_exe().ok()?;
    let exe_dir = exe_path.parent()?;

    let node_name = if platform::is_windows() {
        "node.exe"
    } else {
        "node"
    };

    let node_path = exe_dir.join(node_name);
    if node_path.exists() {
        info!("[Bundled] 找到内置 Node.js: {}", node_path.display());
        Some(node_path)
    } else {
        debug!("[Bundled] 未找到内置 Node.js: {}", node_path.display());
        None
    }
}

/// 获取 bundled Node.js 路径（字符串形式，供 shell.rs 使用）
pub fn get_bundled_node_path_str() -> Option<String> {
    get_bundled_node_path().map(|p| p.to_string_lossy().to_string())
}

/// 获取 runtime 中 openclaw 入口脚本路径
/// 返回 ~/.openclaw/runtime/openclaw/openclaw.mjs（若存在）
pub fn get_runtime_openclaw_path() -> Option<String> {
    let mjs_path = get_runtime_openclaw_dir().join("openclaw.mjs");
    if mjs_path.exists() {
        let path_str = mjs_path.to_string_lossy().to_string();
        info!("[Bundled] 找到 runtime openclaw: {}", path_str);
        Some(path_str)
    } else {
        debug!("[Bundled] 未找到 runtime openclaw: {}", mjs_path.display());
        None
    }
}

/// 确保 openclaw runtime 已解压
/// 从 resource_dir 中的 openclaw-runtime.tgz 解压到 ~/.openclaw/runtime/openclaw/
pub fn ensure_openclaw_extracted(resource_dir: &Path) -> Result<PathBuf, String> {
    let runtime_dir = get_runtime_dir();
    let openclaw_dir = get_runtime_openclaw_dir();
    let version_file = get_version_file();
    let mjs_path = openclaw_dir.join("openclaw.mjs");

    let tgz_path = resource_dir.join("openclaw-runtime.tgz");
    if !tgz_path.exists() {
        debug!("[Bundled] 未找到 bundled openclaw-runtime.tgz: {}", tgz_path.display());
        return Err("未找到内置 openclaw-runtime.tgz".to_string());
    }

    // 读取 tgz 中的版本号
    let bundled_version = read_version_from_tgz(&tgz_path)
        .unwrap_or_else(|_| "unknown".to_string());
    info!("[Bundled] 内置 openclaw 版本: {}", bundled_version);

    // 检查是否已解压且版本匹配
    if mjs_path.exists() && version_file.exists() {
        if let Ok(installed_version) = fs::read_to_string(&version_file) {
            let installed_version = installed_version.trim();
            if installed_version == bundled_version {
                info!("[Bundled] runtime 已就绪，版本: {}", installed_version);
                return Ok(mjs_path);
            }
            info!("[Bundled] 版本不匹配: installed={}, bundled={}",
                installed_version, bundled_version);
        }
    }

    // 需要解压
    info!("[Bundled] 开始解压 openclaw runtime 到 {}", openclaw_dir.display());

    // 确保 runtime 根目录存在
    fs::create_dir_all(&runtime_dir)
        .map_err(|e| format!("创建 runtime 目录失败: {}", e))?;

    // 清理旧的 openclaw 目录
    if openclaw_dir.exists() {
        info!("[Bundled] 清理旧 runtime 目录...");
        fs::remove_dir_all(&openclaw_dir)
            .map_err(|e| format!("清理旧 runtime 失败: {}", e))?;
    }

    fs::create_dir_all(&openclaw_dir)
        .map_err(|e| format!("创建 openclaw 目录失败: {}", e))?;

    // 解压 tgz
    extract_tgz(&tgz_path, &openclaw_dir)?;

    // 写入版本标记
    fs::write(&version_file, &bundled_version)
        .map_err(|e| format!("写入版本标记失败: {}", e))?;

    if mjs_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&mjs_path, fs::Permissions::from_mode(0o755));
        }
        info!("[Bundled] ✓ openclaw runtime 解压完成");
        Ok(mjs_path)
    } else {
        Err(format!("解压完成但未找到 openclaw.mjs: {}", mjs_path.display()))
    }
}

/// 从 tgz 中读取 package.json 的 version 字段
fn read_version_from_tgz(tgz_path: &Path) -> Result<String, String> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let file = fs::File::open(tgz_path)
        .map_err(|e| format!("打开 tgz 失败: {}", e))?;
    let gz = GzDecoder::new(file);
    let mut archive = Archive::new(gz);

    for entry in archive.entries().map_err(|e| format!("读取 tgz entries 失败: {}", e))? {
        let entry = entry.map_err(|e| format!("读取 entry 失败: {}", e))?;
        let path = entry.path().map_err(|e| format!("读取 path 失败: {}", e))?;
        let path_str = path.to_string_lossy();

        if path_str == "package.json" || path_str == "./package.json" {
            let reader = std::io::BufReader::new(entry);
            let pkg: serde_json::Value = serde_json::from_reader(reader)
                .map_err(|e| format!("解析 package.json 失败: {}", e))?;
            if let Some(version) = pkg.get("version").and_then(|v| v.as_str()) {
                return Ok(version.to_string());
            }
            return Err("package.json 中未找到 version 字段".to_string());
        }
    }

    Err("tgz 中未找到 package.json".to_string())
}

/// 解压 tgz 文件到目标目录
fn extract_tgz(tgz_path: &Path, dest: &Path) -> Result<(), String> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let file = fs::File::open(tgz_path)
        .map_err(|e| format!("打开 tgz 失败: {}", e))?;
    let gz = GzDecoder::new(file);
    let mut archive = Archive::new(gz);

    archive.set_preserve_permissions(true);
    archive.set_overwrite(true);

    for entry in archive.entries().map_err(|e| format!("读取 tgz entries 失败: {}", e))? {
        let mut entry = entry.map_err(|e| format!("读取 entry 失败: {}", e))?;
        let path = entry.path().map_err(|e| format!("读取 path 失败: {}", e))?;
        let path = path.to_path_buf();

        // 去除 ./ 前缀
        let stripped = if let Ok(s) = path.strip_prefix("./") {
            s.to_path_buf()
        } else {
            path
        };

        // 跳过空路径和根目录
        if stripped.as_os_str().is_empty() || stripped == Path::new(".") {
            continue;
        }

        let dest_path = dest.join(&stripped);

        if let Some(parent) = dest_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录失败 {}: {}", parent.display(), e))?;
            }
        }

        entry.unpack(&dest_path)
            .map_err(|e| format!("解压文件失败 {}: {}", stripped.display(), e))?;
    }

    info!("[Bundled] tgz 解压完成: {}", dest.display());
    Ok(())
}

/// 检查 bundled runtime 是否可用
pub fn is_runtime_ready() -> bool {
    let mjs_path = get_runtime_openclaw_dir().join("openclaw.mjs");
    let version_file = get_version_file();
    mjs_path.exists() && version_file.exists()
}
