#!/usr/bin/env node

/**
 * Post-install script
 * Downloads platform-specific TyCode binary from GitHub releases
 */

const fs = require('fs');
const path = require('path');
const os = require('os');
const https = require('https');
const { execSync } = require('child_process');

const REPO_OWNER = 'eronic-company';
const REPO_NAME = 'TyCode';
const RELEASE_URL = `https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/releases/latest`;

function getPlatformInfo() {
  const platform = os.platform();
  const arch = os.arch();

  // Map Node.js arch names to Rust target names
  const archMap = {
    'x64': 'x86_64',
    'x32': 'i686',
    'arm': 'armv7',
    'arm64': 'aarch64'
  };

  const platformMap = {
    'linux': 'linux',
    'darwin': 'macos',
    'win32': 'windows'
  };

  return {
    platform: platformMap[platform] || platform,
    arch: archMap[arch] || arch,
    binaryName: platform === 'win32' ? 'tycode.exe' : 'tycode'
  };
}

async function getLatestRelease() {
  return new Promise((resolve, reject) => {
    const options = {
      headers: {
        'User-Agent': 'TyCode-npm-install'
      }
    };

    https.get(RELEASE_URL, options, (res) => {
      let data = '';
      res.on('data', chunk => data += chunk);
      res.on('end', () => {
        if (res.statusCode === 200) {
          try {
            resolve(JSON.parse(data));
          } catch (e) {
            reject(new Error('Failed to parse release info'));
          }
        } else {
          reject(new Error(`Failed to fetch release: ${res.statusCode}`));
        }
      });
    }).on('error', reject);
  });
}

function findAsset(release, platform, arch) {
  const patterns = [
    `tycode-${platform}-${arch}.tar.gz`,
    `tycode-${platform}-${arch}.zip`,
    `tycode-${platform}-${arch}`
  ];

  for (const pattern of patterns) {
    const asset = release.assets.find(a => a.name === pattern);
    if (asset) return asset;
  }

  // Fallback: loose pattern matching
  return release.assets.find(asset =>
    asset.name.includes(platform) && asset.name.includes(arch)
  );
}

async function downloadFile(url, destPath) {
  return new Promise((resolve, reject) => {
    const dir = path.dirname(destPath);
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true });
    }

    const file = fs.createWriteStream(destPath);
    const options = {
      headers: {
        'User-Agent': 'TyCode-npm-install'
      }
    };

    https.get(url, options, (res) => {
      // Handle redirects
      if (res.statusCode === 302 || res.statusCode === 301) {
        file.close();
        downloadFile(res.headers.location, destPath).then(resolve).catch(reject);
        return;
      }

      if (res.statusCode !== 200) {
        file.close();
        fs.unlink(destPath, () => {});
        reject(new Error(`HTTP ${res.statusCode}`));
        return;
      }

      res.pipe(file);
      file.on('finish', () => {
        file.close();
        resolve();
      });
    }).on('error', (err) => {
      file.close();
      fs.unlink(destPath, () => {});
      reject(err);
    });
  });
}

async function install() {
  try {
    const { platform, arch, binaryName } = getPlatformInfo();
    console.log(`[TyCode] Installing for ${platform}-${arch}...`);

    const release = await getLatestRelease();
    const asset = findAsset(release, platform, arch);

    if (!asset) {
      console.warn(
        `[TyCode] ⚠ No pre-built binary for ${platform}-${arch}\n` +
        `[TyCode] Available assets:`
      );
      release.assets.forEach(a => console.warn(`[TyCode]   - ${a.name}`));
      console.warn(
        `[TyCode] Visit: https://github.com/${REPO_OWNER}/${REPO_NAME}/releases\n` +
        `[TyCode] Or build from source: cargo build --release`
      );
      return;
    }

    const destDir = path.join(__dirname, '..', 'bin', 'binaries', platform, arch);
    const destPath = path.join(destDir, binaryName);

    console.log(`[TyCode] Downloading ${asset.name}...`);
    await downloadFile(asset.browser_download_url, destPath);

    // Make binary executable on Unix
    if (process.platform !== 'win32') {
      fs.chmodSync(destPath, 0o755);
    }

    console.log(`[TyCode] ✓ Installed successfully!`);
    console.log(`[TyCode] Run: tycode`);

  } catch (error) {
    console.error(`[TyCode] ✗ Installation failed: ${error.message}`);
    console.error(`[TyCode] Debug info:`);
    console.error(`[TyCode]   Platform: ${os.platform()} ${os.arch()}`);
    console.error(`[TyCode]   Node: ${process.version}`);
    console.error(`[TyCode]`);
    console.error(`[TyCode] Please report: https://github.com/${REPO_OWNER}/${REPO_NAME}/issues`);
  }
}

install();
