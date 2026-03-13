#!/usr/bin/env node

import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '..');
const targetRoot = join(repoRoot, 'src-tauri', 'target');
const bundledRuntimeConfigPath = join(repoRoot, 'bundled-runtime.config.json');
const nodeVersionFilePath = join(repoRoot, '.nvmrc');

function parseArgs(argv) {
  const options = {};

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (!arg.startsWith('--')) {
      continue;
    }

    const [key, inlineValue] = arg.slice(2).split('=');
    if (inlineValue !== undefined) {
      options[key] = inlineValue;
      continue;
    }

    const next = argv[index + 1];
    if (next && !next.startsWith('--')) {
      options[key] = next;
      index += 1;
    } else {
      options[key] = 'true';
    }
  }

  return options;
}

function normalizePlatform(value) {
  const platform = (value ?? process.platform).toLowerCase();
  if (platform === 'win32' || platform === 'windows') {
    return 'windows';
  }
  if (platform === 'darwin' || platform === 'macos' || platform === 'mac') {
    return 'macos';
  }
  if (platform === 'linux') {
    return 'linux';
  }
  return platform;
}

function firstExisting(paths) {
  return paths.find((path) => existsSync(path));
}

function assertExists(path, message) {
  if (!existsSync(path)) {
    throw new Error(`${message}: ${path}`);
  }
}

function isAbsoluteManifestPath(value) {
  return typeof value === 'string' && (value.startsWith('/') || /^[A-Za-z]:[\\/]/.test(value));
}

function assertManifestDoesNotLeakBuildPaths(manifest, manifestPath) {
  const legacyKeys = ['nodeBin', 'nodeRoot', 'openclawPackageRoot'];
  const leakedKey = legacyKeys.find((key) => key in manifest);
  if (leakedKey) {
    throw new Error(`runtime manifest 仍包含构建机绝对路径字段 ${leakedKey}: ${manifestPath}`);
  }

  if (!manifest.paths || typeof manifest.paths !== 'object') {
    return;
  }

  for (const [key, value] of Object.entries(manifest.paths)) {
    if (isAbsoluteManifestPath(value)) {
      throw new Error(`runtime manifest paths.${key} 不能是绝对路径: ${manifestPath}`);
    }
  }
}

function readManifest(runtimeDir) {
  const manifestPath = join(runtimeDir, 'manifest.json');
  assertExists(manifestPath, '缺少 runtime manifest');

  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  if (!manifest.nodeVersion || !manifest.openclawVersion) {
    throw new Error(`runtime manifest 缺少版本字段: ${manifestPath}`);
  }
  assertManifestDoesNotLeakBuildPaths(manifest, manifestPath);

  return { manifest, manifestPath };
}

function readJsonIfExists(path) {
  if (!existsSync(path)) {
    return null;
  }

  return JSON.parse(readFileSync(path, 'utf8'));
}

function readTextIfExists(path) {
  if (!existsSync(path)) {
    return '';
  }

  return readFileSync(path, 'utf8').trim();
}

function normalizeNodeVersion(value) {
  const trimmed = String(value ?? '').trim();
  if (!trimmed) {
    return '';
  }

  return trimmed.startsWith('v') ? trimmed : `v${trimmed}`;
}

function assertPinnedVersions(manifest, manifestPath) {
  const bundledRuntimeConfig = readJsonIfExists(bundledRuntimeConfigPath) ?? {};
  const pinnedNodeVersion = normalizeNodeVersion(readTextIfExists(nodeVersionFilePath));
  const pinnedOpenclawVersion = bundledRuntimeConfig.openclawVersion ?? '';

  if (pinnedNodeVersion && manifest.nodeVersion !== pinnedNodeVersion) {
    throw new Error(
      `runtime manifest Node 版本不匹配，期望 ${pinnedNodeVersion}，实际 ${manifest.nodeVersion}: ${manifestPath}`
    );
  }

  if (pinnedOpenclawVersion && manifest.openclawVersion !== pinnedOpenclawVersion) {
    throw new Error(
      `runtime manifest OpenClaw 版本不匹配，期望 ${pinnedOpenclawVersion}，实际 ${manifest.openclawVersion}: ${manifestPath}`
    );
  }
}

function resolveNodeExecutable(runtimeDir, platform) {
  if (platform === 'windows') {
    return firstExisting([
      join(runtimeDir, 'tools', 'node', 'node.exe'),
      join(runtimeDir, 'tools', 'node', 'bin', 'node.exe'),
    ]);
  }

  return firstExisting([
    join(runtimeDir, 'tools', 'node', 'bin', 'node'),
    join(runtimeDir, 'tools', 'node', 'node'),
  ]);
}

function resolveOpenClawEntry(runtimeDir) {
  return firstExisting([
    join(runtimeDir, 'lib', 'node_modules', 'openclaw', 'openclaw.mjs'),
    join(runtimeDir, 'lib', 'node_modules', 'openclaw', 'dist', 'entry.js'),
  ]);
}

function resolveOpenClawPackageRoot(openclawEntry) {
  if (basename(openclawEntry) === 'openclaw.mjs') {
    return dirname(openclawEntry);
  }

  const distDir = dirname(openclawEntry);
  if (basename(distDir) === 'dist') {
    return dirname(distDir);
  }

  return dirname(openclawEntry);
}

function getReleaseDirCandidates(target) {
  const candidates = [];
  if (target) {
    candidates.push(join(targetRoot, target, 'release'));
  }
  candidates.push(join(targetRoot, 'release'));
  return candidates;
}

function getRuntimeDir(target) {
  const releaseDir = firstExisting(
    getReleaseDirCandidates(target).map((candidate) => join(candidate, 'runtime'))
  );

  if (!releaseDir) {
    throw new Error(`找不到构建产物 runtime 目录，target=${target ?? 'default'}`);
  }

  return releaseDir;
}

function getMacAppRuntimeDir(target) {
  const bundleDir = firstExisting(
    getReleaseDirCandidates(target).map((candidate) => join(candidate, 'bundle', 'macos'))
  );

  if (!bundleDir) {
    throw new Error('找不到 macOS bundle 目录');
  }

  const appDir = readdirSync(bundleDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && entry.name.endsWith('.app'))
    .map((entry) => join(bundleDir, entry.name))
    .sort((left, right) => statSync(right).mtimeMs - statSync(left).mtimeMs)[0];

  if (!appDir) {
    throw new Error(`找不到 .app 产物: ${bundleDir}`);
  }

  return join(appDir, 'Contents', 'Resources', 'runtime');
}

function verifyRuntimeLayout(runtimeDir, platform, label) {
  const { manifest, manifestPath } = readManifest(runtimeDir);
  assertPinnedVersions(manifest, manifestPath);
  const nodeExecutable = resolveNodeExecutable(runtimeDir, platform);
  const openclawEntry = resolveOpenClawEntry(runtimeDir);

  assertExists(runtimeDir, `${label} runtime 目录不存在`);
  if (!nodeExecutable) {
    throw new Error(`${label} 缺少 bundled Node 可执行文件: ${runtimeDir}`);
  }
  if (!openclawEntry) {
    throw new Error(`${label} 缺少 OpenClaw 入口文件: ${runtimeDir}`);
  }

  console.log(`[verify] ${label}`);
  console.log(`  runtime: ${runtimeDir}`);
  console.log(`  manifest: ${manifestPath}`);
  console.log(`  node: ${nodeExecutable}`);
  console.log(`  openclaw: ${openclawEntry}`);
  console.log(`  versions: node=${manifest.nodeVersion}, openclaw=${manifest.openclawVersion}`);

  return { manifest, nodeExecutable, openclawEntry };
}

function smokeTestRuntime(runtimeDir, platform, label, runtimeInfo) {
  const { manifest, nodeExecutable, openclawEntry } = runtimeInfo;
  const packageRoot = resolveOpenClawPackageRoot(openclawEntry);
  const env = { ...process.env };

  delete env.OPENCLAW_HOME;
  delete env.OPENCLAW_STATE_DIR;
  delete env.OPENCLAW_CONFIG_PATH;

  const result = spawnSync(nodeExecutable, [openclawEntry, '--version'], {
    cwd: packageRoot,
    env,
    encoding: 'utf8',
    stdio: 'pipe',
  });

  if (result.status !== 0) {
    throw new Error(
      [
        `${label} smoke test 失败`,
        `runtime: ${runtimeDir}`,
        `command: ${nodeExecutable} ${openclawEntry} --version`,
        result.stdout?.trim(),
        result.stderr?.trim(),
      ]
        .filter(Boolean)
        .join('\n')
    );
  }

  const stdout = result.stdout?.trim() ?? '';
  if (!stdout.includes(manifest.openclawVersion)) {
    throw new Error(
      `${label} smoke test 输出未包含期望的 OpenClaw 版本 ${manifest.openclawVersion}: ${stdout}`
    );
  }

  console.log(`[verify] ${label} smoke test`);
  console.log(`  command: ${nodeExecutable} ${basename(openclawEntry)} --version`);
  console.log(`  stdout: ${stdout}`);
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const target = args.target ?? '';
  const platform = normalizePlatform(args.platform);
  const smoke = args.smoke === 'true';

  const releaseRuntimeDir = getRuntimeDir(target);
  const releaseRuntime = verifyRuntimeLayout(releaseRuntimeDir, platform, 'release runtime');
  if (smoke) {
    smokeTestRuntime(releaseRuntimeDir, platform, 'release runtime', releaseRuntime);
  }

  if (platform === 'macos') {
    const appRuntimeDir = getMacAppRuntimeDir(target);
    const appRuntime = verifyRuntimeLayout(appRuntimeDir, platform, 'macOS app resources');
    if (smoke) {
      smokeTestRuntime(appRuntimeDir, platform, 'macOS app resources', appRuntime);
    }
  }

  console.log('[verify] bundled runtime 检查通过');
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
