import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { resolve } from 'node:path'

// The console reaches its back ends through same-origin proxies, so the
// browser never needs CORS and RPC settings stay on the servers:
//   /veto → Veto demo server (verdicts, review queue)
//   /lab  → this repo's API (src/, port 8070)
//   /svm  → aval-svm (svm/, port 8899)
const VETO = process.env.VETO_URL ?? 'http://127.0.0.1:5173'
const LAB = process.env.LAB_URL ?? 'http://127.0.0.1:8070'
const SVM = process.env.SVM_URL ?? 'http://127.0.0.1:8899'
const proxy = {
  '/veto': { target: VETO, changeOrigin: true, rewrite: (p: string) => p.replace(/^\/veto/, '') },
  '/lab': { target: LAB, changeOrigin: true, rewrite: (p: string) => p.replace(/^\/lab/, '') },
  '/svm': { target: SVM, changeOrigin: true, rewrite: (p: string) => p.replace(/^\/svm/, '') },
}

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { '@': resolve(import.meta.dirname, 'src') } },
  server: { host: '127.0.0.1', port: 5190, proxy },
  preview: { host: '127.0.0.1', port: 4190, proxy },
})
