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
  const platform = os.platform();
  const arch = os.arch();

  if (platform === 'win32') {
    return 'tycode.exe';
  }
  return 'tycode';
}

function findBinary() {
  const binaryName = getBinaryName();
  const binDir = path.join(__dirname, '..', 'bin', 'binaries');

  // Try to find platform-specific binary
  const platformBinary = path.join(binDir, os.platform(), os.arch(), binaryName);
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
