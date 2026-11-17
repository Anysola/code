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

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>
)
