# grep

Source: packages/core/src/tool/plugin/grep.ts, packages/core/src/ripgrep.ts, packages/core/src/ripgrep/binary.ts, packages/core/src/filesystem.ts:66-67

Captured from the 0.5.1 binary's `/v1/code/messages` request (build agent, anysola/claude-sonnet-5-5, Linux). Ground truth: `_captured-request-build-sonnet.json`.

## Description (verbatim, as sent)

````text
Search file contents using ripgrep's regular expression syntax or literal text matching. Use it to locate specific code, symbols, or text patterns, and narrow searches with `path` or `include`. Returns matching file paths, line numbers, and line previews.
````

## input_schema (verbatim, as sent)

```json
{
  "type": "object",
  "properties": {
    "pattern": {
      "type": "string",
      "minLength": 1,
      "description": "Regular expression or literal text to match in file contents."
    },
    "path": {
      "type": "string",
      "description": "File or directory to search. Defaults to the current working directory."
    },
    "include": {
      "type": "string",
      "description": "Glob pattern to filter files (for example, \"*.js\" or \"*.{ts,tsx}\")"
    },
    "literal": {
      "type": "boolean",
      "description": "Treat `pattern` as exact text instead of a regular expression (default: false)."
    },
    "caseSensitive": {
      "type": "boolean",
      "description": "Use case-sensitive matching (default: true)."
    },
    "limit": {
      "type": "integer",
      "exclusiveMinimum": 0,
      "description": "Maximum number of matching lines to return (default: 100)"
    }
  },
  "required": [
    "pattern"
  ],
  "additionalProperties": false
}
```

## Variants

None. Description and schema are static; `(default: 100)` comes from FileSystem.DEFAULT_SEARCH_LIMIT (filesystem.ts:66).

## Model-visible output templates (grep.ts:44-61)

- no matches: `No matches found`
- header: `Found ${n} matches` (always "matches", even for 1)
- per file: `${absolutePath}:` then one `  Line ${line}: ${text}` per match; a blank line between files.
  `text` is rg's `lines.text`, which INCLUDES the line's trailing newline, so each match line is followed by an extra blank line (quirk, keep it).
  Lines longer than 2000 chars are cut to 2000 + `...` (ripgrep.ts:21,247).
- truncated (more than `limit` matches): blank line + `(Results are truncated: showing first ${n} results. Consider using a more specific path or pattern.)`
- errors: `Search path does not exist: ${path ?? "."}`; `Invalid regex pattern: ${rg stderr trimmed}`;
  `Search timed out after 30 seconds. Consider using a more specific path or pattern.`; fallback `Unable to grep for ${pattern}` (only shown when the inner error has no message).

Error envelope (all tools): a failing call reaches the model as a tool_result with `is_error: true` and content
`{"error":{"type":"<type>","message":"<message>"},"content":[]}` (JSON, no spaces), built in
packages/core/src/session/runner/to-llm-message.ts:143-150; `<type>`/`<message>` come from
packages/core/src/session/to-session-error.ts (ToolFailure with an inner error -> the inner error's message wins;
Permission.BlockedError/CorrectedError -> type `permission.rejected`).

### Captured (proj has src/x.ts with "foo one/bar/foo two", .hid/h.ts, gitignored ign/i.ts)

grep {pattern:"foo"}:
````text
Found 3 matches
/home/ubuntu/anysola-code/rust/testing/runs/tools/proj/src/x.ts:
  Line 1: foo one

  Line 3: foo two


/home/ubuntu/anysola-code/rust/testing/runs/tools/proj/.hid/h.ts:
  Line 1: foo

````

grep {pattern:"zzz"}:
````text
No matches found
````

grep {pattern:"foo",limit:1}:
````text
Found 1 matches
/home/ubuntu/anysola-code/rust/testing/runs/tools/proj/src/x.ts:
  Line 1: foo one


(Results are truncated: showing first 1 results. Consider using a more specific path or pattern.)
````

grep {pattern:"(",path:"src"}:
is_error: true
````text
{"error":{"type":"tool.execution","message":"Invalid regex pattern: rg: regex parse error:\n    (?:()\n    ^\nerror: unclosed group"},"content":[]}
````

grep {pattern:"foo",path:"nope"}:
is_error: true
````text
{"error":{"type":"tool.execution","message":"Search path does not exist: nope"},"content":[]}
````
