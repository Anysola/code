# edit

Source: packages/core/src/tool/plugin/edit.ts, packages/core/src/tool/plugin/file-diff.ts, packages/core/src/file-mutation.ts, packages/core/src/file-access.ts, packages/util/src/bom.ts, packages/core/src/formatter.ts

Captured from the 0.5.1 binary's `/v1/code/messages` request (build agent, anysola/claude-sonnet-5-5, Linux). Ground truth: `_captured-request-build-sonnet.json`.

## Description (verbatim, as sent)

````text
Edit the contents of a file by finding and replacing exact text. When editing text from Read output, preserve the exact indentation (tabs or spaces) and omit the line-number prefix, such as `1: `. Never include the prefix in oldString or newString. Write line breaks as \n; the file's existing CRLF or LF line endings are matched and preserved. To change line endings themselves, make oldString and newString differ only in \r; that edit applies to the exact bytes. If there is no exact match, the edit retries while ignoring typographic quote, dash, and space variants, then trailing whitespace on each line. The edit fails if oldString is not found. By default, oldString must identify a UNIQUE location. Multiple matches FAIL unless replaceAll is true. Add more surrounding context to disambiguate, or set replaceAll to true to replace every occurrence. Use replaceAll when the change should apply to every occurrence, such as renaming a variable.
````

## input_schema (verbatim, as sent)

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "File to edit"
    },
    "oldString": {
      "type": "string",
      "description": "Exact text to find and replace"
    },
    "newString": {
      "type": "string",
      "description": "Text to replace oldString with (must differ from oldString)"
    },
    "replaceAll": {
      "type": "boolean",
      "description": "Whether to replace every occurrence of oldString. When false, oldString must match exactly once. Defaults to false."
    }
  },
  "required": [
    "path",
    "oldString",
    "newString"
  ],
  "additionalProperties": false
}
```

## Model-visible output templates

Success (`edit.ts:381`): `Edited ${file} (${n} replacement${n === 1 ? "" : "s"})` where `file` is the location-relative resource (absolute with `/` for external paths). Captured:

```text
Edited e.ts (1 replacement)
Edited dup.txt (2 replacements)
```

Errors (error envelope as in read.md; all captured unless marked):

```text
{"error":{"type":"tool.execution","message":"No changes to apply: oldString and newString are identical."},"content":[]}
{"error":{"type":"tool.execution","message":"oldString must not be empty. Use write to create or overwrite a file."},"content":[]}
{"error":{"type":"tool.execution","message":"File not found: none.txt"},"content":[]}
{"error":{"type":"tool.execution","message":"Path is a directory, not a file: sub"},"content":[]}
{"error":{"type":"tool.execution","message":"Could not find oldString in e.ts. It must match exactly, including whitespace and indentation."},"content":[]}
{"error":{"type":"tool.execution","message":"Could not find oldString in e.ts. It must match exactly, including whitespace and indentation.\n\nThe closest lines in the file:\n2: const y = 1"},"content":[]}
{"error":{"type":"tool.execution","message":"Could not find oldString in e.ts. It must match exactly, including whitespace and indentation.\n\nnewString is already in the file, so this edit may already be applied."},"content":[]}
{"error":{"type":"tool.execution","message":"Found 2 matches for oldString, but expected exactly one. Add more surrounding context to make oldString unique, or set replaceAll to true to replace every occurrence."},"content":[]}
{"error":{"type":"tool.execution","message":"Cannot use edit to modify files outside the Plan directory: <home>/.opencode/plan"},"content":[]}
```

Source templates:
- `No changes to apply: oldString and newString are identical.` (`edit.ts:288`)
- `oldString must not be empty. Use write to create or overwrite a file.` (`edit.ts:293`)
- `File not found: ${input.path}` (`edit.ts:302`)
- `Path is a directory, not a file: ${input.path}` (`edit.ts:306`)
- `Unable to edit ${input.path}` (other kinds / unexpected errors; if a wrapped cause has a non-empty message the model sees that instead) (`edit.ts:307,387`)
- `Could not find oldString in ${input.path}. It must match exactly, including whitespace and indentation.` + optional `\n\n${hint}` (`edit.ts:349`)
  - hint A: `newString is already in the file, so this edit may already be applied.` (when `newString.trim().length >= 20` and normalized file contains normalized newString) (`edit.ts:202-203`)
  - hint B: `The closest lines in the file:` then up to 12 lines `${n}: ${line}`, each line cut at 200 chars + `…` (U+2026) (`edit.ts:190-240`)
  - hint only when `read` of that file is already allowed by rules (never asks) (`edit.ts:339-347`)
- `Found ${n} matches for oldString, but expected exactly one. Add more surrounding context to make oldString unique, or set replaceAll to true to replace every occurrence.` (`edit.ts:354`)
- `No changes to apply: ${input.path} already has those line endings.` (endings-only edit that changes nothing) (`edit.ts:360`)
- `No changes to apply: the edit leaves ${input.path} unchanged. Line endings follow the file; to change them, write \r explicitly in oldString or newString.` (literal backslash-r in the text) (`edit.ts:361`)
- Plan agent: `Cannot use ${tool} to modify files outside the Plan directory: ${directory}` (`packages/core/src/plugin/plan.ts:49-51`)
- Permission blocked by config deny: `Permission denied: edit` (or the rule message) with type `permission.rejected` (`permission.ts:76`).
- Non-interactive `run` ask: same permission.rejected text as in read.md.

## Variants

None: static description/schema (`edit.ts:24-34,276`); identical across the three Anysola models and plan agent. For model ids containing `gpt-` (not `oss`, not `gpt-4`) `edit` and `write` are removed in favour of `patch` (`patch.ts` hook); never true for Anysola's models.
