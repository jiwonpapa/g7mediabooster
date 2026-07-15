import { defineConfig } from 'vite';
import path from 'node:path';

export default defineConfig({
    build: {
        lib: {
            entry: path.resolve(__dirname, 'resources/js/index.ts'),
            name: 'JiwonpapaG7MediaBooster',
            fileName: 'module',
            formats: ['iife'],
        },
        outDir: 'dist',
        emptyOutDir: true,
        sourcemap: true,
        minify: 'esbuild',
        target: 'es2022',
        rollupOptions: {
            output: {
                entryFileNames: 'js/module.iife.js',
                chunkFileNames: 'js/[name]-[hash].js',
                assetFileNames: 'assets/[name][extname]',
            },
        },
    },
});
