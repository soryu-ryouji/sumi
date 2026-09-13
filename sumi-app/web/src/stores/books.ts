// 条目列表查询态：过滤条件（与 ListQuery 契约字段一一对应）+ 分页 + SSE 增量刷新。
import { defineStore } from 'pinia';
import { api } from '@/shared/api/client';
import type { Item, ListQuery } from '@/shared/api/types';

export interface FilterState {
  keywords: string[];
  content: string;
  tags: string[];
  categories: string[];
  authors: string[];
  series: string | null;
  folder: string | null;
  foldersExact: boolean;
  inTrash: boolean;
  readStatus: string | null;
  star: number | null;
  orderBy: string;
  order: 'asc' | 'desc';
}

export const ORDER_LABELS: Record<string, string> = {
  added_time: '入库时间',
  modification_time: '修改时间',
  title: '书名',
  author: '作者',
  size: '大小',
  star: '评分',
  progress: '进度',
  last_read_time: '最近阅读',
  pubdate: '出版日期',
};

const PAGE_SIZE = 120;

export const useBooks = defineStore('books', {
  state: () => ({
    filter: {
      keywords: [],
      content: '',
      tags: [],
      categories: [],
      authors: [],
      series: null,
      folder: null,
      foldersExact: false,
      inTrash: false,
      readStatus: null,
      star: null,
      orderBy: 'added_time',
      order: 'desc',
    } as FilterState,
    items: [] as Item[],
    total: 0,
    totalSize: 0,
    hasMore: false,
    loading: false,
    error: '' as string | unknown,
  }),
  actions: {
    toQuery(offset: number, limit: number): ListQuery {
      const f = this.filter;
      return {
        keywords: f.keywords,
        content: f.content || undefined,
        tags: f.tags,
        categories: f.categories,
        authors: f.authors,
        series: f.series ?? undefined,
        folders: f.folder !== null ? [f.folder] : [],
        folders_exact: f.folder !== null ? f.foldersExact : undefined,
        in_trash: f.inTrash,
        read_status: f.readStatus ?? undefined,
        star: f.star ?? undefined,
        order_by: f.orderBy,
        order: f.order,
        offset,
        limit,
      };
    },
    /** 重置分页并加载（过滤/排序变化、SSE 刷新入口） */
    async load() {
      this.loading = true;
      this.error = '';
      try {
        const data = await api<{ items: Item[]; total: number; total_size: number }>('/item/list', {
          method: 'POST',
          body: this.toQuery(0, PAGE_SIZE),
        });
        this.items = data.items;
        this.total = data.total;
        this.totalSize = data.total_size;
        this.hasMore = data.items.length < data.total;
      } catch (e) {
        this.error = e;
        this.items = [];
        this.total = 0;
        throw e;
      } finally {
        this.loading = false;
      }
    },
    /** 追加下一页（滚动到底） */
    async loadMore() {
      if (!this.hasMore || this.loading) {
        return;
      }
      this.loading = true;
      try {
        const data = await api<{ items: Item[]; total: number }>('/item/list', {
          method: 'POST',
          body: this.toQuery(this.items.length, PAGE_SIZE),
        });
        // SSE 刷新与加载更多交错时按 id 去重
        const known = new Set(this.items.map((i) => i.id));
        this.items.push(...data.items.filter((i) => !known.has(i.id)));
        this.hasMore = this.items.length < this.total;
      } finally {
        this.loading = false;
      }
    },
    /** SSE item.updated 单条就地替换（避免整列表闪烁） */
    patchItem(item: Item) {
      const idx = this.items.findIndex((i) => i.id === item.id);
      if (idx >= 0) {
        this.items[idx] = item;
      }
    },
    dropItem(id: string) {
      this.items = this.items.filter((i) => i.id !== id);
      this.total = Math.max(0, this.total - 1);
    },
  },
});
