// Getting a card out of the terminal: a file on disk and the system clipboard.

const fs = require('fs');
const os = require('os');
const path = require('path');
const { execFileSync } = require('child_process');
const crypto = require('crypto');
const cinema = require('./vestige-cinema.js');

function outDir(env = process.env, home = os.homedir()) {
  if (env.VESTIGE_REM_OUT) return env.VESTIGE_REM_OUT;
  const downloads = path.join(home, 'Downloads');
  return fs.existsSync(downloads) ? downloads : home;
}

function saveArtifact(buffer, fileName, env = process.env) {
  if (!/^[A-Za-z0-9._-]+$/.test(fileName) || fileName.startsWith('.')) throw new Error('not a plain file name');
  const dir = outDir(env);
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, fileName);
  // write a fresh file opened exclusively, then rename it over the name: rename replaces the
  // directory entry itself, so a planted symlink or hard link there is never written through
  const tmp = path.join(dir, `.${fileName}.${process.pid}.${crypto.randomBytes(6).toString('hex')}.part`);
  fs.writeFileSync(tmp, buffer, { flag: 'wx', mode: 0o644 });
  try { fs.renameSync(tmp, file); } catch (error) { try { fs.unlinkSync(tmp); } catch {} throw error; }
  return file;
}

const runQuiet = (command, args, options = {}) => execFileSync(command, args, { stdio: ['pipe', 'ignore', 'ignore'], timeout: 8000, ...options });
const appleString = (text) => '"' + text.replace(/\\/g, '\\\\').replace(/"/g, '\\"') + '"';

function attempt(steps) {
  for (const [via, step] of steps) {
    try { step(); return { ok: true, via }; } catch {}
  }
  return { ok: false, via: '' };
}

// Puts the picture itself on the clipboard, so a paste into a post or a chat attaches the image.
function copyImage(file, { platform = process.platform, run = runQuiet } = {}) {
  if (platform === 'darwin') return attempt([['osascript', () => run('osascript', ['-e', `set the clipboard to (read (POSIX file ${appleString(file)}) as «class PNGf»)`])]]);
  if (platform === 'win32') return attempt([['powershell', () => run('powershell', ['-NoProfile', '-Command', 'Add-Type -AssemblyName System.Windows.Forms,System.Drawing; [System.Windows.Forms.Clipboard]::SetImage([System.Drawing.Image]::FromFile($args[0]))', file])]]);
  return attempt([
    ['wl-copy', () => run('wl-copy', ['--type', 'image/png'], { input: fs.readFileSync(file) })],
    ['xclip', () => run('xclip', ['-selection', 'clipboard', '-t', 'image/png', '-i', file])],
  ]);
}

// Puts a reference to the file on the clipboard, which is how an animated GIF survives a paste.
function copyFile(file, { platform = process.platform, run = runQuiet } = {}) {
  if (platform === 'darwin') return attempt([['osascript', () => run('osascript', ['-e', `set the clipboard to (POSIX file ${appleString(file)})`])]]);
  if (platform === 'win32') return attempt([['powershell', () => run('powershell', ['-NoProfile', '-Command', 'Set-Clipboard -Path $args[0]', file])]]);
  const uri = 'file://' + file.split('/').map(encodeURIComponent).join('/') + '\n';
  return attempt([
    ['wl-copy', () => run('wl-copy', ['--type', 'text/uri-list'], { input: uri })],
    ['xclip', () => run('xclip', ['-selection', 'clipboard', '-t', 'text/uri-list'], { input: uri })],
  ]);
}

function fileLink(file) {
  return cinema.hyperlink(file, 'file://' + file.split('/').map(encodeURIComponent).join('/'));
}

module.exports = { outDir, saveArtifact, copyImage, copyFile, fileLink, appleString };
