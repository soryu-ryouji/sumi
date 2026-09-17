// 启动编排：hash 连接参数解析、shell 事件接线、SSE 订阅。
// 单页生命周期：连接状态机（无连接 → 引导页；连接中 → 启动屏；就绪 → 主界面）全在页面内呈现。
import { useConnection } from '@/stores/connection';
import { useLibrary } from '@/stores/library';
import { useBooks } from '@/stores/books';
import { useUi } from '@/stores/ui';
import { directUrl } from '@/shared/api/client';
import { subscribeEvents } from '@/shared/api/sse';
import type { Item } from '@/shared/api/types';
import { shell } from './shell';

let unsubscribeSse: (() => void) | null = null;

/** hash 形如 `#api=http%3A%2F%2F127.0.0.1%3A1234&token=<hex>`（window.loadMainPage 注入） */
export function parseHashConn(): { address: string; token: string } | null {
  const hash = location.hash.replace(/^#/, '');
  if (!hash) {
    return null;
  }
  const params = new URLSearchParams(hash);
  const address = params.get('api');
  const token = params.get('token');
  if (!address || !token) {
    return null;
  }
  return { address, token };
}

/** server 就绪：重配连接、拉取初始数据、订阅 SSE */
export async function onServerReady(address: string, token: string): Promise<void> {
  const conn = useConnection();
  const library = useLibrary();
  const books = useBooks();
  await conn.connect(address, token);
  history.replaceState(null, '', `#api=${encodeURIComponent(address)}&token=${token}`);
  // 初始数据并行拉取（失败不阻断界面，列表自身会呈现错误态）
  await Promise.allSettled([library.refreshAll(), books.load()]);
  subscribeServerEvents();
}

/** server 停止/重启：清理连接与订阅（主界面 API 已失效） */
export function onServerStopped(): void {
  unsubscribeSse?.();
  unsubscribeSse = null;
  useConnection().disconnect();
}

/** SSE：全部事件收敛为「定向补丁 + 失效刷新」 */
function subscribeServerEvents(): void {
  const conn = useConnection();
  const library = useLibrary();
  const books = useBooks();
  const ui = useUi();
  unsubscribeSse?.();
  unsubscribeSse = subscribeEvents(
    directUrl('/events'),
    {
      'item.added': (p) => {
        if (!books.filter.inTrash) {
          books.load().catch(() => {});
          library.refreshDimensions().catch(() => {});
        }
        void p;
      },
      'items.added': () => {
        if (!books.filter.inTrash) {
          books.load().catch(() => {});
        }
      },
      'item.updated': (p) => {
        books.patchItem(p as Item);
      },
      'items.updated': (items) => {
        for (const item of (items as Item[]) ?? []) {
          books.patchItem(item);
        }
      },
      'item.embed_failed': (p) => {
        // EPUB/PDF 元数据自动回写文件失败：逐个失败路径 toast（库内元数据已保存成功，文件未改动，无需刷新列表）
        const { failures } = p as { failures: { path: string; error: string }[] };
        for (const f of failures ?? []) {
          ui.toastError(`元数据写入文件失败（${f.path}）：${f.error}`);
        }
      },
      'item.trashed': (p) => {
        books.dropItem((p as { id: string }).id);
        // 回收站态下新移入的条目需刷新列表（与 item.restored 对称）
        if (books.filter.inTrash) {
          books.load().catch(() => {});
        }
        library.refreshAll().catch(() => {});
      },
      'item.restored': () => {
        if (books.filter.inTrash) {
          books.load().catch(() => {});
        }
        library.refreshAll().catch(() => {});
      },
      'item.removed': (p) => {
        books.dropItem((p as { id: string }).id);
        library.refreshDimensions().catch(() => {});
      },
      'folder.changed': () => {
        library.refreshTree().catch(() => {});
        books.load().catch(() => {});
      },
      'library.updated': () => {
        library.refreshMeta().catch(() => {});
      },
      'locks.changed': () => {
        library.refreshMeta().catch(() => {});
      },
      'global_filter.changed': () => {
        void conn;
      },
    },
    // lagged 断开：重连后全量对齐
    () => {
      books.load().catch(() => {});
      library.refreshAll().catch(() => {});
      void ui;
    },
  );
}

/** 冷启动入口：hash 带连接参数直连，否则等 shell 事件（换库/首次选择） */
export function boot(): void {
  const conn = parseHashConn();
  if (conn) {
    void onServerReady(conn.address, conn.token);
  }
  const s = shell();
  if (s) {
    // 页面（重）载晚于 server 就绪的竞态兜底
    void s.getServerConn().then((started) => {
      if (started && !useConnection().ready) {
        void onServerReady(started.address, started.token);
      }
    });
    s.onServerStarted((started) => {
      void onServerReady(started.address, started.token);
    });
    s.onServerRestarting(() => {
      onServerStopped();
    });
  }
}
