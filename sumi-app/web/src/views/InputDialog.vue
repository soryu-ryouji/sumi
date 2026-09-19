<script setup lang="ts">
// 通用输入对话框（重命名文件/新建文件夹等）：Teleport 遮罩居中，Enter 确认、Esc 取消，
// 输入 trim 后为空禁用确定。遮罩/Esc 行为对齐 SettingsView（按下抬起都在遮罩才关、
// 打开期间挂 body.dialog-open 挂起窗口拖拽区）。
import { onMounted, onUnmounted, ref } from 'vue';

const props = defineProps<{
  title: string;
  /** 输入框初始值（重命名场景传现名） */
  initial?: string;
  placeholder?: string;
  confirmLabel?: string;
}>();
const emit = defineEmits<{ confirm: [value: string]; cancel: [] }>();

const value = ref(props.initial ?? '');
const input = ref<HTMLInputElement | null>(null);

function confirm(): void {
  const v = value.value.trim();
  if (!v) {
    return;
  }
  emit('confirm', v);
}

onMounted(() => {
  document.body.classList.add('dialog-open');
  input.value?.focus();
  input.value?.select();
});
onUnmounted(() => {
  document.body.classList.remove('dialog-open');
});

// 遮罩关闭：按下与抬起都落在遮罩上才关（输入框拖选滑出松开不误关）
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
      <div class="dialog" role="dialog" aria-modal="true" :aria-label="title" @keydown.enter.prevent="confirm" @keydown.esc.prevent="emit('cancel')">
        <div class="dialog-title">{{ title }}</div>
        <input ref="input" v-model="value" type="text" class="dialog-input" :placeholder="placeholder" />
        <div class="dialog-actions">
          <button class="btn" @click="emit('cancel')">取消</button>
          <button class="btn primary" :disabled="!value.trim()" @click="confirm">{{ confirmLabel ?? '确定' }}</button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.mask {
  position: fixed;
  inset: 0;
  /* 与 SettingsView 同层；嵌套打开时按挂载顺序自然盖在其上 */
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
  margin-bottom: 12px;
}
.dialog-input {
  width: 100%;
  box-sizing: border-box;
  margin-bottom: 14px;
}
.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
