#!/usr/bin/env node

/**
 * Pre-uninstall script
 * Cleans up downloaded binaries
 */

const fs = require('fs');
const path = require('path');

function cleanup() {
  try {
    const binariesDir = path.join(__dirname, '..', 'bin', 'binaries');
    if (fs.existsSync(binariesDir)) {
      fs.rmSync(binariesDir, { recursive: true, force: true });
      console.log('[TyCode] Cleanup complete');
    }
  } catch (error) {
    console.warn(`[TyCode] Warning: Failed to cleanup: ${error.message}`);
  }
}

cleanup();
