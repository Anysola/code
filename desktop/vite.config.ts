import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"

// Tauri serves dist/; `tauri dev` loads the dev server on this port.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { port: 5620, strictPort: true },
  build: { target: "es2022" },
})
