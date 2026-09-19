<script setup lang="ts">
// 移动到文件夹对话框：树形单选（书库根 = ''）+ 当前所在文件夹标记 + 新建子文件夹。
// 单击行选中高亮，确定 emit confirm(选中路径)；遮罩/Esc/拖拽区挂起行为对齐 SettingsView。
// Esc/Enter 在内嵌 InputDialog 打开时让路（由其自行处理，避免一次按键关两层）。
import { onMounted, onUnmounted, ref } from 'vue';
import { useEventListener } from '@vueuse/core';
import { useLibrary } from '@/stores/library';
import { createFolder } from '@/shared/lib/move';
import FolderPickerNode from './FolderPickerNode.vue';
import InputDialog from './InputDialog.vue';

const props = defineProps<{
  title: string;
  /** 当前所在文件夹（高亮标记；null/未传 = 无标记，多选混合位置场景） */
  current?: string | null;
}>();
const emit = defineEmits<{ confirm: [path: string]; cancel: [] }>();

const library = useLibrary();

/** 选中的目标文件夹（'' = 书库根）；初始停在当前所在文件夹 */
const selected = ref(props.current ?? '');
const showInput = ref(false);

function confirm(): void {
  if (showInput.value) {
    return;
  }
  emit('confirm', selected.value);
}

/** 在选中文件夹下新建子文件夹（未选中/根 = 建在根，path 即名字本身）；成功后自动选中新文件夹 */
async function onCreateFolder(name: string): Promise<void> {
  showInput.value = false;
  const path = selected.value ? `${selected.value}/${name}` : name;
  // createFolder 内部等树刷新完成后才 resolve，此时节点已可选中；失败已在内部 toast
  if (await createFolder(path)) {
    selected.value = path;
  }
}

onMounted(() => {
  document.body.classList.add('dialog-open');
});
onUnmounted(() => {
  document.body.classList.remove('dialog-open');
});

// 遮罩关闭：按下与抬起都落在遮罩上才关
let downOnMask = false;
function onMaskDown(e: PointerEvent): void {
  downOnMask = e.target === e.currentTarget;
}
function onMaskUp(e: PointerEvent): void {
  if (downOnMask && e.target === e.currentTarget) {
    emit('cancel');
  }
  downOnMask = false;
}

// Esc 关闭 / Enter 确认（捕获阶段，避免顶栏搜索等全局按键跟着触发；输入框开着时让路）
useEventListener(
  window,
  'keydown',
  (e) => {
    if (showInput.value) {
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      emit('cancel');
    } else if (e.key === 'Enter') {
      // 焦点在按钮/输入框上时放行默认激活（按钮自身语义优先，不整体确认对话框）
      const t = e.target;
      if (t instanceof HTMLButtonElement || t instanceof HTMLInputElement) {
        return;
      }
      e.preventDefault();
      confirm();
    }
  },
  { capture: true },
);
</script>

<template>
  <Teleport to="body">
    <div class="mask" @pointerdown="onMaskDown" @pointerup="onMaskUp">
      <div class="dialog" role="dialog" aria-modal="true" :aria-label="title">
        <div class="dialog-title">{{ title }}</div>
        <div class="folder-tree">
          <div class="picker-row root-row" :class="{ selected: selected === '' }" @click="selected = ''">
            <span class="picker-caret placeholder" />
            <span class="picker-name">（书库根目录）</span>
            <span v-if="current === ''" class="picker-current">当前</span>
          </div>
          <FolderPickerNode
            v-for="node in library.tree"
            :key="node.path"
            :node="node"
            :selected-path="selected"
            :current="current ?? null"
            @select="(p) => (selected = p)"
          />
        </div>
        <div class="dialog-actions">
          <button class="btn small new-folder" @click="showInput = true">新建文件夹</button>
          <button class="btn" @click="emit('cancel')">取消</button>
          <button class="btn primary" @click="confirm">确定</button>
        </div>
      </div>
    </div>
    <InputDialog v-if="showInput" title="新建文件夹" placeholder="文件夹名" confirm-label="创建" @confirm="onCreateFolder" @cancel="showInput = false" />
  </Teleport>
</template>

<style scoped>
.mask {
  position: fixed;
  inset: 0;
  z-index: 450;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.5);
}
.dialog {
  width: min(420px, calc(100vw - 32px));
  max-height: min(560px, 88vh);
  display: flex;
  flex-direction: column;
  border-radius: 10px;
  background: var(--panel);
  border: 1px solid var(--border);
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.4);
  padding: 16px;
}
.dialog-title {
  font-weight: 600;
  margin-bottom: 12px;
}
.folder-tree {
  flex: 1;
  overflow-y: auto;
  min-height: 120px;
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 6px;
  margin-bottom: 12px;
}
/* 根行/节点行样式与 FolderPickerNode 对齐（scoped 隔离，需在此复刻一份） */
.root-row,
.folder-tree :deep(.picker-row) {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  border-radius: 6px;
  cursor: pointer;
  font-size: 13px;
  min-width: 0;
}
.root-row:hover,
.folder-tree :deep(.picker-row:hover) {
  background: var(--hover);
}
.root-row.selected,
.folder-tree :deep(.picker-row.selected) {
  background: var(--accent-dim);
  color: #fff;
}
.root-row .picker-caret.placeholder {
  width: 12px;
  flex: none;
  visibility: hidden;
}
.root-row .picker-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.picker-current {
  flex: none;
  margin-left: auto;
  font-size: 10px;
  color: var(--muted);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0 4px;
}
.root-row.selected .picker-current {
  color: rgba(255, 255, 255, 0.8);
  border-color: rgba(255, 255, 255, 0.4);
}
.dialog-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}
/* 新建文件夹靠左，取消/确定靠右 */
.new-folder {
  margin-right: auto;
}
</style>
