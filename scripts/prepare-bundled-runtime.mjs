#!/usr/bin/env node

import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, basename } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '..');
const runtimeDir = resolve(repoRoot, 'src-tauri', 'runtime');
const manifestPath = join(runtimeDir, 'manifest.json');

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

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    encoding: 'utf8',
    stdio: 'pipe',
    ...options,
  });

  if (result.status !== 0) {
    throw new Error(
      [
        `${command} ${args.join(' ')} 执行失败`,
        result.error ? `spawn error: ${result.error.message}` : '',
        result.status !== null ? `exit code: ${result.status}` : '',
        result.stdout?.trim(),
        result.stderr?.trim(),
      ]
        .filter(Boolean)
        .join('\n')
    );
  }

  return typeof result.stdout === 'string' ? result.stdout.trim() : '';
}

function ensureDir(path) {
  mkdirSync(path, { recursive: true });
}

function cleanPath(path) {
  rmSync(path, { recursive: true, force: true });
}

function copyDir(source, target) {
  cleanPath(target);
  cpSync(source, target, {
    recursive: true,
    force: true,
    dereference: true,
    verbatimSymlinks: false,
  });
}

function resolveNodeRoot(nodeBin) {
  const absoluteNodeBin = resolve(nodeBin);
  const parent = dirname(absoluteNodeBin);
  return basename(parent).toLowerCase() === 'bin' ? dirname(parent) : parent;
}

function resolvePackagedOpenClawRoot(prefix) {
  const candidates = [
    join(prefix, 'lib', 'node_modules', 'openclaw'),
    join(prefix, 'node_modules', 'openclaw'),
  ];

  return candidates.find((candidate) => existsSync(candidate));
}

function readPackageVersion(packageRoot) {
  const packageJsonPath = join(packageRoot, 'package.json');
  const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8'));
  return packageJson.version ?? 'unknown';
}

function getDefaultStagingPrefix() {
  if (process.platform === 'win32') {
    return join(process.env.RUNNER_TEMP ?? process.env.TEMP ?? process.env.TMP ?? tmpdir(), 'ocrt');
  }

  return resolve(repoRoot, '.tmp', 'bundled-openclaw-runtime');
}

function getDefaultNpmCacheDir() {
  if (process.platform === 'win32') {
    return join(
      process.env.RUNNER_TEMP ?? process.env.TEMP ?? process.env.TMP ?? tmpdir(),
      'ocrt-cache'
    );
  }

  return resolve(repoRoot, '.tmp', 'bundled-openclaw-npm-cache');
}

function installOpenClawWithNpm(stagingPrefix, openclawVersion, env) {
  const npmArgs = [
    'install',
    '--global',
    `--prefix=${stagingPrefix}`,
    `openclaw@${openclawVersion}`,
  ];

  if (process.platform === 'win32') {
    const command = process.env.ComSpec ?? process.env.comspec ?? 'cmd.exe';
    const script = `npm.cmd ${npmArgs.map(quoteForWindowsCmd).join(' ')}`;
    return run(command, ['/d', '/s', '/c', script], {
      stdio: 'inherit',
      env,
    });
  }

  return run('npm', npmArgs, {
    stdio: 'inherit',
    env,
  });
}

function quoteForWindowsCmd(value) {
  if (!/[\s"]/u.test(value)) {
    return value;
  }

  return `"${value.replace(/"/g, '""')}"`;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const nodeBin = args['node-bin'] ?? process.env.BOSICLAW_NODE_BIN ?? process.execPath;
  const openclawVersion =
    args['openclaw-version'] ?? process.env.OPENCLAW_VERSION ?? 'latest';
  const packageRootArg =
    args['openclaw-package-root'] ?? process.env.OPENCLAW_PACKAGE_ROOT ?? '';
  const stagingPrefix =
    args['staging-prefix'] ??
    process.env.OPENCLAW_STAGING_PREFIX ??
    getDefaultStagingPrefix();
  const npmCacheDir =
    args['npm-cache-dir'] ??
    process.env.OPENCLAW_NPM_CACHE ??
    getDefaultNpmCacheDir();

  const nodeRoot = resolveNodeRoot(nodeBin);
  if (!existsSync(nodeRoot)) {
    throw new Error(`找不到 Node 运行时目录: ${nodeRoot}`);
  }

  let packageRoot = packageRootArg ? resolve(packageRootArg) : '';

  if (!packageRoot) {
    cleanPath(stagingPrefix);
    ensureDir(stagingPrefix);
    ensureDir(npmCacheDir);

    console.log(`使用 npm 安装 openclaw@${openclawVersion} 到临时前缀: ${stagingPrefix}`);
    if (process.platform === 'win32') {
      console.log(`Windows CI 使用短 npm cache 目录: ${npmCacheDir}`);
    }
    installOpenClawWithNpm(stagingPrefix, openclawVersion, {
      ...process.env,
      SHARP_IGNORE_GLOBAL_LIBVIPS:
        process.env.SHARP_IGNORE_GLOBAL_LIBVIPS ?? '1',
      npm_config_cache: npmCacheDir,
      npm_config_loglevel: process.env.OPENCLAW_NPM_LOGLEVEL ?? 'notice',
    });

    packageRoot = resolvePackagedOpenClawRoot(stagingPrefix) ?? '';
  }

  if (!packageRoot || !existsSync(packageRoot)) {
    throw new Error(
      '未能解析 OpenClaw 包目录。可通过 --openclaw-package-root 指定，或确保 npm 安装成功。'
    );
  }

  const entryCandidates = [join(packageRoot, 'openclaw.mjs'), join(packageRoot, 'dist', 'entry.js')];
  if (!entryCandidates.some((candidate) => existsSync(candidate))) {
    throw new Error(`OpenClaw 包目录缺少入口文件: ${packageRoot}`);
  }

  const targetNodeRoot = join(runtimeDir, 'tools', 'node');
  const targetPackageRoot = join(runtimeDir, 'lib', 'node_modules', 'openclaw');

  ensureDir(runtimeDir);
  ensureDir(dirname(targetNodeRoot));
  ensureDir(dirname(targetPackageRoot));

  console.log(`复制 Node 运行时: ${nodeRoot} -> ${targetNodeRoot}`);
  copyDir(nodeRoot, targetNodeRoot);

  console.log(`复制 OpenClaw 包: ${packageRoot} -> ${targetPackageRoot}`);
  copyDir(packageRoot, targetPackageRoot);

  const manifest = {
    generatedAt: new Date().toISOString(),
    nodeVersion: run(resolve(nodeBin), ['--version']),
    openclawVersion: readPackageVersion(packageRoot),
    runtimeLayoutVersion: 1,
    targetPlatform: process.platform,
    targetArch: process.arch,
    paths: {
      nodeRoot: 'tools/node',
      openclawPackageRoot: 'lib/node_modules/openclaw',
    },
  };

  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

  console.log('内置运行时准备完成。');
  console.log(`Node 版本: ${manifest.nodeVersion}`);
  console.log(`OpenClaw 版本: ${manifest.openclawVersion}`);
  console.log(`运行时目录: ${runtimeDir}`);
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
