// 窗口：主窗口生命周期与加载（macOS hidden titlebar + 红绿灯；Windows/Linux 无边框自绘控制）。
import { app, BrowserWindow } from 'electron';
import path from 'node:path';
import fs from 'node:fs';
import { APP_DIR, APP_ICON, ELECTRON_DIR } from './paths';
import { IPC, type ServerConn } from './ipc-contract';

const isDev = !app.isPackaged;

let mainWindow: BrowserWindow | null = null;

export function getMainWindow(): BrowserWindow | null {
  return mainWindow;
}

export function showMainWindow(): void {
  if (!mainWindow) {
    return;
  }
  if (mainWindow.isMinimized()) {
    mainWindow.restore();
  }
  mainWindow.show();
  mainWindow.focus();
}

export function loadMainPage(conn?: ServerConn): void {
  const hash = conn ? `api=${encodeURIComponent(conn.address)}&token=${conn.token}` : '';
  if (isDev) {
    mainWindow?.loadURL(`http://localhost:5173/${hash ? `#${hash}` : ''}`);
  } else {
    mainWindow?.loadFile(path.join(APP_DIR, 'web', 'dist', 'index.html'), { hash });
  }
}

export function createWindow(): void {
  mainWindow = new BrowserWindow({
    width: 1440,
    height: 900,
    minWidth: 960,
    minHeight: 600,
    show: false,
    backgroundColor: '#1e1e1e',
    // macOS：隐藏系统标题栏但保留原生红绿灯（全屏行为由系统保证）；
    // Windows/Linux：无边框，窗口控制由前端自绘
    ...(process.platform === 'darwin' ? { titleBarStyle: 'hidden' as const, trafficLightPosition: { x: 12, y: 14 } } : { frame: false }),
    // 开发态 / Linux 的窗口图标；打包后各平台图标由 electron-builder 嵌入
    icon: APP_ICON,
    webPreferences: {
      // 产物 preload.cjs 与 main.mjs 同目录（electron/out）
      preload: path.join(ELECTRON_DIR, 'preload.cjs'),
    },
  });
  const win = mainWindow;
  // 首帧渲染完成后再显示窗口：提前 show 会把空白/白窗暴露给用户
  win.once('ready-to-show', () => win.show());
  win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  // 同步最大化状态给渲染进程（标题栏 最大化/还原 图标切换）
  win.on('maximize', () => win.webContents.send(IPC.winMaximized, true));
  win.on('unmaximize', () => win.webContents.send(IPC.winMaximized, false));
  // 窗口内容单页生命周期：启动/引导/进度全在页面内呈现，主进程不再驱动二次导航

  // 无头自检：SUMI_SCREENSHOT=<路径> 时加载完成后截图落盘（端到端冒烟用）
  if (process.env.SUMI_SCREENSHOT) {
    win.webContents.once('did-finish-load', () => {
      const delay = Number(process.env.SUMI_SCREENSHOT_DELAY || 5000);
      setTimeout(async () => {
        const image = await win.webContents.capturePage();
        fs.writeFileSync(process.env.SUMI_SCREENSHOT as string, image.toPNG());
        console.log(`screenshot saved: ${process.env.SUMI_SCREENSHOT}`);
      }, delay);
    });
  }
}
