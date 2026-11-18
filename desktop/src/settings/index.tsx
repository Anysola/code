/**
 * Settings: a full-window view laid over the app (which stays as it was underneath), a nav of
 * sections on the left in the sidebar's place and the section in the main frame.
 */
import { ArrowLeft } from "lucide-react"
import { useEffect, useRef, type ComponentType } from "react"

import { reloadPrefs } from "../prefs"
import { closeSettings, openSettings, useStore, type SettingsSection } from "../store"
import { btn, cn, Kbd, TitleBar, useEscape } from "../ui"
import { AccountSection, BillingSection, UsageSection } from "./account"
import { AdvancedSection, CommandsSection, InstructionsSection, PermissionsSection, SkillsSection } from "./agent"
import { AppearanceSection, ConversationSection, GeneralSection, KeyboardSection, NotificationsSection } from "./app"
import { McpSection } from "./mcp"
import { AboutSection, DataSection, DiscordSection } from "./system"

const NAV: { group: string; items: { id: SettingsSection; label: string; view: ComponentType }[] }[] = [
  {
    group: "Account",
    items: [
      { id: "account", label: "Account", view: AccountSection },
      { id: "usage", label: "Usage", view: UsageSection },
      { id: "billing", label: "Billing", view: BillingSection },
    ],
  },
  {
    group: "App",
    items: [
      { id: "general", label: "General", view: GeneralSection },
      { id: "appearance", label: "Appearance", view: AppearanceSection },
      { id: "conversation", label: "Conversation", view: ConversationSection },
      { id: "notifications", label: "Notifications", view: NotificationsSection },
      { id: "keyboard", label: "Keyboard", view: KeyboardSection },
    ],
  },
  {
    group: "Agent",
    items: [
      { id: "permissions", label: "Permissions", view: PermissionsSection },
      { id: "mcp", label: "MCP servers", view: McpSection },
      { id: "skills", label: "Skills", view: SkillsSection },
      { id: "commands", label: "Commands", view: CommandsSection },
      { id: "instructions", label: "Instructions", view: InstructionsSection },
      { id: "advanced", label: "Advanced", view: AdvancedSection },
    ],
  },
  { group: "Connections", items: [{ id: "discord", label: "Discord", view: DiscordSection }] },
  {
    group: "System",
    items: [
      { id: "data", label: "Data", view: DataSection },
      { id: "about", label: "About", view: AboutSection },
    ],
  },
]

const ALL = NAV.flatMap((g) => g.items)

export function Settings({ section }: { section: SettingsSection }) {
  const scroller = useRef<HTMLDivElement>(null)
  const item = ALL.find((i) => i.id === section) ?? ALL[0]
  const View = item.view
  useEffect(() => {
    void reloadPrefs()
  }, [])
  useEffect(() => {
    scroller.current?.scrollTo({ top: 0 })
  }, [section])
  // Escape goes back, unless a menu, a confirm or a field took it first: in a text field the
  // first Escape leaves the field.
  const root = useRef<HTMLDivElement>(null)
  useEscape(root, (e) => {
    const target = e.target as HTMLElement | null
    if (target?.matches("input, textarea, select") && root.current?.contains(target)) target.blur()
    else closeSettings()
  })
  return (
    <div ref={root} data-settings className="absolute inset-0 z-20 flex bg-background">
      <aside className="flex w-64 shrink-0 flex-col">
        <TitleBar>
          <div className="flex items-center px-2">
            <button onClick={closeSettings} className={btn("ghost", "sm", "gap-1.5 text-muted")}>
              <ArrowLeft className="size-3.5" />
              Back
              <Kbd>Esc</Kbd>
            </button>
          </div>
        </TitleBar>
        <nav className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-2 pt-3 pb-3" aria-label="Settings">
          {NAV.map((g) => (
            <div key={g.group}>
              <div className="text-2xs flex h-7 items-center px-2 font-medium tracking-wide text-muted">{g.group}</div>
              <ul className="flex flex-col">
                {g.items.map((i) => (
                  <li key={i.id}>
                    <button
                      onClick={() => openSettings(i.id)}
                      aria-current={i.id === item.id ? "page" : undefined}
                      className={cn(
                        "flex h-8 w-full items-center rounded-lg px-2 text-left text-sm transition-colors focus-visible:status-focused-field",
                        i.id === item.id ? "bg-default" : "text-foreground/85 hover:bg-default/60"
                      )}
                    >
                      {i.label}
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </nav>
      </aside>
      <main className="relative m-2 ml-0 flex min-w-0 flex-1 flex-col overflow-hidden rounded-2xl bg-surface/40">
        <TitleBar controls />
        <div ref={scroller} className="min-h-0 flex-1 overflow-y-auto">
          <View key={item.id} />
        </div>
      </main>
    </div>
  )
}

/** The section the nav shows, for others that link here. */
export function sectionLabel(id: SettingsSection) {
  return ALL.find((i) => i.id === id)?.label ?? "Settings"
}
