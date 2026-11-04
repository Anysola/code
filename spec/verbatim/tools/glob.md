# glob

Source: packages/core/src/tool/plugin/glob.ts, packages/core/src/ripgrep.ts:163-190

Captured from the 0.5.1 binary's `/v1/code/messages` request (build agent, anysola/claude-sonnet-5-5, Linux). Ground truth: `_captured-request-build-sonnet.json`.

## Description (verbatim, as sent)

````text
Search file paths using a glob pattern (examples: "**/*.ts", "src/**/*.tsx").
````

## input_schema (verbatim, as sent)

```json
{
  "type": "object",
  "properties": {
    "pattern": {
      "type": "string",
      "description": "Glob pattern to match files against"
    },
    "path": {
      "type": "string",
      "description": "Directory to search. Defaults to the current working directory."
    },
    "hidden": {
      "type": "boolean",
      "description": "Include hidden files and directories (default: false)."
    },
    "limit": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "description": "Maximum number of matching files to return (default: 100)"
    }
  },
  "required": [
    "pattern"
  ],
  "additionalProperties": false
}
```

## Variants

None (static; default 100 from FileSystem.DEFAULT_SEARCH_LIMIT).

## Model-visible output templates (glob.ts:35-42)

- one absolute path per line, in ripgrep `--files` output order (not sorted, not mtime)
- none: `No files found`
- truncated: blank line + `(Results are truncated: showing first ${n} results. Consider using a more specific path or pattern.)`
- errors: `Search path does not exist: ${path}`; `Search path is not a directory: ${path}`; timeout text as grep; fallback `Unable to find files matching ${pattern}`.
- `path` values "undefined"/"null" (strings) are treated as omitted (glob.ts:66).

Error envelope (all tools): a failing call reaches the model as a tool_result with `is_error: true` and content
`{"error":{"type":"<type>","message":"<message>"},"content":[]}` (JSON, no spaces), built in
packages/core/src/session/runner/to-llm-message.ts:143-150; `<type>`/`<message>` come from
packages/core/src/session/to-session-error.ts (ToolFailure with an inner error -> the inner error's message wins;
Permission.BlockedError/CorrectedError -> type `permission.rejected`).

### Captured

glob **/*.ts:
````text
/home/ubuntu/anysola-code/rust/testing/runs/tools/proj/src/x.ts
````

glob **/*.ts hidden:true:
````text
/home/ubuntu/anysola-code/rust/testing/runs/tools/proj/src/x.ts
/home/ubuntu/anysola-code/rust/testing/runs/tools/proj/.hid/h.ts
````

glob *.none:
````text
No files found
````

glob **/* limit:1 (note: a positive --glob that matches the ignored directory overrides .gitignore):
````text
/home/ubuntu/anysola-code/rust/testing/runs/tools/proj/ign/i.ts

(Results are truncated: showing first 1 results. Consider using a more specific path or pattern.)
````

glob path a.txt:
is_error: true
````text
{"error":{"type":"tool.execution","message":"Search path is not a directory: a.txt"},"content":[]}
````
