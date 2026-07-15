import { defineConfig } from 'vite';
import { resolve } from 'node:path';

export default defineConfig({
    build: {
        lib: {
            entry: resolve(__dirname, 'resources/js/index.ts'),
            name: 'G7MediaBoosterG5',
            formats: ['iife'],
            fileName: () => 'uploader.iife.js',
        },
        outDir: resolve(__dirname, 'plugin/g7mediabooster/assets'),
        emptyOutDir: true,
        sourcemap: true,
        target: 'es2020',
    },
});
