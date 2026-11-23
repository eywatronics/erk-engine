// The smallest Electron app that shows a page: one 800x600 window, no
// menu, the page from a file. The memory comparison (../compare.ps1)
// starts it with the page's path, and `--no-gpu` to draw without the GPU.
const { app, BrowserWindow } = require('electron');
const path = require('path');

const page = process.argv.find((arg) => arg.endsWith('.html'));
if (process.argv.includes('--no-gpu')) {
  app.disableHardwareAcceleration();
}

app.whenReady().then(() => {
  const window = new BrowserWindow({
    width: 800,
    height: 600,
    useContentSize: true,
    webPreferences: { sandbox: true, contextIsolation: true },
  });
  window.setMenu(null);
  window.loadFile(path.resolve(page));
});

app.on('window-all-closed', () => app.quit());
