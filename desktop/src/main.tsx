import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import { App } from "./app"
import "./styles.css"

// A file dropped anywhere but the composer would otherwise open in the webview in place of
// the app. Text drags into fields keep their default.
const files = (e: DragEvent) => e.dataTransfer?.types.includes("Files") ?? false
document.addEventListener("dragover", (e) => {
  if (!files(e)) return
  // The composer has taken it already; elsewhere the cursor says no drop.
  if (!e.defaultPrevented && e.dataTransfer) e.dataTransfer.dropEffect = "none"
  e.preventDefault()
})
document.addEventListener("drop", (e) => {
  if (files(e)) e.preventDefault()
})

// WebView2's browser keys and menu: a reload drops waiting asks while the turn keeps waiting
// on them, and print has no place here. Development keeps them for reloading.
if (import.meta.env.PROD) {
  document.addEventListener(
    "keydown",
    (e) => {
      const key = e.key.toLowerCase()
      const ctrl = e.ctrlKey || e.metaKey
      if (key === "f5" || (ctrl && (key === "r" || key === "p"))) e.preventDefault()
    },
    true
  )
  document.addEventListener("contextmenu", (e) => {
    const target = e.target as HTMLElement | null
    const editable = target?.closest("input, textarea, [contenteditable]:not([contenteditable='false'])")
    // Copy and paste stay where there is something to copy or a field to paste into.
    if (editable || document.getSelection()?.toString()) return
    e.preventDefault()
  })
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>
)
