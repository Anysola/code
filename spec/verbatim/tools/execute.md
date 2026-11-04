# execute

Source: packages/core/src/codemode/tool.ts, packages/core/src/codemode/catalog.ts, packages/core/src/codemode/instructions.ts, packages/codemode/src (acorn-based interpreter)

Captured from the 0.5.1 binary's `/v1/code/messages` request (build agent, anysola/claude-sonnet-5-5, Linux). Ground truth: `_captured-request-build-sonnet.json`.

## Description (verbatim, as sent)

````text
Run JavaScript in a confined Code Mode runtime to script tool calls and HTTP requests and compose their results.
`fetch` is available for HTTP requests. Imports, direct filesystem access, and timers are unavailable: there is no `setTimeout` or sleep. All other external access goes through `tools`.
Within `{ code }`, the only callable tools are those explicitly listed in the Code Mode catalog instructions or returned by the `search` function. Inside `{ code }`, ignore tools shown outside the Code Mode catalog. They are not available in the Code Mode runtime.
Call tools through `tools` using only exact paths and signatures from the catalog. Do not infer or normalize tool names; preserve bracket notation such as `tools.<namespace>["tool-name"](input)`.
Prefer an explicit `return`; if omitted, the final top-level expression becomes the result.
Await every call whose completion matters; pending calls are interrupted when execution ends. Run independent calls concurrently with `Promise.allSettled`.
````

## input_schema (verbatim, as sent)

```json
{
  "type": "object",
  "properties": {
    "code": {
      "type": "string"
    }
  },
  "required": [
    "code"
  ],
  "additionalProperties": false
}
```

Extra key on this tool entry: `"cache_control": {"type": "ephemeral"}` (it is the last tool in the array).

## Variants

The description is static (codemode/tool.ts:75-82). The tool only appears when `execute` is not wholly denied and at
least... (it is always appended last in `tools`, after the alphabetically sorted direct tools, tool.ts:310-315) and the
last tool entry carries `cache_control: {type: ephemeral}`. Without Code Mode (execute denied) the namespaced tools are
exposed as direct tools named `opencode_<tool>` (tool.ts:276-278, runtime.ts:289-292).
The catalog lives in the SYSTEM prompt (block 2), not in the tool description.

## Code Mode catalog rendering (catalog.ts:47-176)

- Namespaces sorted by name; line `- <ns> (<N> tools[, <k> shown | , none shown]) // <namespace description>`.
- Each tool line `  - <signature> // <first description line, cut to 117 chars + "..." if longer than 120>`; tools sorted by path.
- Inline budget 2000 tokens (4 chars per token); full listings picked shortest-first round-robin per namespace until the budget is spent; the rest are reachable via `search`.

### Catalog as sent (default config, 8 tools)

````text
# Code Mode

The tools listed below cannot be called directly: they only work as JavaScript inside the `execute` tool. Every other tool you have is a normal tool call: call it directly, never inside `execute`.

The catalog is complete. Do not guess tool names.

## Code Mode tools (call only inside `execute`)

- opencode (8 tools) // Tools for managing OpenCode itself, such as working with sessions and worktrees, searching the available models, and reading MCP resources.
  - tools.opencode.list_mcp_resources({
  /** MCP server name as configured. Omit to list every server. */
  server?: string,
}): Promise<{
  resources: Array<{
      server: string,
      name: string,
      uri: string,
      description?: string,
      mimeType?: string,
    }>,
  templates: Array<{
      server: string,
      name: string,
      uriTemplate: string,
      description?: string,
      mimeType?: string,
    }>,
}> // List documents, records, and other data exposed by MCP servers. Use this when the user refers to something that is no...
  - tools.opencode.models({
  /** Text to search for in model names and IDs. */
  query?: string,
  /** Provider ID or name to filter by. Try your own provider first. */
  provider?: string,
  /**
   * Which kind of model to list. Defaults to "chat".
   * "chat": a language model (LLM). Powers sessions, agents and subagents, and can call tools.
   * "image": generates or edits images from a prompt. It can't chat or call tools.
   */
  type?: "chat" | "image",
  /** Include older versions of each model family. By default only the newest version is listed. */
  all?: boolean,
  /** Maximum number of models to return. Defaults to 20. @integer @minimum 1 @maximum 100 */
  limit?: number,
  /** Number of models to skip, for paging through results. @integer @minimum 0 */
  offset?: number,
}): Promise<{
  /** Matching models grouped by provider. Your own provider comes first. */
  providers: Array<{
      id: string,
      name: string,
      /** Newest first. */
      models: Array<{
          /** providerID/modelID */
          id: string,
          name: string,
          /** Release date as a Unix timestamp in milliseconds, or 0 when unknown. */
          released: number,
          variants: Array<string>,
          /** Pricing in USD per million tokens. */
          cost: Array<{
              tier?: {
                type: "context",
                /** @integer */
                size: number,
              },
              input: number,
              output: number,
              cache: {
                read: number,
                write: number,
              },
            }>,
          status: "alpha" | "beta" | "deprecated" | "active",
        }>,
    }>,
  /** Number of matching models across all pages. @integer */
  total: number,
  /** Offset of the next page, or null on the last page. */
  next: number | null,
}> // Search the models available to use. Use this to turn a model name the user mentions into an exact reference for a too...
  - tools.opencode.read_mcp_resource({
  /** The server field of the discovered resource. */
  server: string,
  /** Exact resource URI from discovery, an expanded uriTemplate, or the user. */
  uri: string,
}): Promise<{
  server: string,
  uri: string,
  contents: Array<{
        type: "text",
        uri: string,
        text: string,
        mimeType?: string,
      } | {
        type: "blob",
        uri: string,
        blob: string,
        mimeType?: string,
      }>,
}> // Read one MCP resource by server and URI. Not for local files. Always return the full contents rather than slicing or ...
  - tools.opencode.session_move({
  /** Omit to move the current session. @pattern ^ses */
  sessionID?: string,
  /** Destination directory, relative to the target session's directory or absolute. Supports ~. @minLength 1 */
  directory: string,
}): Promise<{
  /** @pattern ^ses */
  sessionID: string,
  directory: string,
}> // Move a session to another directory, or omit sessionID to move the current session. The current session moves at the ...
  - tools.opencode.session_rename({
  /** Omit to rename the current session. @pattern ^ses */
  sessionID?: string,
  /** New session title. @minLength 1 */
  title: string,
}): Promise<{
  /** @pattern ^ses */
  sessionID: string,
  title: string,
}> // Rename a session, or omit sessionID to rename the current session. Use a short, specific title that summarizes the wo...
  - tools.opencode.worktree_create({
  name: string,
}): Promise<{
  directory: string,
}> // Create a named worktree. Returns its directory.
  - tools.opencode.worktree_list(): Promise<{
  worktrees: Array<{
      directory: string,
      strategy?: string,
    }>,
}> // List a repository's worktrees.
  - tools.opencode.worktree_remove({
  /** Absolute path of the worktree to remove. @minLength 1 */
  directory: string,
  /** Discard modified and untracked files. Only set this when the user has confirmed forced removal. */
  force?: boolean,
}): Promise<{
  directory: string,
  removed: boolean,
}> // Remove a worktree. The repository location selects configuration; directory identifies the worktree to remove.
````

### Catalog as sent with experimental.todo/schedules/artifacts on (17 tools, 14 shown)

See `_captured-request-build-experimental-flags.json` system block; header line:
`- opencode (17 tools, 14 shown) // Tools for managing OpenCode itself, such as working with sessions and worktrees, searching the available models, and reading MCP resources.`

## Full descriptions of the opencode namespace tools (source; the catalog cuts them)

- session_rename: `Rename a session, or omit sessionID to rename the current session. Use a short, specific title that summarizes the work being done.` (opencode.ts:118-120)
- session_move: `Move a session to another directory, or omit sessionID to move the current session. The current session moves at the next safe boundary; do not run destination-dependent tools in the same execute call.` (opencode.ts:138-140)
- worktree_create: `Create a named worktree. Returns its directory.` (input name: "Worktree name.")
- worktree_list: `List a repository's worktrees.`
- worktree_remove: `Remove a worktree. The repository location selects configuration; directory identifies the worktree to remove.`
- models: `Search the models available to use. Use this to turn a model name the user mentions into an exact reference for a tool that takes a model. Check your own provider first.`
- list_mcp_resources: `List documents, records, and other data exposed by MCP servers. Use this when the user refers to something that is not a local file, such as a URI with a custom scheme, then load a match with read_mcp_resource. Each entry names the server to read it from. Templates are resources addressed by a parameter such as a record ID; fill in the uriTemplate placeholders before reading.` (mcp-resource.ts:19-22)
- read_mcp_resource: `Read one MCP resource by server and URI. Not for local files. Always return the full contents rather than slicing or filtering them; oversized output is truncated automatically and the full content is saved to a file you can read. Images and PDFs are shown to you directly.` (mcp-resource.ts:51-54)
- namespace `opencode`: `Tools for managing OpenCode itself, such as working with sessions and worktrees, searching the available models, and reading MCP resources.`
Signatures/schemas: as rendered in the catalog above.

## Model-visible output templates (codemode/tool.ts ~400-420)

Result = JSON.stringify(value, null, 2) (or the string itself), then `\n\nLogs:\n<console lines>` if anything was logged;
errors are returned as ordinary (NOT is_error) text: message plus suggestions, e.g. unknown tool. Inner tool failures
are thrown into the program with the same message the model would see for a direct call.

### Captured

return await tools.opencode.worktree_list() with console.log('hey'):
````text
{
  "worktrees": [
    {
      "directory": "/home/ubuntu/anysola-code/rust/testing/runs/tools/proj"
    }
  ]
}

Logs:
hey
````

tools.opencode.nope():
````text
Unknown tool 'opencode.nope'.
Use search to find available tools.
````

search('worktree') (wrong arg type):
````text
Invalid input for tool 'search': SchemaError(Expected object)
````
