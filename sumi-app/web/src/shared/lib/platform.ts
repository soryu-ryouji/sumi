// 平台与运行形态判定（布局避让分支：macOS 红绿灯在左上、无边框自绘控制在右上）。
import { shell } from '@/app/shell';

/** Electron 壳是否存在（纯浏览器/LAN web 查看时为 false：无窗口控件，不做避让） */
export const hasShell = (): boolean => shell() !== null;

/** 是否 macOS（原生红绿灯压窗口左上角，约 x:12-78, y:14-30；仅 Electron 壳下成立） */
export const isMac = (): boolean => shell()?.platform === 'darwin';

/** 窗口左上角红绿灯避让宽度（macOS；三钮含间隙） */
export const TRAFFIC_INSET = 80;

/** 窗口右上角自绘控制避让宽度（Windows/Linux；3 × 44px 按钮） */
export const CONTROLS_INSET = 136;

/** 窗口右上角自绘控制条高度（Windows/Linux；WindowControls 与内容区顶部避让共用） */
export const CONTROLS_HEIGHT = 38;

/** 双击拖拽区切换最大化（macOS 惯例；控件上双击不触发由调用方过滤） */
export function dragDoubleclickMaximize(e: MouseEvent): void {
  if ((e.target as HTMLElement).closest('button, input, select, a')) {
    return;
  }
  void shell()?.toggleMaximizeWindow();
}
