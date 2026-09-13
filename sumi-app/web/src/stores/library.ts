// 书库聚合状态：文件夹树 + 分类/标签/作者/系列计数 + 锁集合（侧栏数据源）。
// SSE（item.* / folder.changed / locks.changed）驱动刷新。
import { defineStore } from 'pinia';
import { api } from '@/shared/api/client';
import type { CountEntry, DimensionNames, FolderNode } from '@/shared/api/types';

export const useLibrary = defineStore('library', {
  state: () => ({
    tree: [] as FolderNode[],
    categories: [] as CountEntry[],
    tags: [] as CountEntry[],
    authors: [] as CountEntry[],
    series: [] as CountEntry[],
    locks: null as DimensionNames | null,
    libraryName: '',
    loaded: false,
  }),
  actions: {
    async refreshAll() {
      await Promise.all([this.refreshTree(), this.refreshDimensions(), this.refreshMeta()]);
      this.loaded = true;
    },
    async refreshTree() {
      this.tree = await api<FolderNode[]>('/folder/list');
    },
    async refreshDimensions() {
      const [cats, tags, authors, series] = await Promise.all([
        api<CountEntry[]>('/category/list'),
        api<CountEntry[]>('/tag/list'),
        api<CountEntry[]>('/author/list'),
        api<CountEntry[]>('/series/list'),
      ]);
      this.categories = cats;
      this.tags = tags;
      this.authors = authors;
      this.series = series;
    },
    async refreshMeta() {
      const [locks, info] = await Promise.all([
        api<DimensionNames>('/lock/list'),
        api<{ name: string }>('/library/info'),
      ]);
      this.locks = locks;
      this.libraryName = info.name;
    },
  },
});
