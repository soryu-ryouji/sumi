// API 类型别名：schema.d.ts 是 openapi-typescript 从后端固化 schema 生成的契约类型，
// 前端不允许手写对接口（改接口先改后端 → dump openapi.json → npm run gen:types）。
import type { components } from './schema';

export type Item = components['schemas']['ItemDto'];
export type ListQuery = components['schemas']['ListQuery'];
export type UpdateQuery = components['schemas']['UpdateQuery'];
export type AppInfo = components['schemas']['AppInfo'];
export type LanConfig = components['schemas']['LanConfig'];
export type StartupPayload = components['schemas']['StartupPayload'];

/** 文件夹树节点（GET folder/list 响应） */
export interface FolderNode {
  path: string;
  name: string;
  children: FolderNode[];
  modification_time: number;
}

/** 名称维度聚合条目（category/tag/author/series list 响应） */
export interface CountEntry {
  name: string;
  count: number;
}

/** 阅读目录节点（GET item/toc 响应） */
export interface TocEntry {
  title: string;
  anchor: string;
  children: TocEntry[];
}

/** 锁集合快照（lock/list 与 global_filter/list 响应同构） */
export interface DimensionNames {
  folders: string[];
  categories: string[];
  tags: string[];
  authors: string[];
}

/** SSE 事件名（与 daemon core/events 常量一致，构成持久契约） */
export const SSE_EVENTS = [
  'item.added',
  'items.added',
  'item.updated',
  'items.updated',
  'item.trashed',
  'item.restored',
  'item.removed',
  'folder.changed',
  'library.updated',
  'global_filter.changed',
  'locks.changed',
  'task.progress',
] as const;
export type SseEventName = (typeof SSE_EVENTS)[number];

export { type SumiShell, type ServerConn, type ServerProgress, type LibraryList, type LibraryHistoryItem, type UpdateChannel, type UpdateInfo, type UpdateProgress, UPDATE_CANCELLED } from '../../../../electron/src/ipc-contract';
