// 白名单 IPC：窗口控制、书库选择/历史、文件管理器/剪贴板、局域网地址、缓存父目录。
// 业务数据一律走 REST，不经 IPC。
import { app, clipboard, dialog, ipcMain, shell } from 'electron';
import electron from 'electron';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { getMainWindow } from './window';
import { getLibraryRoot, listLibraries, readConfig, removeLibraryHistory, writeConfig } from './app-config';
import { getStartedConn, openLibraryAt, pickLibrary, restartServerWithCacheParent } from './server';
import { IPC } from './ipc-contract';

// webUtils 不在 electron 的 ESM 命名导出里，经 default export 解构
const { webUtils } = electron as typeof import('electron');

/** 库内相对路径 → 绝对路径（越界与空值拒绝；主进程是路径换算的唯一可信方） */
function resolveInLibrary(relPath: string): string | null {
  const root = getLibraryRoot();
  if (!root) {
    return null;
  }
  const abs = path.resolve(root, relPath);
  if (abs !== root && !abs.startsWith(root + path.sep)) {
    return null;
  }
  return abs;
}

/** 历史记录内的库路径才允许直达打开/在文件管理器中展示（防任意目录访问） */
function historyPath(p: string): string | null {
  return (readConfig().libraryHistory ?? []).includes(p) ? p : null;
}

export function registerIpc(): void {
  ipcMain.handle(IPC.winMinimize, () => getMainWindow()?.minimize());
  ipcMain.handle(IPC.winMaximizeToggle, () => {
    const win = getMainWindow();
    if (!win) {
      return false;
    }
    if (win.isMaximized()) {
      win.unmaximize();
    } else {
      win.maximize();
    }
    return win.isMaximized();
  });
  ipcMain.handle(IPC.winClose, () => getMainWindow()?.close());

  ipcMain.handle(IPC.selectLibrary, async (): Promise<boolean> => {
    const selected = await pickLibrary();
    if (!selected) {
      return false;
    }
    try {
      // 端口/token 即选即生成；页面切应用内启动屏，就绪经 sumi:server-started 通知
      await openLibraryAt(selected);
      return true;
    } catch (error) {
      dialog.showErrorBox('sumi-daemon 启动失败', String(error instanceof Error ? error.message : error));
      return false;
    }
  });

  ipcMain.handle(IPC.listLibraries, () => listLibraries());

  ipcMain.handle(IPC.openLibrary, async (_event, p: string): Promise<boolean> => {
    const libPath = historyPath(p);
    if (!libPath) {
      return false;
    }
    try {
      await openLibraryAt(libPath);
      return true;
    } catch (error) {
      dialog.showErrorBox('sumi-daemon 启动失败', String(error instanceof Error ? error.message : error));
      return false;
    }
  });

  ipcMain.handle(IPC.openLibraryFolder, (_event, p: string): void => {
    const libPath = historyPath(p);
    if (libPath) {
      shell.openPath(libPath);
    }
  });

  ipcMain.handle(IPC.removeLibrary, (_event, p: string) => removeLibraryHistory(p));

  ipcMain.handle(IPC.copyPath, (_event, relPath: string): void => {
    const abs = resolveInLibrary(relPath);
    if (abs) {
      clipboard.writeText(abs);
    }
  });

  ipcMain.handle(IPC.showInFolder, (_event, relPath: string): void => {
    const abs = resolveInLibrary(relPath);
    if (abs && fs.existsSync(abs)) {
      shell.showItemInFolder(abs);
    }
  });

  ipcMain.handle(IPC.openFolder, (_event, relPath: string): void => {
    const abs = resolveInLibrary(relPath);
    if (abs && fs.existsSync(abs)) {
      shell.openPath(abs);
    }
  });

  // 拖拽导入：File 对象 → 绝对路径（浏览器下无此能力，返回空串由前端回退 base64 上传）
  ipcMain.handle(IPC.getPathForFile, (_event, file: File): string => {
    try {
      return webUtils.getPathForFile(file);
    } catch {
      return '';
    }
  });

  // 本机局域网 IPv4（设置面板 LAN 配置展示访问地址用）
  ipcMain.handle(IPC.lanAddresses, (): string[] => {
    const out: string[] = [];
    const ifs = os.networkInterfaces();
    for (const list of Object.values(ifs)) {
      for (const net of list ?? []) {
        if (net.family === 'IPv4' && !net.internal) {
          out.push(net.address);
        }
      }
    }
    return out;
  });

  // 应用选择框（打开方式配置用）：macOS 文件选择器可直接选中 .app 包
  ipcMain.handle(IPC.pickApp, async (): Promise<string | null> => {
    const win = getMainWindow();
    if (!win) {
      return null;
    }
    const result = await dialog.showOpenDialog(win, {
      title: '选择应用',
      properties: ['openFile'],
      ...(process.platform === 'win32' ? { filters: [{ name: '应用程序', extensions: ['exe', 'bat', 'cmd', 'lnk'] }] } : {}),
    });
    return result.canceled ? null : result.filePaths[0];
  });

  ipcMain.handle(IPC.cacheDirGet, (): { current: string; isDefault: boolean } => {
    const cacheParent = readConfig().cacheParent;
    return { current: cacheParent ?? '系统默认（按平台缓存目录下的 sumi/cache）', isDefault: !cacheParent };
  });

  ipcMain.handle(IPC.cacheDirPick, async (): Promise<string | null> => {
    const win = getMainWindow();
    if (!win) {
      return null;
    }
    const result = await dialog.showOpenDialog(win, {
      title: '选择缓存父目录',
      properties: ['openDirectory', 'createDirectory'],
    });
    return result.canceled ? null : result.filePaths[0];
  });

  // 保存缓存父目录偏好并重启 server（缓存目录在 daemon 启动期决定；null 恢复系统默认）
  ipcMain.handle(IPC.cacheDirSet, async (_event, p: string | null): Promise<void> => {
    if (p === null) {
      writeConfig({ cacheParent: undefined });
    }
    await restartServerWithCacheParent(p);
  });

  ipcMain.handle(IPC.serverConn, () => getStartedConn());

  // 调试便利：渲染进程可请求退出（启动错误屏用）
  ipcMain.handle(IPC.quitApp, () => {
    app.quit();
  });
}
