// scripts/npm-postinstall.js
// Downloads the pre-compiled docgen Rust binary from GitHub Releases on npm install.
// Preserves `pnpm run docgen` compatibility after the Rust rewrite.
'use strict';

const https = require('https');
const fs = require('fs');
const path = require('path');

const REPO = 'WillIsback/ai-devops-toolkit';
const BIN_DIR = path.join(__dirname, '..', 'bin');
const BIN_PATH = path.join(BIN_DIR, process.platform === 'win32' ? 'docgen.exe' : 'docgen');

// Map Node platform/arch to GitHub Release asset names
const PLATFORM_MAP = {
  'linux-x64':   'docgen-linux-amd64',
  'linux-arm64': 'docgen-linux-arm64',
  'darwin-x64':  'docgen-macos-amd64',
  'darwin-arm64':'docgen-macos-arm64',
  'win32-x64':   'docgen-windows-amd64.exe',
};

function download(url, dest, redirects) {
  redirects = redirects || 0;
  if (redirects > 5) {
    return Promise.reject(new Error('Too many redirects'));
  }
  return new Promise(function(resolve, reject) {
    const file = fs.createWriteStream(dest);
    https.get(url, { headers: { 'User-Agent': 'npm-postinstall' } }, function(res) {
      if (res.statusCode === 301 || res.statusCode === 302) {
        file.close();
        fs.unlink(dest, function() {});
        res.resume(); // drain the redirect response body
        resolve(download(res.headers.location, dest, redirects + 1));
        return;
      }
      if (res.statusCode !== 200) {
        file.close();
        fs.unlink(dest, function() {});
        reject(new Error('HTTP ' + res.statusCode + ' for ' + url));
        return;
      }
      res.pipe(file);
      file.on('finish', function() { file.close(); resolve(); });
      file.on('error', function(err) { fs.unlink(dest, function() {}); reject(err); });
    }).on('error', function(err) { fs.unlink(dest, function() {}); reject(err); });
  });
}

async function main() {
  const key = process.platform + '-' + process.arch;
  const assetName = PLATFORM_MAP[key];

  if (!assetName) {
    console.warn('[docgen] Unsupported platform: ' + key + ' — skipping binary download');
    return;
  }

  if (!fs.existsSync(BIN_DIR)) {
    fs.mkdirSync(BIN_DIR, { recursive: true });
  }

  const url = 'https://github.com/' + REPO + '/releases/latest/download/' + assetName;
  console.log('[docgen] Downloading ' + assetName + ' from GitHub Releases...');

  try {
    await download(url, BIN_PATH);
    if (process.platform !== 'win32') {
      fs.chmodSync(BIN_PATH, 0o755);
    }
    console.log('[docgen] Binary installed to ' + BIN_PATH);
  } catch (err) {
    // Non-fatal: warn but don't fail the install
    console.warn('[docgen] Could not download binary: ' + err.message);
    console.warn('[docgen] You can manually download from https://github.com/' + REPO + '/releases');
  }
}

main().catch(function(err) {
  console.warn('[docgen] postinstall failed: ' + err.message);
  // Non-fatal: exit 0 so pnpm/npm install succeeds
});
