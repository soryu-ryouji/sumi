// IPC 契约：主进程 handler、preload 暴露、web 前端消费的三方对齐点（单一定义）。
// 通道名一律经 IPC 常量引用，不手写字符串；web 端从本文件 re-export 类型。

/** IPC 通道名（invoke/handle 与 send/on 的全集） */
export const IPC = {
  // ---- 渲染进程 → 主进程（ipcRenderer.invoke / ipcMain.handle） ----
  selectLibrary: 'sumi:select-library',
  listLibraries: 'sumi:list-libraries',
  openLibrary: 'sumi:open-library',
  openLibraryFolder: 'sumi:open-library-folder',
  removeLibrary: 'sumi:remove-library',
  copyPath: 'sumi:copy-path',
  showInFolder: 'sumi:show-in-folder',
  openFolder: 'sumi:open-folder',
  getPathForFile: 'sumi:get-path-for-file',
  lanAddresses: 'sumi:lan-addresses',
  pickApp: 'sumi:pick-app',
  cacheDirGet: 'sumi:cache-dir-get',
  cacheDirPick: 'sumi:cache-dir-pick',
  cacheDirSet: 'sumi:cache-dir-set',
  winMinimize: 'sumi:win-minimize',
  winMaximizeToggle: 'sumi:win-maximize-toggle',
  winClose: 'sumi:win-close',
  quitApp: 'sumi:quit-app',
  // ---- 主进程 → 渲染进程（webContents.send / ipcRenderer.on） ----
  winMaximized: 'sumi:win-maximized',
  serverProgress: 'sumi:server-progress',
  serverRestarting: 'sumi:server-restarting',
  serverStarted: 'sumi:server-started',
  serverConn: 'sumi:server-conn',
  serverError: 'sumi:server-error',
} as const;

/** 素材库历史条目（主进程记录，最近使用在前） */
export interface LibraryHistoryItem {
  path: string;
  name: string;
  exists: boolean;
}

export interface LibraryList {
  current: string | null;
  libraries: LibraryHistoryItem[];
}

/** server 就绪事件负载（冷启动/换库都会到达，会换端口） */
export interface ServerConn {
  address: string;
  token: string;
}

/** server 索引进度事件负载（total=0 表示不定态） */
export interface ServerProgress {
  phase: string;
  processed: number;
  total: number;
}

/** Electron preload 注入的白名单通道（window.sumiShell；浏览器纯前端调试时不存在） */
export interface SumiShell {
  platform: string;
  /** 更换书库：弹目录选择框并拉起新 server；就绪经 onServerStarted 通知 */
  selectLibrary(): Promise<boolean>;
  /** 本机打开过的书库历史与当前库路径 */
  listLibraries(): Promise<LibraryList>;
  /** 打开历史书库（仅限历史记录内的路径） */
  openLibrary(path: string): Promise<boolean>;
  /** 在系统文件管理器中打开书库目录（仅限历史记录内的路径） */
  openLibraryFolder(path: string): Promise<void>;
  /** 从历史记录移除一条书库（不动目录本身），返回移除后的列表 */
  removeLibrary(path: string): Promise<LibraryList>;
  /** 复制库内文件的绝对路径到剪贴板（相对路径） */
  copyPath(relPath: string): Promise<void>;
  /** 在系统文件管理器中显示库内文件（相对路径） */
  showInFolder(relPath: string): Promise<void>;
  /** 在系统文件管理器中打开库内文件夹本身 */
  openFolder(relPath: string): Promise<void>;
  /** 拖拽导入时取文件绝对路径（Electron webUtils；浏览器下返回空） */
  getPathForFile(file: File): Promise<string>;
  /** 本机局域网 IPv4 地址列表（设置面板展示用；LAN 配置读写走 REST app/lan） */
  lanAddresses(): Promise<string[]>;
  /** 弹应用选择框（打开方式配置用；macOS 可选 .app，Windows/Linux 选可执行文件）。取消返回 null */
  pickApp(): Promise<string | null>;
  /** 当前缓存父目录（isDefault=true 表示 daemon 系统默认） */
  getCacheDir(): Promise<{ current: string; isDefault: boolean }>;
  /** 弹目录选择框选新缓存父目录（取消返回 null） */
  pickCacheDir(): Promise<string | null>;
  /** 保存缓存父目录偏好并重启 server（就绪经 onServerStarted）；传 null 恢复系统默认 */
  setCacheDir(path: string | null): Promise<void>;
  minimizeWindow(): Promise<void>;
  /** 最大化/还原切换，返回切换后的最大化状态 */
  toggleMaximizeWindow(): Promise<boolean>;
  closeWindow(): Promise<void>;
  /** 真正退出应用（启动错误屏用） */
  quitApp(): Promise<void>;
  /** 订阅 server 就绪（携带新地址与 token，需重配 API 并重启数据），返回退订函数。
   *  事件只发一次：页面（重）加载晚于就绪时会丢失，须与 getServerConn 拉取配合 */
  onServerStarted(cb: (conn: ServerConn) => void): () => void;
  /** 拉取当前已就绪的 server 连接（未就绪返回 null）：页面加载晚于 server 就绪的竞态兜底 */
  getServerConn(): Promise<ServerConn | null>;
  /** 订阅 server 启动/运行失败，返回退订函数 */
  onServerError(cb: (error: { message: string }) => void): () => void;
  /** 订阅 server 即将重启（旧 server 已停，应立即切启动屏），返回退订函数 */
  onServerRestarting(cb: () => void): () => void;
  /** 订阅 server 扫描进度（应用内启动屏用），返回退订函数 */
  onServerProgress(cb: (progress: ServerProgress) => void): () => void;
  /** 订阅最大化状态变化（含 Aero Snap 等系统途径），返回退订函数 */
  onWindowMaximized(cb: (maximized: boolean) => void): () => void;
}
