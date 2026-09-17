<script setup lang="ts">
// 顶层状态机：无连接（引导页）→ 连接中（启动屏）→ 主界面。
// 窗口拖拽区由各视图顶部承担（DragBar / 侧栏顶条 / TopBar），无整窗通栏标题栏；
// Windows/Linux 的窗口控制为 fixed 右上角（内容按 CONTROLS_INSET 避让）。
// server 错误覆盖层在最外层兜底。
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { useConnection } from '@/stores/connection';
import { boot } from './boot';
import { shell } from './shell';
import { startupAutoCheck } from '@/shared/lib/updater';
import ConnectScreen from './screens/ConnectScreen.vue';
import StartingScreen from './screens/StartingScreen.vue';
import WindowControls from './chrome/WindowControls.vue';
import ContextMenuHost from './chrome/ContextMenuHost.vue';
import MainView from '@/views/MainView.vue';
import SettingsView from '@/views/SettingsView.vue';
import { useUi } from '@/stores/ui';

const conn = useConnection();
const ui = useUi();
const serverError = ref('');
/** server 正在启动的预期（config 有 current 书库，或重启/换库事件到达）；
 *  判据是「有 current」而非历史条数——书库被删后主进程已清失效 current，
 *  用历史条数会把失效库误判为 server 正在启动，卡在启动屏 */
const startingExpected = ref(false);
let offError: (() => void) | undefined;
let offRestart: (() => void) | undefined;
let offStarted: (() => void) | undefined;
onMounted(() => {
  boot();
  offError = shell()?.onServerError((e) => {
    serverError.value = e.message;
    startingExpected.value = false;
  });
  offRestart = shell()?.onServerRestarting(() => {
    startingExpected.value = true;
  });
  offStarted = shell()?.onServerStarted(() => {
    startingExpected.value = false;
  });
  void shell()
    ?.listLibraries()
    .then((list) => {
      startingExpected.value = list?.current != null;
    });
});
onUnmounted(() => {
  offError?.();
  offRestart?.();
  offStarted?.();
});

const screen = computed<'connect' | 'starting' | 'main'>(() => (conn.ready ? 'main' : startingExpected.value ? 'starting' : 'connect'));

// 主界面就绪后触发一次启动静默检查（延迟 8s，每会话一次；见 shared/lib/updater.ts）
watch(
  () => conn.ready,
  (ready) => {
    if (ready) {
      startupAutoCheck();
    }
  },
);

function quitApp(): void {
  void shell()?.quitApp();
}
</script>

<template>
  <div class="app-root">
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
        <MainView />
      </template>
    </template>
    <!-- app-region 按 DOM 序合成：后到的 no-drag 才能压住前面视图的 drag 区（TopBar/DragBar 横跨全宽），
         故本组件必须排在各视图之后，否则窗口控制三钮被覆盖成拖拽区、点击失效 -->
    <WindowControls />
    <ContextMenuHost />
    <!-- 设置浮动对话框：组件自身 Teleport 到 body，挂载位置仅作语义占位；
         遮罩 z-index 低于窗口控制（500），打开期间右上三钮仍可点击 -->
    <SettingsView v-if="ui.settingsOpen" />
    <div class="toast-wrap">
      <div v-for="t in ui.toasts" :key="t.id" class="toast" :class="t.kind">{{ t.text }}</div>
    </div>
  </div>
</template>
