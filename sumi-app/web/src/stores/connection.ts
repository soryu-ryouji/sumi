// 连接状态：server 地址/token、app 能力、解锁票据。boot 流程在 app/boot.ts。
import { defineStore } from 'pinia';
import { api, configureApi, setUnlockTickets } from '@/shared/api/client';
import type { AppInfo } from '@/shared/api/types';

export const useConnection = defineStore('connection', {
  state: () => ({
    /** server 是否就绪（地址与 token 已注入 client） */
    ready: false,
    address: '',
    token: '',
    appInfo: null as AppInfo | null,
    /** 解锁票据（lock/unlock 发放，daemon 内存态；随请求经 X-Sumi-Unlock 携带） */
    unlockTickets: [] as string[],
  }),
  getters: {
    writable: (s) => s.appInfo?.writable ?? false,
    isAdmin: (s) => s.appInfo?.access === 'admin',
  },
  actions: {
    /** server 就绪（冷启动/换库/重启）：重配 client 并拉取能力信息 */
    async connect(address: string, token: string) {
      configureApi(address, token);
      this.address = address;
      this.token = token;
      this.appInfo = await api<AppInfo>('/app/info');
      this.ready = true;
    },
    disconnect() {
      this.ready = false;
      this.address = '';
      this.token = '';
      this.appInfo = null;
    },
    addUnlockTicket(t: string) {
      if (t && !this.unlockTickets.includes(t)) {
        this.unlockTickets.push(t);
        setUnlockTickets(this.unlockTickets);
      }
    },
  },
});
