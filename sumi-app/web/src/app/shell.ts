// window.sumiShell 访问（preload 注入；浏览器纯前端调试时不存在）。
import type { SumiShell } from '@/shared/api/types';

declare global {
  interface Window {
    sumiShell?: SumiShell;
  }
}

export function shell(): SumiShell | null {
  return window.sumiShell ?? null;
}
