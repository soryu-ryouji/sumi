<script setup lang="ts">
// 上下文菜单宿主：全局遮罩 + 菜单位置 + Esc/滚动/右键关闭 + 边缘回翻。
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { closeContextMenu, useContextMenuState, type CtxItem } from '@/shared/lib/context-menu';

const state = useContextMenuState();
const menuEl = ref<HTMLElement | null>(null);

function run(item: CtxItem): void {
  closeContextMenu();
  item.action?.();
}

function onKeydown(e: KeyboardEvent): void {
  if (e.key === 'Escape') {
    closeContextMenu();
  }
}

// 打开后量实际尺寸修正：超出视口右/下边缘则向回翻
watch(
  () => state.open,
  (open) => {
    if (!open) {
      return;
    }
    void nextTick(() => {
      const el = menuEl.value;
      if (!el) {
        return;
      }
      const rect = el.getBoundingClientRect();
      if (state.x + rect.width > window.innerWidth - 8) {
        state.x = Math.max(8, state.x - rect.width);
      }
      if (state.y + rect.height > window.innerHeight - 8) {
        state.y = Math.max(8, state.y - rect.height);
      }
    });
  },
);

onMounted(() => window.addEventListener('keydown', onKeydown));
onUnmounted(() => window.removeEventListener('keydown', onKeydown));
</script>

<template>
  <Teleport to="body">
    <div v-if="state.open" class="ctx-mask" @click="closeContextMenu" @contextmenu.prevent="closeContextMenu" @wheel="closeContextMenu">
      <div ref="menuEl" class="ctx-menu" :style="{ left: state.x + 'px', top: state.y + 'px' }" @click.stop>
        <template v-for="(item, i) in state.items" :key="i">
          <div v-if="item.separator" class="ctx-sep" />
          <button v-else class="ctx-item" :class="{ danger: item.danger }" @click="run(item)">
            <span class="ctx-check">{{ item.checked ? '✓' : '' }}</span>{{ item.label }}
          </button>
        </template>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.ctx-mask {
  position: fixed;
  inset: 0;
  z-index: 900;
}
.ctx-menu {
  position: fixed;
  min-width: 160px;
  padding: 5px;
  background: var(--panel-2);
  border: 1px solid var(--border);
  border-radius: 9px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.45);
  display: flex;
  flex-direction: column;
}
.ctx-item {
  text-align: left;
  padding: 7px 12px;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--text);
  font-size: 13px;
  cursor: pointer;
  white-space: nowrap;
}
.ctx-item:hover {
  background: var(--accent-dim);
  color: #fff;
}
.ctx-item.danger {
  color: var(--danger);
}
.ctx-item.danger:hover {
  background: var(--danger);
  color: #fff;
}
.ctx-sep {
  height: 1px;
  background: var(--border);
  margin: 4px 8px;
}
/* 勾选位：固定宽度保持无勾项同样缩进对齐 */
.ctx-check {
  display: inline-block;
  width: 16px;
  color: var(--accent);
}
</style>
