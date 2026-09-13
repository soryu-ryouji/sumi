import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import { fileURLToPath, URL } from 'node:url';

export default defineConfig({
  root: 'web',
  // 公共目录直指 build/（electron-builder 资源目录），icon 等资产保持单一来源
  publicDir: '../build',
  plugins: [vue()],
  // `@` 别名指向 web/src
  resolve: { alias: { '@': fileURLToPath(new URL('./web/src', import.meta.url)) } },
  // file:// 打包加载需要相对路径（未来 daemon LAN 托管同一产物时 http 下同样可用）
  base: './',
  server: { port: 5173, strictPort: true },
  build: { outDir: 'dist', emptyOutDir: true },
});
