// 展示格式化：字节、时间、进度。
export function formatBytes(n: number): string {
  if (n < 1024) {
    return `${n} B`;
  }
  if (n < 1024 * 1024) {
    return `${(n / 1024).toFixed(1)} KB`;
  }
  if (n < 1024 * 1024 * 1024) {
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  }
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

export function formatTime(unixMs: number): string {
  if (!unixMs) {
    return '—';
  }
  const d = new Date(unixMs);
  const y = d.getFullYear();
  const now = new Date();
  const sameYear = y === now.getFullYear();
  const md = `${d.getMonth() + 1}月${d.getDate()}日`;
  const hm = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  return sameYear ? `${md} ${hm}` : `${y}年${md}`;
}

export const READ_STATUS_LABEL: Record<string, string> = {
  unread: '未读',
  reading: '在读',
  finished: '读完',
  abandoned: '弃读',
};
