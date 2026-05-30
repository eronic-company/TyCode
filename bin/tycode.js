#!/usr/bin/env node

/**
 * TyCode CLI wrapper
 * Executes the platform-specific Rust binary
 */

const path = require('path');
const fs = require('fs');
const { spawnSync } = require('child_process');
const os = require('os');

function getBinaryName() {
  return os.platform() === 'win32' ? 'tycode.exe' : 'tycode';
}

// Map Node's os.platform()/os.arch() to the same directory scheme install.js
// writes to (linux/macos/windows × x86_64/aarch64/…). These MUST stay in sync
// with install.js or the wrapper will never find the downloaded binary.
function getPlatformDir() {
  const platformMap = { linux: 'linux', darwin: 'macos', win32: 'windows' };
  const archMap = { x64: 'x86_64', x32: 'i686', arm: 'armv7', arm64: 'aarch64' };
  return {
    platform: platformMap[os.platform()] || os.platform(),
    arch: archMap[os.arch()] || os.arch(),
  };
}

function findBinary() {
  const binaryName = getBinaryName();
  const binDir = path.join(__dirname, '..', 'bin', 'binaries');
  const { platform, arch } = getPlatformDir();

  // Try to find platform-specific binary
  const platformBinary = path.join(binDir, platform, arch, binaryName);
  if (fs.existsSync(platformBinary)) {
    return platformBinary;
  }

  // Fallback: Check if binary is in PATH (for development)
  try {
    const result = spawnSync('which', ['tycode'], { encoding: 'utf8' });
    if (result.status === 0 && result.stdout) {
      return result.stdout.trim();
    }
  } catch (e) {
    // Not available on Windows
  }

  console.error(`Error: TyCode binary not found for ${os.platform()} ${os.arch()}`);
  console.error(`Please reinstall: npm install -g @eronic-company/tycode`);
  process.exit(1);
}

function run() {
  const binary = findBinary();
  const args = process.argv.slice(2);

  const result = spawnSync(binary, args, {
    stdio: 'inherit',
    shell: true
  });

  process.exit(result.status || 0);
}

run();
