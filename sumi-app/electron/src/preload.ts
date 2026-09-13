// preload：contextBridge 暴白名单通道为 window.sumiShell（渲染进程 sandbox 下唯一的系统能力口）。
import { contextBridge, ipcRenderer } from 'electron';
import { IPC, type SumiShell } from './ipc-contract';

const api: SumiShell = {
  platform: process.platform,
  selectLibrary: () => ipcRenderer.invoke(IPC.selectLibrary),
  listLibraries: () => ipcRenderer.invoke(IPC.listLibraries),
  openLibrary: (p) => ipcRenderer.invoke(IPC.openLibrary, p),
  openLibraryFolder: (p) => ipcRenderer.invoke(IPC.openLibraryFolder, p),
  removeLibrary: (p) => ipcRenderer.invoke(IPC.removeLibrary, p),
  copyPath: (relPath) => ipcRenderer.invoke(IPC.copyPath, relPath),
  showInFolder: (relPath) => ipcRenderer.invoke(IPC.showInFolder, relPath),
  openFolder: (relPath) => ipcRenderer.invoke(IPC.openFolder, relPath),
  getPathForFile: (file) => ipcRenderer.invoke(IPC.getPathForFile, file),
  lanAddresses: () => ipcRenderer.invoke(IPC.lanAddresses),
  getCacheDir: () => ipcRenderer.invoke(IPC.cacheDirGet),
  pickCacheDir: () => ipcRenderer.invoke(IPC.cacheDirPick),
  setCacheDir: (p) => ipcRenderer.invoke(IPC.cacheDirSet, p),
  minimizeWindow: () => ipcRenderer.invoke(IPC.winMinimize),
  toggleMaximizeWindow: () => ipcRenderer.invoke(IPC.winMaximizeToggle),
  closeWindow: () => ipcRenderer.invoke(IPC.winClose),
  quitApp: () => ipcRenderer.invoke(IPC.quitApp),
  onServerStarted: (cb) => {
    const listener = (_e: unknown, payload: Parameters<typeof cb>[0]) => cb(payload);
    ipcRenderer.on(IPC.serverStarted, listener);
    return () => ipcRenderer.off(IPC.serverStarted, listener);
  },
  getServerConn: () => ipcRenderer.invoke(IPC.serverConn),
  onServerError: (cb) => {
    const listener = (_e: unknown, payload: Parameters<typeof cb>[0]) => cb(payload);
    ipcRenderer.on(IPC.serverError, listener);
    return () => ipcRenderer.off(IPC.serverError, listener);
  },
  onServerRestarting: (cb) => {
    const listener = () => cb();
    ipcRenderer.on(IPC.serverRestarting, listener);
    return () => ipcRenderer.off(IPC.serverRestarting, listener);
  },
  onServerProgress: (cb) => {
    const listener = (_e: unknown, payload: Parameters<typeof cb>[0]) => cb(payload);
    ipcRenderer.on(IPC.serverProgress, listener);
    return () => ipcRenderer.off(IPC.serverProgress, listener);
  },
  onWindowMaximized: (cb) => {
    const listener = (_e: unknown, payload: Parameters<typeof cb>[0]) => cb(payload);
    ipcRenderer.on(IPC.winMaximized, listener);
    return () => ipcRenderer.off(IPC.winMaximized, listener);
  },
};

contextBridge.exposeInMainWorld('sumiShell', api);
