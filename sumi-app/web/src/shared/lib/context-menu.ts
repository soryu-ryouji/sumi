// 上下文菜单：全局单例（模块级响应式状态 + 任意组件 openContextMenu 触发）。
// 宿主组件 ContextMenuHost 在 App 根挂载一次（边缘翻转测量在宿主完成）。
import { reactive } from 'vue';

export interface CtxItem {
  label?: string;
  danger?: boolean;
  separator?: boolean;
  action?: () => void;
}

const state = reactive({
  open: false,
  x: 0,
  y: 0,
  items: [] as CtxItem[],
});

export function useContextMenuState() {
  return state;
}

/** 在鼠标位置打开菜单；item.action 内如需访问选中项，由调用方闭包捕获；
 *  超出视口右/下边缘的回翻由宿主组件测量修正 */
export function openContextMenu(items: CtxItem[], e: MouseEvent): void {
  e.preventDefault();
  e.stopPropagation();
  state.items = items;
  state.x = e.clientX;
  state.y = e.clientY;
  state.open = true;
}

export function closeContextMenu(): void {
  state.open = false;
}
