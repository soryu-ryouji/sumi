// SSE 订阅：书库变更推送（事件名与负载即持久契约）。
// 订阅端积压被服务端断开（lagged）时自动重连并回调 onLagged（客户端全量对齐）。

export interface SseHandlers {
  [event: string]: (payload: unknown) => void;
}

export function subscribeEvents(url: string, handlers: SseHandlers, onLagged?: () => void): () => void {
  let es: EventSource | null = new EventSource(url);
  let closed = false;
  let retryTimer: ReturnType<typeof setTimeout> | undefined;
  // lagged/关闭后服务端断流：EventSource 会自动重连，但 lagged 语义要求先全量对齐——
  // 断开即回调，由调用方决定重连节奏（这里 1s 后重建连接）
  es.onerror = () => {
    if (closed) {
      return;
    }
    es?.close();
    es = null;
    onLagged?.();
    retryTimer = setTimeout(() => {
      if (!closed) {
        es = wire(new EventSource(url));
      }
    }, 1000);
  };
  function wire(source: EventSource): EventSource {
    for (const name of Object.keys(handlers)) {
      source.addEventListener(name, (ev) => {
        const payload = JSON.parse((ev as MessageEvent).data ?? 'null');
        handlers[name]?.(payload);
      });
    }
    source.onerror = () => {
      if (closed) {
        return;
      }
      source.close();
      es = null;
      onLagged?.();
      retryTimer = setTimeout(() => {
        if (!closed) {
          es = wire(new EventSource(url));
        }
      }, 1000);
    };
    return source;
  }
  es = wire(es);
  return () => {
    closed = true;
    clearTimeout(retryTimer);
    es?.close();
  };
}
