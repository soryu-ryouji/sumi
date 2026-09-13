<script setup lang="ts">
// 顶层状态机：无连接（引导页）→ 连接中（启动屏）→ 主界面。
// 窗口拖拽区由各视图顶部承担（DragBar / 侧栏顶条 / TopBar），无整窗通栏标题栏；
// Windows/Linux 的窗口控制为 fixed 右上角（内容按 CONTROLS_INSET 避让）。
// server 错误覆盖层在最外层兜底。
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { useConnection } from '@/stores/connection';
import { boot } from './boot';
import { shell } from './shell';
import ConnectScreen from './screens/ConnectScreen.vue';
import StartingScreen from './screens/StartingScreen.vue';
import WindowControls from './chrome/WindowControls.vue';
import ContextMenuHost from './chrome/ContextMenuHost.vue';
import MainView from '@/views/MainView.vue';
import TrashView from '@/views/TrashView.vue';
import SettingsView from '@/views/SettingsView.vue';
import { useUi } from '@/stores/ui';

const conn = useConnection();
const ui = useUi();
const serverError = ref('');
let offError: (() => void) | undefined;
onMounted(() => {
  boot();
  offError = shell()?.onServerError((e) => {
    serverError.value = e.message;
  });
});
onUnmounted(() => offError?.());
const hasHistory = ref(false);
void shell()
  ?.listLibraries()
  .then((list) => {
    hasHistory.value = list.libraries.length > 0;
  });

const screen = computed<'connect' | 'starting' | 'main'>(() => (conn.ready ? 'main' : hasHistory.value ? 'starting' : 'connect'));

function quitApp(): void {
  void shell()?.quitApp();
}
</script>

<template>
  <div class="app-root">
    <WindowControls />
    <div v-if="serverError" class="fatal-error">
      <div class="fatal-card">
        <h2>后端服务异常</h2>
        <pre>{{ serverError }}</pre>
        <div class="fatal-actions">
          <button class="btn" @click="serverError = ''">重试</button>
          <button class="btn danger" @click="quitApp">退出 sumi</button>
        </div>
      </div>
    </div>
    <template v-else>
      <ConnectScreen v-if="screen === 'connect'" />
      <StartingScreen v-else-if="screen === 'starting'" />
      <template v-else>
        <SettingsView v-if="ui.view === 'settings'" />
        <TrashView v-else-if="ui.view === 'trash'" />
        <MainView v-else />
      </template>
    </template>
    <ContextMenuHost />
    <div class="toast-wrap">
      <div v-for="t in ui.toasts" :key="t.id" class="toast" :class="t.kind">{{ t.text }}</div>
    </div>
  </div>
</template>
