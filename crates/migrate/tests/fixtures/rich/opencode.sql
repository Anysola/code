PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE `account_state` (
          `id` integer PRIMARY KEY,
          `active_account_id` text,
          `active_org_id` text,
          CONSTRAINT `fk_account_state_active_account_id_account_id_fk` FOREIGN KEY (`active_account_id`) REFERENCES `account`(`id`) ON DELETE SET NULL
        );
CREATE TABLE `account` (
          `id` text PRIMARY KEY,
          `email` text NOT NULL,
          `url` text NOT NULL,
          `access_token` text NOT NULL,
          `refresh_token` text NOT NULL,
          `token_expiry` integer,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL
        );
CREATE TABLE `control_account` (
          `email` text NOT NULL,
          `url` text NOT NULL,
          `access_token` text NOT NULL,
          `refresh_token` text NOT NULL,
          `token_expiry` integer,
          `active` integer NOT NULL,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          CONSTRAINT `control_account_pk` PRIMARY KEY(`email`, `url`)
        );
CREATE TABLE `credential` (
          `id` text PRIMARY KEY,
          `integration_id` text,
          `label` text NOT NULL,
          `value` text NOT NULL,
          `connector_id` text,
          `method_id` text,
          `active` integer,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL
        );
INSERT INTO credential VALUES('cred_1a828ea97001kvNOl6mJY5MmS9','anysola','Anysola account','{"type":"oauth","methodID":"device","refresh":"","access":"asc_FAKEtoken0000000000","expires":1825286400000,"metadata":{"apiURL":"http://127.0.0.1:18841","device":"agim-206806"}}',NULL,NULL,1,1793822616223,1793822616223);
CREATE TABLE `event_sequence` (
          `aggregate_id` text PRIMARY KEY,
          `seq` integer NOT NULL,
          `owner_id` text
        );
INSERT INTO event_sequence VALUES('7f21fb1c4ceed8c54dd0f3df103168b3211dc272',0,NULL);
INSERT INTO event_sequence VALUES('ses_e57d70d7fffe0TqPcsNMhaeh6R',71,NULL);
INSERT INTO event_sequence VALUES('ses_e57d7012cffe72xpyfIsYUr33S',10,NULL);
INSERT INTO event_sequence VALUES('ses_e57d6f7a8ffen9abAz9CaEqwMp',83,NULL);
INSERT INTO event_sequence VALUES('ses_e57d6e6feffe4gefNuJdeuiqXd',24,NULL);
CREATE TABLE `event` (
          `id` text PRIMARY KEY,
          `aggregate_id` text NOT NULL,
          `seq` integer NOT NULL,
          `created` integer DEFAULT 0 NOT NULL,
          `type` text NOT NULL,
          `data` text NOT NULL,
          CONSTRAINT `fk_event_aggregate_id_event_sequence_aggregate_id_fk` FOREIGN KEY (`aggregate_id`) REFERENCES `event_sequence`(`aggregate_id`) ON DELETE CASCADE
        );
CREATE TABLE `job` (
          `notification_id` text PRIMARY KEY,
          `id` text NOT NULL,
          `session_id` text NOT NULL,
          `child_session_id` text,
          `kind` text NOT NULL,
          `title` text,
          `status` text NOT NULL,
          `output` text,
          `error` text,
          `recovery` text NOT NULL,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL
        );
CREATE TABLE `kv` (
          `key` text PRIMARY KEY,
          `value` text NOT NULL,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL
        );
INSERT INTO kv VALUES('anysola:config','{"model":"anysola/claude-sonnet-5-5","small_model":"anysola/claude-haiku-5-5","agents":{"title":{"model":"anysola/claude-haiku-5-5"}},"providers":{"anysola":{"name":"Anysola","package":"@opencode/ai/providers/anthropic-compatible","settings":{"baseURL":"http://127.0.0.1:18842/v1/code","apiKey":"{env:ANYSOLA_TOKEN}"},"headers":{"x-anysola-client":"opencode"},"models":{"auto":{"name":"Auto","description":"Picks Haiku or Sonnet per turn","limit":{"context":1000000,"output":32000},"capabilities":{"reasoning":{"supported":true,"effort":["low","medium","high","xhigh","max"]}},"compatibility":{"supportsThinkingBlockBinding":true,"supportsEffortUpdates":true},"variants":[{"id":"economy","settings":{"thinking":{"type":"adaptive","display":"summarized"},"effort":"low"},"headers":{"x-anysola-variant":"economy"}}]},"claude-sonnet-5-5":{"name":"Claude Sonnet 5.5","description":"Default","limit":{"context":1000000,"output":32000},"capabilities":{"reasoning":{"supported":true,"effort":["low","medium","high","xhigh","max"]}},"variants":[{"id":"economy","settings":{"thinking":{"type":"adaptive","display":"summarized"},"effort":"low"},"headers":{"x-anysola-variant":"economy"}}],"credits_per_turn_estimate":6},"claude-haiku-5-5":{"name":"Claude Haiku 5.5","description":"Fastest","limit":{"context":200000,"output":32000},"credits_per_turn_estimate":1}}}}}',1793822616283,1793822633283);
INSERT INTO kv VALUES('anysola:session:ses_e57d70d7fffe0TqPcsNMhaeh6R','{"total":16,"turns":1,"budget":null}',1793822619783,1793822621941);
INSERT INTO kv VALUES('anysola:session:ses_e57d7012cffe72xpyfIsYUr33S','{"total":2,"turns":1,"budget":null}',1793822621610,1793822621746);
INSERT INTO kv VALUES('anysola:session:ses_e57d6f7a8ffen9abAz9CaEqwMp','{"total":2,"turns":1,"budget":null}',1793822625674,1793822625931);
INSERT INTO kv VALUES('anysola:session:ses_e57d6e6feffe4gefNuJdeuiqXd','{"total":2,"turns":2,"budget":null}',1793822629599,1793822634639);
INSERT INTO kv VALUES('anysola:budget:default','25',1793787700000,1793787700000);
INSERT INTO kv VALUES('anysola:session:ses_orphan','{"total":3,"turns":1,"budget":null}',1793787700000,1793787700000);
CREATE TABLE `permission` (
          `id` text PRIMARY KEY,
          `project_id` text NOT NULL,
          `action` text NOT NULL,
          `resource` text NOT NULL,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          CONSTRAINT `fk_permission_project_id_project_id_fk` FOREIGN KEY (`project_id`) REFERENCES `project`(`id`) ON DELETE CASCADE
        );
INSERT INTO permission VALUES('psv_fixture','7f21fb1c4ceed8c54dd0f3df103168b3211dc272','shell','git status *',1793787700000,1793787700000);
CREATE TABLE `project_directory` (
          `project_id` text NOT NULL,
          `directory` text NOT NULL,
          `type` text,
          `strategy` text,
          `time_created` integer NOT NULL,
          CONSTRAINT `project_directory_pk` PRIMARY KEY(`project_id`, `directory`),
          CONSTRAINT `fk_project_directory_project_id_project_id_fk` FOREIGN KEY (`project_id`) REFERENCES `project`(`id`) ON DELETE CASCADE
        );
CREATE TABLE `project` (
          `id` text PRIMARY KEY,
          `worktree` text NOT NULL,
          `vcs` text,
          `name` text,
          `icon_url` text,
          `icon_url_override` text,
          `icon_color` text,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          `time_initialized` integer,
          `time_active` integer DEFAULT 0 NOT NULL,
          `time_archived` integer,
          `sandboxes` text NOT NULL,
          `commands` text
        );
INSERT INTO project VALUES('7f21fb1c4ceed8c54dd0f3df103168b3211dc272','@HOME@/proj','git',NULL,NULL,NULL,NULL,1793822613470,1793822613470,NULL,1793822632998,NULL,'[]',NULL);
CREATE TABLE `schedule` (
          `id` text PRIMARY KEY,
          `session_id` text,
          `name` text,
          `action` text NOT NULL,
          `check` text,
          `when` text NOT NULL,
          `tz` text NOT NULL,
          `status` text DEFAULT 'active' NOT NULL,
          `next_due_at` integer,
          `ends_at` integer,
          `remaining_runs` integer,
          `run_count` integer DEFAULT 0 NOT NULL,
          `checks` integer DEFAULT 0 NOT NULL,
          `failures` integer DEFAULT 0 NOT NULL,
          `last_fired_at` integer,
          `last_message_id` text,
          `last_outcome` text,
          `retry_at` integer,
          `state` text,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          CONSTRAINT `fk_schedule_session_id_session_v2_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session_v2`(`id`) ON DELETE CASCADE
        );
CREATE TABLE `blob_ref` (
          `session_id` text NOT NULL,
          `message_id` text NOT NULL,
          `hash` text NOT NULL,
          `seq` integer NOT NULL,
          CONSTRAINT `blob_ref_pk` PRIMARY KEY(`session_id`, `message_id`, `hash`),
          CONSTRAINT `fk_blob_ref_session_id_session_v2_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session_v2`(`id`) ON DELETE CASCADE,
          CONSTRAINT `fk_blob_ref_hash_blob_hash_fk` FOREIGN KEY (`hash`) REFERENCES `blob`(`hash`) ON DELETE CASCADE
        );
CREATE TABLE `blob` (
          `hash` text PRIMARY KEY,
          `mime` text NOT NULL,
          `bytes` integer NOT NULL,
          `width` integer,
          `height` integer,
          `data` blob NOT NULL,
          `time_created` integer NOT NULL
        );
CREATE TABLE `instruction_blob` (
          `hash` text PRIMARY KEY,
          `value` text
        );
INSERT INTO instruction_blob VALUES('1f1291478accf8b3e4d96a40c150bf844d60557dede4aae56c47ea0043bbdce2','{"total":8,"shown":8,"namespaces":[{"name":"opencode","description":"Tools for managing OpenCode itself, such as working with sessions and worktrees, searching the available models, and reading MCP resources.","count":8,"entries":[{"path":"opencode.list_mcp_resources","line":"  - tools.opencode.list_mcp_resources({\n  /** MCP server name as configured. Omit to list every server. */\n  server?: string,\n}): Promise<{\n  resources: Array<{\n      server: string,\n      name: string,\n      uri: string,\n      description?: string,\n      mimeType?: string,\n    }>,\n  templates: Array<{\n      server: string,\n      name: string,\n      uriTemplate: string,\n      description?: string,\n      mimeType?: string,\n    }>,\n}> // List documents, records, and other data exposed by MCP servers. Use this when the user refers to something that is no..."},{"path":"opencode.models","line":"  - tools.opencode.models({\n  /** Text to search for in model names and IDs. */\n  query?: string,\n  /** Provider ID or name to filter by. Try your own provider first. */\n  provider?: string,\n  /**\n   * Which kind of model to list. Defaults to \"chat\".\n   * \"chat\": a language model (LLM). Powers sessions, agents and subagents, and can call tools.\n   * \"image\": generates or edits images from a prompt. It can''t chat or call tools.\n   */\n  type?: \"chat\" | \"image\",\n  /** Include older versions of each model family. By default only the newest version is listed. */\n  all?: boolean,\n  /** Maximum number of models to return. Defaults to 20. @integer @minimum 1 @maximum 100 */\n  limit?: number,\n  /** Number of models to skip, for paging through results. @integer @minimum 0 */\n  offset?: number,\n}): Promise<{\n  /** Matching models grouped by provider. Your own provider comes first. */\n  providers: Array<{\n      id: string,\n      name: string,\n      /** Newest first. */\n      models: Array<{\n          /** providerID/modelID */\n          id: string,\n          name: string,\n          /** Release date as a Unix timestamp in milliseconds, or 0 when unknown. */\n          released: number,\n          variants: Array<string>,\n          /** Pricing in USD per million tokens. */\n          cost: Array<{\n              tier?: {\n                type: \"context\",\n                /** @integer */\n                size: number,\n              },\n              input: number,\n              output: number,\n              cache: {\n                read: number,\n                write: number,\n              },\n            }>,\n          status: \"alpha\" | \"beta\" | \"deprecated\" | \"active\",\n        }>,\n    }>,\n  /** Number of matching models across all pages. @integer */\n  total: number,\n  /** Offset of the next page, or null on the last page. */\n  next: number | null,\n}> // Search the models available to use. Use this to turn a model name the user mentions into an exact reference for a too..."},{"path":"opencode.read_mcp_resource","line":"  - tools.opencode.read_mcp_resource({\n  /** The server field of the discovered resource. */\n  server: string,\n  /** Exact resource URI from discovery, an expanded uriTemplate, or the user. */\n  uri: string,\n}): Promise<{\n  server: string,\n  uri: string,\n  contents: Array<{\n        type: \"text\",\n        uri: string,\n        text: string,\n        mimeType?: string,\n      } | {\n        type: \"blob\",\n        uri: string,\n        blob: string,\n        mimeType?: string,\n      }>,\n}> // Read one MCP resource by server and URI. Not for local files. Always return the full contents rather than slicing or ..."},{"path":"opencode.session_move","line":"  - tools.opencode.session_move({\n  /** Omit to move the current session. @pattern ^ses */\n  sessionID?: string,\n  /** Destination directory, relative to the target session''s directory or absolute. Supports ~. @minLength 1 */\n  directory: string,\n}): Promise<{\n  /** @pattern ^ses */\n  sessionID: string,\n  directory: string,\n}> // Move a session to another directory, or omit sessionID to move the current session. The current session moves at the ..."},{"path":"opencode.session_rename","line":"  - tools.opencode.session_rename({\n  /** Omit to rename the current session. @pattern ^ses */\n  sessionID?: string,\n  /** New session title. @minLength 1 */\n  title: string,\n}): Promise<{\n  /** @pattern ^ses */\n  sessionID: string,\n  title: string,\n}> // Rename a session, or omit sessionID to rename the current session. Use a short, specific title that summarizes the wo..."},{"path":"opencode.worktree_create","line":"  - tools.opencode.worktree_create({\n  name: string,\n}): Promise<{\n  directory: string,\n}> // Create a named worktree. Returns its directory."},{"path":"opencode.worktree_list","line":"  - tools.opencode.worktree_list(): Promise<{\n  worktrees: Array<{\n      directory: string,\n      strategy?: string,\n    }>,\n}> // List a repository''s worktrees."},{"path":"opencode.worktree_remove","line":"  - tools.opencode.worktree_remove({\n  /** Absolute path of the worktree to remove. @minLength 1 */\n  directory: string,\n  /** Discard modified and untracked files. Only set this when the user has confirmed forced removal. */\n  force?: boolean,\n}): Promise<{\n  directory: string,\n  removed: boolean,\n}> // Remove a worktree. The repository location selects configuration; directory identifies the worktree to remove."}]}]}');
INSERT INTO instruction_blob VALUES('0bcf371d396484ebb990d27221437252e87df91ebb2b32b08f36af7ca80d008f','[{"id":"built-in-browser","name":"built-in-browser","description":"Read this skill before the first step that uses the built-in browser, the browser pane inside the Claude desktop app (also called the in-app browser, the browser pane, Claude''s browser, or \"your own browser\"), whose tools are named mcp__Claude_Browser__* when the session runs in the desktop app and mcp__remote-devices__Claude_Browser__* when a cloud session is linked to the person''s computer; before those tools are turned on there may be a single enable__mcp__remote-devices__Claude_Browser tool instead. It covers the pane''s persistent sign-ins, tabs and preview_start, reading pages as text, site approvals, what the pane cannot open, and what to do when it cannot be reached. It is not for Claude in Chrome (mcp__claude-in-chrome__* tools), which has its own skill, and it does not decide which browser to use."},{"id":"chrome-browser","name":"chrome-browser","description":"Read this skill before the first step that uses Claude in Chrome, the browser extension whose tools are named mcp__claude-in-chrome__* (also called Chrome, the browser extension, or the external browser) and which acts in the person''s real Chrome with their own sign-ins; before those tools are turned on there may be a single enable__mcp__claude-in-chrome tool instead. It covers loading the tools in one ToolSearch call, checking the person''s open tabs and working in a new tab, site permissions, GIF recordings, console logs, dialogs to avoid, and when to stop and ask. It is not for the built-in browser (mcp__Claude_Browser__* or mcp__remote-devices__Claude_Browser__* tools), which has its own skill, and it does not decide which browser to use."},{"id":"computer-use","name":"computer-use","description":"Read this skill before the first step of any request to do something in an app on the person''s own computer (Notes, Finder, System Settings, any desktop app), to look at their screen, or for \"computer use\". Computer use (desktop control) lets Claude take screenshots of the person''s desktop and control it with clicks, typing and scrolling through the Claude desktop app; its tools are named mcp__computer-use__* when the session runs in the desktop app and mcp__remote-devices__computer_* when a cloud session is linked to the person''s computer; before computer use is turned on for a conversation there may be no such tools, only an enable__mcp__remote-devices__computer tool, which turns it on. It covers turning it on, picking the right tool, the access flow, and the safety rules for tiered apps, links and financial actions. It is not for websites, which go through Claude in Chrome or the built-in browser and their own skills."},{"id":"deep-research","name":"deep-research","description":"Use this skill when the user''s prompt requires (1) researching a topic across multiple sources, comparing options or alternatives, analyzing trends or history, understanding markets or industries, or reviewing literature or studies and (2) synthesizing that research into a comprehensive, narrative report. If you''re planning to search the web or internal knowledge bases, consider using this skill. This skill coordinates research subagents, so use it only when you have a tool for spawning subagents (the Agent or Task tool); otherwise, research the question directly."},{"id":"docs","name":"docs","description":"docs (editable docs people share and comment on; the default for any document, named as a doc or not: a document, report, proposal, resume, cover letter, letter, contract, policy, form, template, worksheet, essay, handbook, guide, how-to, cheat sheet, SOP or other writing to keep, share, collaborate on, send, submit, print or sign; a doc exports to Word, PDF, Markdown or Google Docs, so needing a file to send, attach, upload, submit or print is no reason to pick Word, and a file nobody asked for is a doc, not Word; a plan, comparison, summary or notes asked in chat stays in chat; a pasted claude.ai artifact link may be a doc: check with docs tools first; Word or another file format named, tracked changes wanted, or a .docx to change or use as a template → that format''s skill): making one → if no docs-connector instructions are in context, call its `guide` (topic.instructions) first; then create the doc (headings only, no body) before any search, file read or plan, even with files attached."},{"id":"docx","name":"docx","description":"Use this skill whenever the user wants to create, read, edit, or manipulate Word documents (.docx) or Word templates (.dotx). Triggers include: any mention of Microsoft Word Documents, such as ''Word doc'', ''word document'', ''.docx'', ''.dotx'', ''microsoft doc''. Also use when extracting or reorganizing content from .docx or .dotx files, inserting or replacing images in documents, find-and-replace in Word files, working with tracked changes or comments, or converting content into a polished Word document. If the user asks for a deliverable as a Word or .docx file (to download, email or print), use this skill. However, if they ask for a document, page, report, memo, or notes WITHOUT naming a file format and the session offers Claude''s own dedicated document or page skill or connector, use that instead, even if they will email or print it. Do NOT use for PDFs, spreadsheets, Google Docs, or coding unrelated to document generation."},{"id":"google-workspace","name":"google-workspace","description":"Read this before the first Google Drive, Docs, Sheets or Slides connector call whenever the task creates or changes a Google file. Use this skill whenever the user wants to create or change a Google Doc, Sheet or Slides file in their Google Drive. Triggers include: a request that names Google Docs, Sheets, Slides or Drive and asks to make, edit, format, copy or rename a file; a docs.google.com link with a request to change that file, even a one-line fix or suggested edits; and any follow-up change to a Google file from earlier in the chat, even \"change it\" or \"add a tab\". Includes helper scripts for document positions, cell ranges and slide layout. However, if the user asks for a doc, deck or spreadsheet without naming Google, or gives a Google file only as source material for something new, use Claude''s own output type instead. Do NOT use for read-only questions about a Google file, or for Word, Excel, PowerPoint or PDF files."},{"id":"import-memory","name":"import-memory","description":"Import a memory export from another AI assistant into Claude''s memory — conversationally, additively, and with the content treated as data."},{"id":"merakli-stil","name":"merakli-stil","description":"Meraklı YouTube kanalı için banner, kapak, karakter, animasyon ve seslendirmeli video üretirken kullan; marka renkleri, kapak kuralları ve Canvas2D video hattını içerir."},{"id":"morning","name":"morning","description":"Render the user''s morning brief as a styled HTML artifact, or set it up as a recurring weekday task. Use only when the user explicitly asks to run, see, or set up their morning brief, or if they invoke /morning by name. A question about their day, schedule, or calendar is not by itself a request for the brief; answer it directly instead."},{"id":"nocobase-acl-manage","name":"nocobase-acl-manage","description":"Task-driven ACL governance through nb CLI for role lifecycle, global role mode, permission policy, Portal entry access, user-role membership, and risk assessment. Use when users describe business permission outcomes instead of raw command arguments, including which roles may enter a Portal."},{"id":"nocobase-ai-builder","name":"nocobase-ai-builder","description":"Design, build, and verify source-code applications in NocoBase AI Portals based on @nocobase/portal-template-default. This skill is mandatory whenever the selected Portal has portalType \"ai\". Use it for complete systems, pages, navigation, CRUD, dashboards, forms, dialogs, drawers, URL-addressable subpages, authentication UI, frontend access control, localization, themes, NocoBase API integration, responsive design, test-driven verification, and safe upgrades of existing Portal template, SDK, and Registry source."},{"id":"nocobase-ai-employee","name":"nocobase-ai-employee","description":"Use when a NocoBase task requires AI employee lifecycle work such as discovering existing employees, judging fit, creating or maintaining a dedicated employee, and preparing it before another skill binds it to a UI surface."},{"id":"nocobase-ai-knowledge-base-manager","name":"nocobase-ai-knowledge-base-manager","description":"Use when users need to check Professional+ knowledge-base capability, consent to enabling an installed-disabled KB plugin, or manage NocoBase vector databases, Local/Readonly/External knowledge bases, documents, retrieval tests, and binding preparation through nb api kb."},{"id":"nocobase-ai-manager","name":"nocobase-ai-manager","description":"Use when users need to inspect or maintain NocoBase core AI prerequisites through nb api, including LLM providers, saved services, CLI-versus-UI configuration routing, chat models, embedding discovery, secure credentials, and dependency-safe changes."},{"id":"nocobase-data-analysis","name":"nocobase-data-analysis","description":"Query and analyze business data in NocoBase via MCP. Use when users want current counts, grouped breakdowns, owner/source distributions, or business summaries across collections, with main data source first and fallback discovery to other enabled data sources."},{"id":"nocobase-data-modeling","name":"nocobase-data-modeling","description":"Create and manage NocoBase data models through the available data-modeling surface. Use when users want to inspect or change collections, fields, relations, or view-backed schemas in a NocoBase app."},{"id":"nocobase-dsl-reconciler","name":"nocobase-dsl-reconciler","description":"**Opt-in DSL path** for NocoBase app building. Use ONLY when the user explicitly asks for YAML / DSL / committed-to-git / `cli push` / spec files — e.g. \"use the DSL reconciler\", \"I want YAML I can commit\", \"build this as a workspaces/ project\". For any other UI authoring request (new page, new block, tweak an existing screen), default to `nocobase-ui-builder` instead — this reconciler is still in active development and has rough edges that the live-UI path avoids.\nWhen the user opts in: produces/changes files under `workspaces/<project>/`, supports new pages, menus, modules, whole systems, collections, tables, sub-tables, popups, dashboards, approval workflows, recordActions, and deploys them via `cli push`."},{"id":"nocobase-env-manage","name":"nocobase-env-manage","description":"Use when users need NocoBase bootstrap, runtime lifecycle, CLI maintenance, and skills maintenance with nb CLI only."},{"id":"nocobase-file-manager","name":"nocobase-file-manager","description":"Manage NocoBase file storage engines, file collections, their business relations, and file records through nb CLI. Use when the primary task concerns NocoBase file storage configuration, file-model setup, access or retention behavior, or storage/file-record lifecycle."},{"id":"nocobase-notification-manage","name":"nocobase-notification-manage","description":"Use when users need to configure, inspect, test, or troubleshoot NocoBase notification management, including in-app message channels, email SMTP channels, workflow notification nodes, and notification send logs."},{"id":"nocobase-plugin-development","name":"nocobase-plugin-development","description":"Step-by-step playbook for developing a NocoBase plugin, covering scaffolding, server-side code (collections, APIs, ACL, migrations), client-side code (blocks, fields, actions, settings pages, routes, components), i18n, and verification.\nTRIGGER when: user asks to create, build, implement, or develop a NocoBase plugin, mentions ''NocoBase plugin'', or describes a feature to be built as a NocoBase plugin. This skill contains NocoBase-specific conventions and templates that general coding cannot replicate — always invoke it instead of planning from scratch."},{"id":"nocobase-plugin-manage","name":"nocobase-plugin-manage","description":"Use when users need to inspect, enable, or disable NocoBase plugins with direct `nb plugin` commands only."},{"id":"nocobase-portal-manage","name":"nocobase-portal-manage","description":"PRIMARY ENTRY. Default dispatcher for every NocoBase UI authoring request, including pages, menus, blocks, fields, actions, layouts, reactions, KPI interfaces, dashboards, and charts, even when the user does not mention a Portal. Resolve an enabled Portal from structured `nb portal list -j` output before loading any downstream UI implementation skill: exact `name` when specified; otherwise zero stops, exactly one is automatic, and multiple Portals require explicit selection even when one has `isDefault: true`; show the default marker as informational only. Read the type only from `portalType`; explicitly invoke `nocobase-ui-builder` only for a resolved no-code Portal and require `nocobase-ai-builder` for AI Portal source development. Also use direct `nb portal` commands for explicit Portal lifecycle, source storage, sync, deployment, diagnosis, and destroy tasks."},{"id":"nocobase-prototype-repro","name":"nocobase-prototype-repro","description":"Use when the task hands over a prototype to reproduce in NocoBase — an HTML file, an image, or a link, INCLUDING the published form \"Build a NocoBase app — X … Match the layout and signature visuals of this reference prototype: <url>\" — or when someone says a built page \"doesn''t match the prototype / looks monotone / is ugly\". A prototype/URL being present is what triggers this skill, EVEN WHEN the same prompt also says \"build an app\" — the verb \"build\" does not cancel the prototype. Skip it only when there is NO prototype at all: a bare \"build me a CRM/app\" → nocobase-app- discipline; a one-field edit / rename → nocobase-ui-builder. It turns \"here is the prototype, build it\" into a faithful app — analyze prototype, native CRUD, the right native block per region, then a screenshot-vs-prototype visual loop. Native-first; JS only inside a native container; never a full-page JS block. Mechanics live in nocobase-ui-builder, nocobase-data-modeling, nocobase-app-discipline."},{"id":"nocobase-publish-manage","name":"nocobase-publish-manage","description":"Use when users need NocoBase backup restore or migration publish operations through nb api backup and nb api migration commands."},{"id":"nocobase-revision","name":"nocobase-revision","description":"Use when building a NocoBase app with NocoBase skills or the nb CLI and you want to save a completed, meaningful milestone as a restorable revision."},{"id":"nocobase-ui-builder","name":"nocobase-ui-builder","description":"INTERNAL CONTINUATION ONLY. Never select this skill from a raw user UI request. Use it only after nocobase-portal-manage has already resolved exactly one enabled Portal and confirmed portalType=no-code, or has proved the runtime has capabilities.multiPortal === false. Portal count or `isDefault` marker alone never enables this skill. It authors NocoBase Modern UI pages, blocks, menu items, fields, actions, layouts, reactions, and AI employee placements through backend flow-surfaces via nb api. Unresolved UI requests go to nocobase-portal-manage first; a sole AI Portal is handed off automatically for source-code implementation. Hand off AI employee lifecycle work to nocobase-ai-employee and explicit DSL, YAML, git, or cli push workflows to nocobase-dsl-reconciler. It does not handle ACL, data modeling, workflow orchestration, browser reproduction, page error postmortems, Portal lifecycle, AI Portal source-code UI, or non-Modern page navigation."},{"id":"nocobase-utils","name":"nocobase-utils","description":"General-purpose NocoBase reference utilities covering filter conditions and field-specific operators, evaluator engines, expression syntax, UID generation, and more. Use when you need authoritative reference information or reusable snippets that apply across multiple NocoBase features."},{"id":"nocobase-workflow-manage","name":"nocobase-workflow-manage","description":"Use when users need to inspect, create, update, copy, enable, or diagnose NocoBase workflows through `nb`, including version-safe edits, node changes, approval surfaces, and execution troubleshooting."},{"id":"pdf","name":"pdf","description":"Use this skill whenever the user wants to do anything with PDF files. This includes reading or extracting text/tables from PDFs, combining or merging multiple PDFs into one, splitting PDFs apart, rotating pages, adding watermarks, creating new PDFs, filling PDF forms, encrypting/decrypting PDFs, extracting images, and OCR on scanned PDFs to make them searchable. If the user mentions a .pdf file or asks to produce one, use this skill."},{"id":"pptx","name":"pptx","description":"Use this skill any time a .pptx or .potx file is involved in any way — as input, output, or both. This includes: creating slide decks, pitch decks, or presentations as PowerPoint (.pptx) files; reading, parsing, or extracting text from any .pptx or .potx file (even if the extracted content will be used elsewhere, like in an email, summary, or creating a different type of slide deck); editing, modifying, or updating existing presentations; combining or splitting slide files; working with templates (.potx), layouts, speaker notes, or comments. Trigger whenever the user asks for a PowerPoint or .pptx file, or references a .pptx or .potx filename, regardless of what they plan to do with the content afterward. However, when the user asks for a deck, slides, a slide deck, or a presentation without naming a file format, default to using a dedicated slide-deck artifact type or a separate slides skill if this session offers one; otherwise, use this skill."},{"id":"skill-creator","name":"skill-creator","description":"Create new skills, modify and improve existing skills, and measure skill performance. Use when users want to create a skill from scratch, edit, or optimize an existing skill, run evals to test a skill, benchmark skill performance with variance analysis, or optimize a skill''s description for better triggering accuracy."},{"id":"web-artifacts-builder","name":"web-artifacts-builder","description":"Suite of tools for creating elaborate, multi-component claude.ai HTML artifacts using modern frontend web technologies (React, Tailwind CSS, shadcn/ui). Use for complex artifacts requiring state management, routing, or shadcn/ui components - not for simple single-file HTML/JSX artifacts."},{"id":"xlsx","name":"xlsx","description":"Use this skill any time a spreadsheet file is the primary input or output. This means any task where the user wants to: open, read, edit, or fix an existing .xlsx, .xlsm, .xltx, .csv, or .tsv file (e.g., adding columns, computing formulas, formatting, charting, cleaning messy data); create a new spreadsheet from scratch or from other data sources; or convert between tabular file formats. Trigger especially when the user references a spreadsheet file by name or path — even casually (like \"the xlsx in my downloads\") — and wants something done to it or produced from it. Also trigger for cleaning or restructuring messy tabular data files (malformed rows, misplaced headers, junk data) into proper spreadsheets. The deliverable must be a spreadsheet file. Do NOT trigger when the primary deliverable is a Word document, HTML report, standalone Python script, database pipeline, or Google Sheets API integration, even if tabular data is involved."}]');
INSERT INTO instruction_blob VALUES('586c69c6cc14b767cc6926489f159ee96a06f041b9bfb4dec28f5d2a1d690447','"Wed Nov 04 2026"');
INSERT INTO instruction_blob VALUES('cce5ef0c262018ff82e7ad47a3f661aedfa1d982f7c2b50f6abc06a3b29ef34a','"<env>\n  Working directory: @HOME@/proj\n  Workspace root folder: @HOME@/proj\n  Is directory a git repo: yes\n  Platform: linux\n  Prefer /tmp/opencode over generic system temporary directories such as /tmp; it is pre-created and approved for external access.\n</env>"');
CREATE TABLE `instruction_entry` (
          `session_id` text NOT NULL,
          `key` text NOT NULL,
          `value` text,
          `removed` integer DEFAULT false NOT NULL,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          CONSTRAINT `instruction_entry_pk` PRIMARY KEY(`session_id`, `key`),
          CONSTRAINT `fk_instruction_entry_session_id_session_v2_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session_v2`(`id`) ON DELETE CASCADE
        );
CREATE TABLE `instruction_state` (
          `session_id` text PRIMARY KEY,
          `epoch_start` integer NOT NULL,
          `through_seq` integer NOT NULL,
          `initial_values` text NOT NULL,
          `current_values` text NOT NULL,
          CONSTRAINT `fk_instruction_state_session_id_session_v2_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session_v2`(`id`) ON DELETE CASCADE
        );
INSERT INTO instruction_state VALUES('ses_e57d70d7fffe0TqPcsNMhaeh6R',3,3,'{"core/codemode":"1f1291478accf8b3e4d96a40c150bf844d60557dede4aae56c47ea0043bbdce2","core/skill-guidance":"0bcf371d396484ebb990d27221437252e87df91ebb2b32b08f36af7ca80d008f","core/date":"586c69c6cc14b767cc6926489f159ee96a06f041b9bfb4dec28f5d2a1d690447","core/environment":"cce5ef0c262018ff82e7ad47a3f661aedfa1d982f7c2b50f6abc06a3b29ef34a"}','{"core/codemode":"1f1291478accf8b3e4d96a40c150bf844d60557dede4aae56c47ea0043bbdce2","core/skill-guidance":"0bcf371d396484ebb990d27221437252e87df91ebb2b32b08f36af7ca80d008f","core/date":"586c69c6cc14b767cc6926489f159ee96a06f041b9bfb4dec28f5d2a1d690447","core/environment":"cce5ef0c262018ff82e7ad47a3f661aedfa1d982f7c2b50f6abc06a3b29ef34a"}');
INSERT INTO instruction_state VALUES('ses_e57d7012cffe72xpyfIsYUr33S',3,3,'{"core/codemode":"1f1291478accf8b3e4d96a40c150bf844d60557dede4aae56c47ea0043bbdce2","core/skill-guidance":"0bcf371d396484ebb990d27221437252e87df91ebb2b32b08f36af7ca80d008f","core/date":"586c69c6cc14b767cc6926489f159ee96a06f041b9bfb4dec28f5d2a1d690447","core/environment":"cce5ef0c262018ff82e7ad47a3f661aedfa1d982f7c2b50f6abc06a3b29ef34a"}','{"core/codemode":"1f1291478accf8b3e4d96a40c150bf844d60557dede4aae56c47ea0043bbdce2","core/skill-guidance":"0bcf371d396484ebb990d27221437252e87df91ebb2b32b08f36af7ca80d008f","core/date":"586c69c6cc14b767cc6926489f159ee96a06f041b9bfb4dec28f5d2a1d690447","core/environment":"cce5ef0c262018ff82e7ad47a3f661aedfa1d982f7c2b50f6abc06a3b29ef34a"}');
INSERT INTO instruction_state VALUES('ses_e57d6f7a8ffen9abAz9CaEqwMp',0,0,'{"core/codemode":"1f1291478accf8b3e4d96a40c150bf844d60557dede4aae56c47ea0043bbdce2","core/skill-guidance":"0bcf371d396484ebb990d27221437252e87df91ebb2b32b08f36af7ca80d008f","core/date":"586c69c6cc14b767cc6926489f159ee96a06f041b9bfb4dec28f5d2a1d690447","core/environment":"cce5ef0c262018ff82e7ad47a3f661aedfa1d982f7c2b50f6abc06a3b29ef34a"}','{"core/codemode":"1f1291478accf8b3e4d96a40c150bf844d60557dede4aae56c47ea0043bbdce2","core/skill-guidance":"0bcf371d396484ebb990d27221437252e87df91ebb2b32b08f36af7ca80d008f","core/date":"586c69c6cc14b767cc6926489f159ee96a06f041b9bfb4dec28f5d2a1d690447","core/environment":"cce5ef0c262018ff82e7ad47a3f661aedfa1d982f7c2b50f6abc06a3b29ef34a"}');
INSERT INTO instruction_state VALUES('ses_e57d6e6feffe4gefNuJdeuiqXd',9,9,'{"core/codemode":"1f1291478accf8b3e4d96a40c150bf844d60557dede4aae56c47ea0043bbdce2","core/skill-guidance":"0bcf371d396484ebb990d27221437252e87df91ebb2b32b08f36af7ca80d008f","core/date":"586c69c6cc14b767cc6926489f159ee96a06f041b9bfb4dec28f5d2a1d690447","core/environment":"cce5ef0c262018ff82e7ad47a3f661aedfa1d982f7c2b50f6abc06a3b29ef34a"}','{"core/codemode":"1f1291478accf8b3e4d96a40c150bf844d60557dede4aae56c47ea0043bbdce2","core/skill-guidance":"0bcf371d396484ebb990d27221437252e87df91ebb2b32b08f36af7ca80d008f","core/date":"586c69c6cc14b767cc6926489f159ee96a06f041b9bfb4dec28f5d2a1d690447","core/environment":"cce5ef0c262018ff82e7ad47a3f661aedfa1d982f7c2b50f6abc06a3b29ef34a"}');
CREATE TABLE `session_inbox` (
          `id` text PRIMARY KEY,
          `session_id` text NOT NULL,
          `type` text NOT NULL,
          `payload` text NOT NULL,
          `delivery` text NOT NULL,
          `enqueued_seq` integer NOT NULL,
          `time_created` integer NOT NULL,
          CONSTRAINT `fk_session_inbox_session_id_session_v2_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session_v2`(`id`) ON DELETE CASCADE
        );
CREATE TABLE `session_message` (
          `id` text PRIMARY KEY,
          `session_id` text NOT NULL,
          `type` text NOT NULL,
          `seq` integer NOT NULL,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          `data` text NOT NULL,
          CONSTRAINT `fk_session_message_session_id_session_v2_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session_v2`(`id`) ON DELETE CASCADE
        );
INSERT INTO session_message VALUES('msg_1a828f765001VlbOALiBS6o9AK','ses_e57d70d7fffe0TqPcsNMhaeh6R','user',4,1793822619633,1793822619638,'{"time":{"created":1793822619633},"text":"\"RICHTEST please do the scripted steps\"","files":[]}');
INSERT INTO session_message VALUES('msg_1a828f833001WdfLqr3O7Xn5b0','ses_e57d70d7fffe0TqPcsNMhaeh6R','assistant',7,1793822619717,1793822620294,'{"time":{"created":1793822619717,"streamed":1793822620037,"completed":1793822620280},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"reasoning","text":"I should look at the repo first.","native":{"signature":"SIG_0_0_ErUBCkYIBRgCKkA"},"time":{"created":1793822619912,"completed":1793822619944}},{"type":"text","text":"Let me check the shell."},{"type":"tool","id":"toolu_0_shell","name":"shell","executed":false,"state":{"status":"completed","input":{"command":"echo hello > hello.txt && cat hello.txt"},"content":[{"type":"text","text":"hello\n"}],"metadata":{"status":"completed","truncated":false,"exit":0}},"time":{"created":1793822619992,"ran":1793822620023,"completed":1793822620160}}],"snapshot":{"start":"eda95054045192600138498365449de4b83842bc","end":"e07a13d265c0818b1b97b82c40f2f729e7eeec77","files":["hello.txt"]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a828faa7001r2BuZko71RGmAE','ses_e57d70d7fffe0TqPcsNMhaeh6R','assistant',18,1793822620337,1793822620568,'{"time":{"created":1793822620337,"streamed":1793822620457,"completed":1793822620563},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"reasoning","text":"","native":{"redactedData":"REDACTED_1_EmwKAhgBEgy"},"time":{"created":1793822620399,"completed":1793822620413}},{"type":"reasoning","text":"Now write notes.","native":{"signature":"SIG_1_1_ErUBCkYIBRgCKkA"},"time":{"created":1793822620420,"completed":1793822620429}},{"type":"tool","id":"toolu_1_write","name":"write","executed":false,"state":{"status":"completed","input":{"path":"notes.md","content":"# Notes\n\nfirst line\n"},"content":[{"type":"text","text":"Created file successfully: notes.md"}],"metadata":{"truncated":false}},"time":{"created":1793822620436,"ran":1793822620445,"completed":1793822620481}}],"snapshot":{"start":"e07a13d265c0818b1b97b82c40f2f729e7eeec77","end":"21866f0bbca4ac1a47e2a1748dc77807b5d639ac","files":["notes.md"]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a828fbb7001cffT3dxGe26mwO','ses_e57d70d7fffe0TqPcsNMhaeh6R','assistant',29,1793822620609,1793822620737,'{"time":{"created":1793822620609,"streamed":1793822620672,"completed":1793822620736},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"tool","id":"toolu_2_edit","name":"edit","executed":false,"state":{"status":"completed","input":{"path":"notes.md","oldString":"first line","newString":"second line"},"content":[{"type":"text","text":"Edited notes.md (1 replacement)"}],"metadata":{"files":[{"file":"notes.md","patch":"Index: notes.md\n===================================================================\n--- notes.md\n+++ notes.md\n@@ -1,3 +1,3 @@\n # Notes\n \n-first line\n+second line\n","status":"modified","additions":1,"deletions":1}],"truncated":false}},"time":{"created":1793822620654,"ran":1793822620667,"completed":1793822620693}}],"snapshot":{"start":"21866f0bbca4ac1a47e2a1748dc77807b5d639ac","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":["notes.md"]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a828fc59001m9b3HfWebm6Oeg','ses_e57d70d7fffe0TqPcsNMhaeh6R','assistant',36,1793822620767,1793822620885,'{"time":{"created":1793822620767,"streamed":1793822620835,"completed":1793822620882},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"tool","id":"toolu_3_read","name":"read","executed":false,"state":{"status":"completed","input":{"path":"notes.md"},"content":[{"type":"text","text":"Read file notes.md, lines 1-3\n1: # Notes\n2: \n3: second line"}],"metadata":{"truncated":false}},"time":{"created":1793822620812,"ran":1793822620827,"completed":1793822620853}}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a828fce5001j9YMrxhkoTHXJN','ses_e57d70d7fffe0TqPcsNMhaeh6R','assistant',43,1793822620905,1793822620969,'{"time":{"created":1793822620905,"streamed":1793822620938,"completed":1793822620965},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"tool","id":"toolu_4_read","name":"read","executed":false,"state":{"status":"error","input":{"path":"does-not-exist.txt"},"error":{"type":"tool.execution","message":"File not found: does-not-exist.txt"}},"time":{"created":1793822620929,"ran":1793822620934,"completed":1793822620944}}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a828fd3f001ZyQIyyr2fKoXD0','ses_e57d70d7fffe0TqPcsNMhaeh6R','assistant',50,1793822621003,1793822621171,'{"time":{"created":1793822621003,"streamed":1793822621116,"completed":1793822621169},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"tool","id":"toolu_5_todowrite","name":"todowrite","executed":false,"state":{"status":"error","input":{"todos":[{"content":"Write notes","status":"completed"},{"content":"Review notes","status":"in_progress"},{"content":"Ship","status":"pending"}]},"error":{"type":"tool.execution","message":"No tool named \"todowrite\" is currently available. Please use a tool from the available tool list."}},"time":{"created":1793822621097,"ran":1793822621110,"completed":1793822621121}}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a828fe2d0016wvX7g5t44aVNv','ses_e57d70d7fffe0TqPcsNMhaeh6R','assistant',57,1793822621246,1793822621815,'{"time":{"created":1793822621246,"streamed":1793822621391,"completed":1793822621813},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"tool","id":"toolu_6_subagent","name":"subagent","executed":false,"state":{"status":"completed","input":{"agent":"general","description":"List the files","prompt":"SUBTASK: list the files in this directory"},"content":[{"type":"text","text":"<subagent sessionID=\"ses_e57d7012cffe72xpyfIsYUr33S\" state=\"completed\">\nhello.txt notes.md\n</subagent>"}],"metadata":{"sessionID":"ses_e57d7012cffe72xpyfIsYUr33S","status":"completed","truncated":false}},"time":{"created":1793822621372,"ran":1793822621384,"completed":1793822621773}}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a828fefe001aoQmbYyFH6sCF4','ses_e57d7012cffe72xpyfIsYUr33S','user',4,1793822621511,1793822621513,'{"time":{"created":1793822621511},"text":"You are a subagent spawned by another session.\nSUBTASK: list the files in this directory"}');
INSERT INTO session_message VALUES('msg_1a828ff4e001Gg5wXVlqrp6Me1','ses_e57d7012cffe72xpyfIsYUr33S','assistant',5,1793822621525,1793822621741,'{"time":{"created":1793822621525,"streamed":1793822621652,"completed":1793822621707},"agent":"general","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"text","text":"hello.txt notes.md"}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"stop","rawFinish":"end_turn","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a8290038001ywV5jMhcN53yRp','ses_e57d7012cffe72xpyfIsYUr33S','idle',10,1793822621752,1793822621768,'{"time":{"created":1793822621752},"outcome":"succeeded"}');
INSERT INTO session_message VALUES('msg_1a829009b001mlo5rLK9VvFPYF','ses_e57d70d7fffe0TqPcsNMhaeh6R','assistant',64,1793822621870,1793822621936,'{"time":{"created":1793822621870,"streamed":1793822621907,"completed":1793822621934},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"reasoning","text":"All done.","native":{"signature":"SIG_7_0_ErUBCkYIBRgCKkA"},"time":{"created":1793822621893,"completed":1793822621897}},{"type":"text","text":"Done: wrote notes.md and listed files."}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"stop","rawFinish":"end_turn","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a82900fb002Xl7WvxvDuJCw3v','ses_e57d70d7fffe0TqPcsNMhaeh6R','idle',71,1793822621947,1793822621949,'{"time":{"created":1793822621947},"outcome":"succeeded"}');
INSERT INTO session_message VALUES('msg_1a8290a03001DhRzpdCJ5yi7Bk_4','ses_e57d6f7a8ffen9abAz9CaEqwMp','user',4,1793822619633,1793822619638,'{"time":{"created":1793822619633},"text":"\"RICHTEST please do the scripted steps\"","files":[]}');
INSERT INTO session_message VALUES('msg_1a8290a03001DhRzpdCJ5yi7Bk_7','ses_e57d6f7a8ffen9abAz9CaEqwMp','assistant',7,1793822619717,1793822620294,'{"time":{"created":1793822619717,"streamed":1793822620037,"completed":1793822620280},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"reasoning","text":"I should look at the repo first.","native":{"signature":"SIG_0_0_ErUBCkYIBRgCKkA"},"time":{"created":1793822619912,"completed":1793822619944}},{"type":"text","text":"Let me check the shell."},{"type":"tool","id":"toolu_0_shell","name":"shell","executed":false,"state":{"status":"completed","input":{"command":"echo hello > hello.txt && cat hello.txt"},"content":[{"type":"text","text":"hello\n"}],"metadata":{"status":"completed","truncated":false,"exit":0}},"time":{"created":1793822619992,"ran":1793822620023,"completed":1793822620160}}],"snapshot":{"start":"eda95054045192600138498365449de4b83842bc","end":"e07a13d265c0818b1b97b82c40f2f729e7eeec77","files":["hello.txt"]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a8290a03001DhRzpdCJ5yi7Bk_18','ses_e57d6f7a8ffen9abAz9CaEqwMp','assistant',18,1793822620337,1793822620568,'{"time":{"created":1793822620337,"streamed":1793822620457,"completed":1793822620563},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"reasoning","text":"","native":{"redactedData":"REDACTED_1_EmwKAhgBEgy"},"time":{"created":1793822620399,"completed":1793822620413}},{"type":"reasoning","text":"Now write notes.","native":{"signature":"SIG_1_1_ErUBCkYIBRgCKkA"},"time":{"created":1793822620420,"completed":1793822620429}},{"type":"tool","id":"toolu_1_write","name":"write","executed":false,"state":{"status":"completed","input":{"path":"notes.md","content":"# Notes\n\nfirst line\n"},"content":[{"type":"text","text":"Created file successfully: notes.md"}],"metadata":{"truncated":false}},"time":{"created":1793822620436,"ran":1793822620445,"completed":1793822620481}}],"snapshot":{"start":"e07a13d265c0818b1b97b82c40f2f729e7eeec77","end":"21866f0bbca4ac1a47e2a1748dc77807b5d639ac","files":["notes.md"]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a8290a03001DhRzpdCJ5yi7Bk_29','ses_e57d6f7a8ffen9abAz9CaEqwMp','assistant',29,1793822620609,1793822620737,'{"time":{"created":1793822620609,"streamed":1793822620672,"completed":1793822620736},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"tool","id":"toolu_2_edit","name":"edit","executed":false,"state":{"status":"completed","input":{"path":"notes.md","oldString":"first line","newString":"second line"},"content":[{"type":"text","text":"Edited notes.md (1 replacement)"}],"metadata":{"files":[{"file":"notes.md","patch":"Index: notes.md\n===================================================================\n--- notes.md\n+++ notes.md\n@@ -1,3 +1,3 @@\n # Notes\n \n-first line\n+second line\n","status":"modified","additions":1,"deletions":1}],"truncated":false}},"time":{"created":1793822620654,"ran":1793822620667,"completed":1793822620693}}],"snapshot":{"start":"21866f0bbca4ac1a47e2a1748dc77807b5d639ac","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":["notes.md"]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a8290a03001DhRzpdCJ5yi7Bk_36','ses_e57d6f7a8ffen9abAz9CaEqwMp','assistant',36,1793822620767,1793822620885,'{"time":{"created":1793822620767,"streamed":1793822620835,"completed":1793822620882},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"tool","id":"toolu_3_read","name":"read","executed":false,"state":{"status":"completed","input":{"path":"notes.md"},"content":[{"type":"text","text":"Read file notes.md, lines 1-3\n1: # Notes\n2: \n3: second line"}],"metadata":{"truncated":false}},"time":{"created":1793822620812,"ran":1793822620827,"completed":1793822620853}}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a8290a03001DhRzpdCJ5yi7Bk_43','ses_e57d6f7a8ffen9abAz9CaEqwMp','assistant',43,1793822620905,1793822620969,'{"time":{"created":1793822620905,"streamed":1793822620938,"completed":1793822620965},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"tool","id":"toolu_4_read","name":"read","executed":false,"state":{"status":"error","input":{"path":"does-not-exist.txt"},"error":{"type":"tool.execution","message":"File not found: does-not-exist.txt"}},"time":{"created":1793822620929,"ran":1793822620934,"completed":1793822620944}}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a8290a03001DhRzpdCJ5yi7Bk_50','ses_e57d6f7a8ffen9abAz9CaEqwMp','assistant',50,1793822621003,1793822621171,'{"time":{"created":1793822621003,"streamed":1793822621116,"completed":1793822621169},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"tool","id":"toolu_5_todowrite","name":"todowrite","executed":false,"state":{"status":"error","input":{"todos":[{"content":"Write notes","status":"completed"},{"content":"Review notes","status":"in_progress"},{"content":"Ship","status":"pending"}]},"error":{"type":"tool.execution","message":"No tool named \"todowrite\" is currently available. Please use a tool from the available tool list."}},"time":{"created":1793822621097,"ran":1793822621110,"completed":1793822621121}}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a8290a03001DhRzpdCJ5yi7Bk_57','ses_e57d6f7a8ffen9abAz9CaEqwMp','assistant',57,1793822621246,1793822621815,'{"time":{"created":1793822621246,"streamed":1793822621391,"completed":1793822621813},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"tool","id":"toolu_6_subagent","name":"subagent","executed":false,"state":{"status":"completed","input":{"agent":"general","description":"List the files","prompt":"SUBTASK: list the files in this directory"},"content":[{"type":"text","text":"<subagent sessionID=\"ses_e57d7012cffe72xpyfIsYUr33S\" state=\"completed\">\nhello.txt notes.md\n</subagent>"}],"metadata":{"sessionID":"ses_e57d7012cffe72xpyfIsYUr33S","status":"completed","truncated":false}},"time":{"created":1793822621372,"ran":1793822621384,"completed":1793822621773}}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"tool-calls","rawFinish":"tool_use","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a8290a03001DhRzpdCJ5yi7Bk_64','ses_e57d6f7a8ffen9abAz9CaEqwMp','assistant',64,1793822621870,1793822621936,'{"time":{"created":1793822621870,"streamed":1793822621907,"completed":1793822621934},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"reasoning","text":"All done.","native":{"signature":"SIG_7_0_ErUBCkYIBRgCKkA"},"time":{"created":1793822621893,"completed":1793822621897}},{"type":"text","text":"Done: wrote notes.md and listed files."}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"stop","rawFinish":"end_turn","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a8290a03001DhRzpdCJ5yi7Bk_71','ses_e57d6f7a8ffen9abAz9CaEqwMp','idle',71,1793822621947,1793822621949,'{"time":{"created":1793822621947},"outcome":"succeeded"}');
INSERT INTO session_message VALUES('msg_1a8290dd5001I5pXxBcukYcobM','ses_e57d6f7a8ffen9abAz9CaEqwMp','model-switched',72,1793822625237,1793822625250,'{"time":{"created":1793822625237},"model":{"id":"claude-sonnet-5-5","providerID":"anysola","variant":"high"}}');
INSERT INTO session_message VALUES('msg_1a8290dc80013a6FkHNn7eMXb3','ses_e57d6f7a8ffen9abAz9CaEqwMp','user',75,1793822625503,1793822625513,'{"time":{"created":1793822625503},"text":"\"third turn with files\"\n\n<file name=\"note.txt\">\nattached note\n\n</file>","files":[{"data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==","mime":"image/png","source":{"type":"inline"},"name":"pixel.png"}]}');
INSERT INTO session_message VALUES('msg_1a8290f1c001EzSK5tiHXygMsm','ses_e57d6f7a8ffen9abAz9CaEqwMp','assistant',76,1793822625593,1793822625916,'{"time":{"created":1793822625593,"streamed":1793822625823,"completed":1793822625908},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola","variant":"high"},"content":[{"type":"reasoning","text":"All done.","native":{"signature":"SIG_8_0_ErUBCkYIBRgCKkA"},"time":{"created":1793822625759,"completed":1793822625776}},{"type":"text","text":"Done: wrote notes.md and listed files."}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"stop","rawFinish":"end_turn","cost":0,"tokens":{"input":120,"output":42,"reasoning":0,"cache":{"read":800,"write":40}}}');
INSERT INTO session_message VALUES('msg_1a8291099001T2fd4mCz03kJil','ses_e57d6f7a8ffen9abAz9CaEqwMp','idle',83,1793822625945,1793822625946,'{"time":{"created":1793822625945},"outcome":"succeeded"}');
INSERT INTO session_message VALUES('msg_1a8291d11001i3CxTzYLBpESQC','ses_e57d6e6feffe4gefNuJdeuiqXd','user',4,1793822629251,1793822629252,'{"time":{"created":1793822629251},"text":"\"OVERFLOW please\"","files":[]}');
INSERT INTO session_message VALUES('msg_1a8291e20001EtK7VDHboL82ov','ses_e57d6e6feffe4gefNuJdeuiqXd','compaction',5,1793822629408,1793822629538,'{"time":{"created":1793822629408},"status":"completed","reason":"auto","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"summary":"## Objective\n- Test objective\n\n## Requirements\n- (none)\n\n## Decisions\n- (none)\n\n## Work State\n### Completed\n- (none)\n\n### Active\n- (none)\n\n### Blocked\n- (none)\n\n## Next Move\n1. (none)\n\n## Relevant Files\n- (none)\n\n## Important Context\n- (none)","cost":0,"tokens":{"input":10,"output":3,"reasoning":0,"cache":{"read":5,"write":3}}}');
INSERT INTO session_message VALUES('msg_1a8291ea9001pPmy5cqykLoag1','ses_e57d6e6feffe4gefNuJdeuiqXd','assistant',10,1793822629577,1793822629826,'{"time":{"created":1793822629577,"streamed":1793822629699,"completed":1793822629789},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"text","text":"Hello answer."}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"stop","rawFinish":"end_turn","cost":0,"tokens":{"input":10,"output":3,"reasoning":0,"cache":{"read":5,"write":3}}}');
INSERT INTO session_message VALUES('msg_1a8291fd6001Jvrn42BvBYKS3Y','ses_e57d6e6feffe4gefNuJdeuiqXd','idle',15,1793822629846,1793822629856,'{"time":{"created":1793822629846},"outcome":"succeeded"}');
INSERT INTO session_message VALUES('msg_1a8293077001BrgITkn8TfyPDQ','ses_e57d6e6feffe4gefNuJdeuiqXd','user',18,1793822634224,1793822634230,'{"time":{"created":1793822634224},"text":"\"after the checkpoint\"","files":[]}');
INSERT INTO session_message VALUES('msg_1a829311b0013nhmyjE5HNB0vU','ses_e57d6e6feffe4gefNuJdeuiqXd','assistant',19,1793822634300,1793822634628,'{"time":{"created":1793822634300,"streamed":1793822634555,"completed":1793822634617},"agent":"build","model":{"id":"claude-sonnet-5-5","providerID":"anysola"},"content":[{"type":"text","text":"Hello answer."}],"snapshot":{"start":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","end":"c05783d36a778ed4171287fd5c0f15cf0d95d69d","files":[]},"finish":"stop","rawFinish":"end_turn","cost":0,"tokens":{"input":10,"output":3,"reasoning":0,"cache":{"read":5,"write":3}}}');
INSERT INTO session_message VALUES('msg_1a8293291001JAyv8EccE057oB','ses_e57d6e6feffe4gefNuJdeuiqXd','idle',24,1793822634641,1793822634653,'{"time":{"created":1793822634641},"outcome":"succeeded"}');
CREATE TABLE `session_part` (
          `message_id` text NOT NULL,
          `session_id` text NOT NULL,
          `ordinal` integer NOT NULL,
          `type` text NOT NULL,
          `call_id` text,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          `data` text NOT NULL,
          CONSTRAINT `session_part_pk` PRIMARY KEY(`message_id`, `ordinal`),
          CONSTRAINT `fk_session_part_message_id_session_message_id_fk` FOREIGN KEY (`message_id`) REFERENCES `session_message`(`id`) ON DELETE CASCADE,
          CONSTRAINT `fk_session_part_session_id_session_v2_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session_v2`(`id`) ON DELETE CASCADE
        );
CREATE TABLE `session_v2` (
          `id` text PRIMARY KEY,
          `project_id` text NOT NULL,
          `workspace_id` text,
          `parent_id` text,
          `fork_session_id` text,
          `fork_boundary` text,
          `slug` text NOT NULL,
          `directory` text NOT NULL,
          `path` text,
          `title` text,
          `version` text NOT NULL,
          `share_url` text,
          `summary_additions` integer,
          `summary_deletions` integer,
          `summary_files` integer,
          `summary_diffs` text,
          `metadata` text,
          `cost` real DEFAULT 0 NOT NULL,
          `tokens_input` integer DEFAULT 0 NOT NULL,
          `tokens_output` integer DEFAULT 0 NOT NULL,
          `tokens_reasoning` integer DEFAULT 0 NOT NULL,
          `tokens_cache_read` integer DEFAULT 0 NOT NULL,
          `tokens_cache_write` integer DEFAULT 0 NOT NULL,
          `revert` text,
          `permission` text,
          `agent` text,
          `model` text,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          `time_busy` integer,
          `time_idle` integer,
          `time_viewed` integer,
          `idle_outcome` text,
          `time_compacting` integer,
          `time_archived` integer,
          `time_suspended` integer,
          `resume_attempts` integer DEFAULT 0 NOT NULL,
          CONSTRAINT `fk_session_v2_project_id_project_id_fk` FOREIGN KEY (`project_id`) REFERENCES `project`(`id`) ON DELETE CASCADE
        );
INSERT INTO session_v2 VALUES('ses_e57d70d7fffe0TqPcsNMhaeh6R','7f21fb1c4ceed8c54dd0f3df103168b3211dc272',NULL,NULL,NULL,NULL,'quick-cactus','@HOME@/proj','','Rich storage test','2.2.5+anysola.0.5.1',NULL,NULL,NULL,NULL,NULL,NULL,0.0,1080,378,0,7200,360,NULL,NULL,NULL,NULL,1793822618308,1793822621934,1793822619546,1793822621947,NULL,'succeeded',NULL,NULL,NULL,0);
INSERT INTO session_v2 VALUES('ses_e57d7012cffe72xpyfIsYUr33S','7f21fb1c4ceed8c54dd0f3df103168b3211dc272',NULL,'ses_e57d70d7fffe0TqPcsNMhaeh6R',NULL,NULL,'cosmic-nebula','@HOME@/proj','','List the files','2.2.5+anysola.0.5.1',NULL,NULL,NULL,NULL,NULL,NULL,0.0,120,42,0,800,40,NULL,NULL,'general',NULL,1793822621431,1793822621707,1793822621475,1793822621752,NULL,'succeeded',NULL,NULL,NULL,0);
INSERT INTO session_v2 VALUES('ses_e57d6f7a8ffen9abAz9CaEqwMp','7f21fb1c4ceed8c54dd0f3df103168b3211dc272',NULL,NULL,'ses_e57d70d7fffe0TqPcsNMhaeh6R','{"type":"through","messageID":"msg_1a82900fb002Xl7WvxvDuJCw3v"}','stellar-mountain','@HOME@/proj','','Rich storage test (fork #1)','2.2.5+anysola.0.5.1',NULL,NULL,NULL,NULL,NULL,NULL,0.0,120,42,0,800,40,NULL,NULL,NULL,'{"id":"claude-sonnet-5-5","providerID":"anysola","variant":"high"}',1793822624259,1793822625908,1793822625416,1793822625945,NULL,'succeeded',NULL,NULL,NULL,0);
INSERT INTO session_v2 VALUES('ses_e57d6e6feffe4gefNuJdeuiqXd','7f21fb1c4ceed8c54dd0f3df103168b3211dc272',NULL,NULL,NULL,NULL,'stellar-lagoon','@HOME@/proj','','Fake title','2.2.5+anysola.0.5.1',NULL,NULL,NULL,NULL,NULL,NULL,0.0,40,12,0,20,12,NULL,NULL,NULL,NULL,1793822628170,1793822634617,1793822634143,1793822634641,NULL,'succeeded',NULL,NULL,NULL,0);
CREATE TABLE `workspace` (
          `id` text PRIMARY KEY,
          `provider` text NOT NULL,
          `binding` text,
          `created_at` integer NOT NULL,
          `last_used_at` integer NOT NULL
        );
CREATE TABLE `worktree` (
          `project_id` text NOT NULL,
          `directory` text NOT NULL,
          `strategy` text,
          `time_created` integer NOT NULL,
          CONSTRAINT `worktree_pk` PRIMARY KEY(`project_id`, `directory`),
          CONSTRAINT `fk_worktree_project_id_project_id_fk` FOREIGN KEY (`project_id`) REFERENCES `project`(`id`) ON DELETE CASCADE
        );
INSERT INTO worktree VALUES('7f21fb1c4ceed8c54dd0f3df103168b3211dc272','@HOME@/proj',NULL,1793822613483);
CREATE TABLE IF NOT EXISTS "migration" (id TEXT PRIMARY KEY, time_completed INTEGER NOT NULL);
INSERT INTO migration VALUES('20260127222353_familiar_lady_ursula',1793822613166);
INSERT INTO migration VALUES('20260211171708_add_project_commands',1793822613166);
INSERT INTO migration VALUES('20260213144116_wakeful_the_professor',1793822613167);
INSERT INTO migration VALUES('20260225215848_workspace',1793822613167);
INSERT INTO migration VALUES('20260227213759_add_session_workspace_id',1793822613168);
INSERT INTO migration VALUES('20260228203230_blue_harpoon',1793822613168);
INSERT INTO migration VALUES('20260303231226_add_workspace_fields',1793822613168);
INSERT INTO migration VALUES('20260309230000_move_org_to_state',1793822613168);
INSERT INTO migration VALUES('20260312043431_session_message_cursor',1793822613168);
INSERT INTO migration VALUES('20260323234822_events',1793822613168);
INSERT INTO migration VALUES('20260410174513_workspace-name',1793822613168);
INSERT INTO migration VALUES('20260413175956_chief_energizer',1793822613168);
INSERT INTO migration VALUES('20260423070820_add_icon_url_override',1793822613168);
INSERT INTO migration VALUES('20260427172553_slow_nightmare',1793822613169);
INSERT INTO migration VALUES('20260428004200_add_session_path',1793822613170);
INSERT INTO migration VALUES('20260501142318_next_venus',1793822613170);
INSERT INTO migration VALUES('20260504145000_add_sync_owner',1793822613171);
INSERT INTO migration VALUES('20260507164347_add_workspace_time',1793822613171);
INSERT INTO migration VALUES('20260510033149_session_usage',1793822613171);
INSERT INTO migration VALUES('20260511000411_data_migration_state',1793822613172);
INSERT INTO migration VALUES('20260511173437_session-metadata',1793822613172);
INSERT INTO migration VALUES('20260601010001_normalize_storage_paths',1793822613172);
INSERT INTO migration VALUES('20260601202201_amazing_prowler',1793822613176);
INSERT INTO migration VALUES('20260602002951_lowly_union_jack',1793822613176);
INSERT INTO migration VALUES('20260602182828_add_project_directories',1793822613177);
INSERT INTO migration VALUES('20260603001617_session_message_projection_indexes',1793822613177);
INSERT INTO migration VALUES('20260603040000_session_message_projection_order',1793822613177);
INSERT INTO migration VALUES('20260603141458_session_input_inbox',1793822613177);
INSERT INTO migration VALUES('20260603160727_jittery_ezekiel_stane',1793822613177);
INSERT INTO migration VALUES('20260604172448_event_sourced_session_input',1793822613177);
INSERT INTO migration VALUES('20260605003541_add_session_context_snapshot',1793822613178);
INSERT INTO migration VALUES('20260605042240_add_context_epoch_agent',1793822613178);
INSERT INTO migration VALUES('20260611035744_credential',1793822613178);
INSERT INTO migration VALUES('20260611192811_lush_chimera',1793822613178);
INSERT INTO migration VALUES('20260612174303_project_dir_strategy',1793822613178);
INSERT INTO migration VALUES('20260622142730_simplify_session_context_epoch',1793822613179);
INSERT INTO migration VALUES('20260622170816_reset_v2_session_state',1793822613179);
INSERT INTO migration VALUES('20260622202450_simplify_session_input',1793822613179);
INSERT INTO migration VALUES('20260804233008_loose_psylocke',1793822613179);
INSERT INTO migration VALUES('20260805200742_import_legacy_credentials',1793822613179);
INSERT INTO migration VALUES('20260808023530_workspace_domain',1793822613180);
INSERT INTO migration VALUES('20260811161259_execution_claim_attempts',1793822613180);
INSERT INTO migration VALUES('20260812181746_session_inbox',1793822613180);
INSERT INTO migration VALUES('20260812213948_worktree',1793822613180);
INSERT INTO migration VALUES('20260819222447_session_viewed_state',1793822613180);
INSERT INTO migration VALUES('20260823191254_nullable_workspace_binding',1793822613180);
INSERT INTO migration VALUES('20260910120000_clear_v1_session_permission',1793822613181);
INSERT INTO migration VALUES('20260923013825_project_time_active',1793822613181);
INSERT INTO migration VALUES('20261007190000_azure_cli_external_credential',1793822613181);
INSERT INTO migration VALUES('20261010180456_project_time_archived',1793822613181);
INSERT INTO migration VALUES('20261024002554_session_allowance',1793822613181);
INSERT INTO migration VALUES('20261024175301_drop_session_allowance',1793822613182);
INSERT INTO migration VALUES('20261025123711_blob',1793822613182);
INSERT INTO migration VALUES('20261025154125_drop_session_pending',1793822613182);
INSERT INTO migration VALUES('20261026223005_schedule',1793822613182);
INSERT INTO migration VALUES('20261029002625_session_part',1793822613182);
INSERT INTO migration VALUES('20261029073502_session_time_busy',1793822613182);
INSERT INTO migration VALUES('20261029165540_schedule_checks',1793822613182);
INSERT INTO migration VALUES('20261030054400_job',1793822613188);
CREATE UNIQUE INDEX `event_aggregate_seq_idx` ON `event` (`aggregate_id`,`seq`);
CREATE INDEX `event_aggregate_type_seq_idx` ON `event` (`aggregate_id`,`type`,`seq`);
CREATE INDEX `job_session_idx` ON `job` (`session_id`);
CREATE INDEX `job_child_session_idx` ON `job` (`child_session_id`);
CREATE INDEX `job_id_idx` ON `job` (`id`);
CREATE UNIQUE INDEX `permission_project_action_resource_idx` ON `permission` (`project_id`,`action`,`resource`);
CREATE INDEX `schedule_session_idx` ON `schedule` (`session_id`);
CREATE INDEX `schedule_due_idx` ON `schedule` (`next_due_at`) WHERE "schedule"."status" = 'active' and "schedule"."next_due_at" is not null;
CREATE INDEX `blob_ref_hash_idx` ON `blob_ref` (`hash`);
CREATE INDEX `blob_ref_session_seq_idx` ON `blob_ref` (`session_id`,`seq`);
CREATE INDEX `session_inbox_session_delivery_seq_idx` ON `session_inbox` (`session_id`,`delivery`,`enqueued_seq`);
CREATE UNIQUE INDEX `session_inbox_session_enqueued_seq_idx` ON `session_inbox` (`session_id`,`enqueued_seq`);
CREATE UNIQUE INDEX `session_message_session_seq_idx` ON `session_message` (`session_id`,`seq`);
CREATE INDEX `session_message_session_type_seq_idx` ON `session_message` (`session_id`,`type`,`seq`);
CREATE INDEX `session_message_session_time_created_id_idx` ON `session_message` (`session_id`,`time_created`,`id`);
CREATE INDEX `session_message_time_created_idx` ON `session_message` (`time_created`);
CREATE INDEX `session_part_session_message_idx` ON `session_part` (`session_id`,`message_id`,`ordinal`);
CREATE INDEX `session_v2_project_idx` ON `session_v2` (`project_id`);
CREATE INDEX `session_v2_workspace_idx` ON `session_v2` (`workspace_id`);
CREATE INDEX `session_v2_parent_idx` ON `session_v2` (`parent_id`);
CREATE INDEX `session_v2_time_suspended_idx` ON `session_v2` (`time_suspended`) WHERE "session_v2"."time_suspended" is not null;
COMMIT;
