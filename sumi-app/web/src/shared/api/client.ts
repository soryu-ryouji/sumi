// REST 客户端：信封解包、token/解锁票据注入、直链 URL 拼装。
// 连接参数（base/token）由 connection store 在 server 就绪时注入。
export class ApiError extends Error {
  constructor(
    public code: string,
    message: string,
    public status: number,
  ) {
    super(message);
  }
}

let base = '';
let token = '';
let unlockTickets: string[] = [];

export function configureApi(nextBase: string, nextToken: string): void {
  base = nextBase;
  token = nextToken;
}

export function setUnlockTickets(tickets: string[]): void {
  unlockTickets = tickets;
}

/** 直链端点（cover/file/toc/content/resource、SSE）：EventSource/<img> 无法设请求头，token 走查询参数 */
export function directUrl(path: string, extra: Record<string, string> = {}): string {
  const params = new URLSearchParams({ token, ...extra });
  return `${base}/api/v1${path}?${params.toString()}`;
}

export interface ApiOptions {
  method?: string;
  body?: unknown;
}

/** 统一请求：成功信封解 data，错误信封抛 ApiError（code 即 REST 契约错误码） */
export async function api<T>(path: string, opts: ApiOptions = {}): Promise<T> {
  const headers: Record<string, string> = { authorization: `Bearer ${token}` };
  if (unlockTickets.length > 0) {
    headers['x-sumi-unlock'] = unlockTickets.join(',');
  }
  if (opts.body !== undefined) {
    headers['content-type'] = 'application/json';
  }
  let res: Response;
  try {
    res = await fetch(`${base}/api/v1${path}`, {
      method: opts.method ?? 'GET',
      headers,
      body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
    });
  } catch (e) {
    throw new ApiError('NETWORK', `无法连接 sumi-daemon: ${e instanceof Error ? e.message : e}`, 0);
  }
  const json = (await res.json().catch(() => null)) as { status?: string; data?: unknown; error?: { code: string; message: string } } | null;
  if (!res.ok || json?.status === 'error') {
    throw new ApiError(json?.error?.code ?? 'INTERNAL', json?.error?.message ?? `HTTP ${res.status}`, res.status);
  }
  return (json?.data ?? null) as T;
}
