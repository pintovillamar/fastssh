import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

export default defineConfig({
  plugins: [svelte()],
  server: {
    // `npm run dev` gives hot reload; terminal sessions still go to the Rust server.
    proxy: {
      '/ws': { target: 'ws://127.0.0.1:7422', ws: true },
    },
  },
})
