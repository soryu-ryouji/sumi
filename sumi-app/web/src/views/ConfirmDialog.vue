<script setup lang="ts">
// 通用确认对话框（删除文件夹等危险操作）：Enter 确认、Esc 取消；danger 时确定按钮红色。
// 遮罩/Esc/拖拽区挂起行为对齐 SettingsView 与 InputDialog。
import { onMounted, onUnmounted } from 'vue';

const props = defineProps<{
  title: string;
  message: string;
  danger?: boolean;
  confirmLabel?: string;
}>();
const emit = defineEmits<{ confirm: []; cancel: [] }>();

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
</script>

<template>
  <Teleport to="body">
    <div class="mask" @pointerdown="onMaskDown" @pointerup="onMaskUp">
      <div class="dialog" role="dialog" aria-modal="true" :aria-label="title" @keydown.enter.prevent="emit('confirm')" @keydown.esc.prevent="emit('cancel')">
        <div class="dialog-title">{{ title }}</div>
        <div class="dialog-message">{{ message }}</div>
        <div class="dialog-actions">
          <button class="btn" @click="emit('cancel')">取消</button>
          <button class="btn" :class="danger ? 'danger' : 'primary'" @click="emit('confirm')">{{ confirmLabel ?? '确定' }}</button>
        </div>
      </div>
    </div>
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
  width: min(380px, calc(100vw - 32px));
  border-radius: 10px;
  background: var(--panel);
  border: 1px solid var(--border);
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.4);
  padding: 16px;
}
.dialog-title {
  font-weight: 600;
  margin-bottom: 10px;
}
.dialog-message {
  font-size: 13px;
  color: var(--muted);
  margin-bottom: 16px;
  white-space: pre-wrap;
  word-break: break-all;
}
.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
/* danger 确定钮：红字红框，悬停填充红底（styles.css 的 .btn.danger 只有红字） */
.btn.danger {
  border-color: var(--danger);
}
.btn.danger:hover {
  background: var(--danger);
  border-color: var(--danger);
  color: #fff;
}
</style>
