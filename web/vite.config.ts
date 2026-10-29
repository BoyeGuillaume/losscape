import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// The Rust server serves the built app in production; in dev, Vite proxies API calls to it.
const server = process.env.LOSSCAPE_SERVER ?? 'http://localhost:8080'

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/api': server,
      '/healthz': server,
    },
  },
})
