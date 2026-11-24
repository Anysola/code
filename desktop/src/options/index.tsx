/**
 * Where the option sets plug into the app. Every slot renders nothing without `?opt=`.
 */
import { Wallet } from "lucide-react"

import type { SessionRow } from "../bridge"
import type { Open } from "../store"
import { MenuItem } from "../ui"
import * as A from "./a"
import * as B from "./b"
import * as C from "./c"
import { editBudget, OPT, openPalette } from "./shared"

export { OPT, openPalette }
export { forkTitle } from "./shared"

export function OptPalette() {
  if (OPT === "a") return <A.Palette />
  if (OPT === "b") return <B.Palette />
  if (OPT === "c") return <C.Palette />
  return null
}

export function OptHeader({ open }: { open: Open }) {
  return OPT === "a" ? <A.HeaderBudget open={open} /> : null
}

export function OptHeaderNote({ open }: { open: Open }) {
  return OPT === "c" ? <C.HeaderNote open={open} /> : null
}

export function OptFooter({ open }: { open: Open }) {
  return OPT === "b" ? <B.FooterBudget open={open} /> : null
}

export function OptBanner({ open }: { open: Open }) {
  if (OPT === "a") return <A.Banner open={open} />
  if (OPT === "b") return <B.Banner open={open} />
  if (OPT === "c") return <C.Banner open={open} />
  return null
}

export function OptTranscriptTop({ open }: { open: Open }) {
  return OPT === "b" ? <B.TranscriptTop open={open} /> : null
}

export function OptUserAction({ id }: { id: string }) {
  if (OPT === "a") return <A.UserAction id={id} />
  if (OPT === "b") return <B.UserAction id={id} />
  return null
}

/** "Set budget…" in the header's session menu. */
export function OptMenuItem({ close }: { close: () => void }) {
  if (!OPT) return null
  return (
    <MenuItem
      onSelect={() => {
        close()
        if (OPT === "c") openPalette("budget")
        else setTimeout(() => editBudget(), 0)
      }}
    >
      <Wallet className="size-3.5 text-muted" /> Set budget…
    </MenuItem>
  )
}

/** The sidebar's search row; null keeps the app's own. */
export function OptSearchRow() {
  if (OPT === "a") return <A.SearchRow />
  if (OPT === "c") return <C.SearchRow />
  return null
}

/** A session row's title; null keeps the app's own. */
export function OptRowTitle({ row, active }: { row: SessionRow; active: boolean }) {
  if (OPT === "a") return <A.RowTitle title={row.title} active={active} />
  if (OPT === "b") return <B.RowTitle title={row.title} active={active} />
  if (OPT === "c") return <C.RowTitle title={row.title} active={active} id={row.id} />
  return null
}

export function optNest(rows: SessionRow[]): (SessionRow & { nested?: boolean })[] {
  return OPT === "c" ? C.nest(rows) : rows
}
