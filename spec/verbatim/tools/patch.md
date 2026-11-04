# patch

Source: packages/core/src/tool/plugin/patch.ts, packages/core/src/tool/patch.txt, packages/util/src/patch.ts, packages/util/src/bom.ts

Status: [dead for Anysola 0.5.1] registered always, but a session `context`/`compaction`/`generate` hook removes it unless the model id contains `gpt-` and contains neither `oss` nor `gpt-4` (`patch.ts:331-342`); in that case `edit` and `write` are removed instead. Anysola models (`auto`, `claude-sonnet-5-5`, `claude-haiku-5-5`) never select it; the captured requests do not contain `patch`. Permission action: `edit`. codemode: false.

## Description (verbatim from tool/patch.txt; note U+2011 non-breaking hyphens in "stripped‑down" and "high‑level"; file ends with a newline)

````text
Use the `patch` tool to edit files. Your patch language is a stripped‑down, file‑oriented diff format designed to be easy to parse and safe to apply. You can think of it as a high‑level envelope:

*** Begin Patch
[ one or more file sections ]
*** End Patch

Within that envelope, you get a sequence of file operations.
You MUST include a header to specify the action you are taking.
Each operation starts with one of three headers:

*** Add File: <path> - create a new file. Every following line is a + line (the initial contents).
*** Delete File: <path> - remove an existing file. Nothing follows.
*** Update File: <path> - patch an existing file in place (optionally with a rename).

Example patch:

```
*** Begin Patch
*** Add File: hello.txt
+Hello world
*** Update File: src/app.py
*** Move to: src/main.py
@@ def greet():
-print("Hi")
+print("Hello, world!")
*** Delete File: obsolete.txt
*** End Patch
```

It is important to remember:

- You must include a header with your intended action (Add/Delete/Update)
- You must prefix new lines with `+` even when creating a new file
````

## input_schema (derived from source, not captured)

```json
{
  "type": "object",
  "properties": {
    "patchText": {
      "type": "string",
      "description": "The full patch text describing add, update, and delete operations"
    }
  },
  "required": [
    "patchText"
  ],
  "additionalProperties": false
}
```

## Model-visible output templates

Success (`patch.ts:39-45`):

```text
Success. Updated the following files:
A <resource>
M <resource>
D <resource>
```

(`A` add, `M` update, `D` delete; for a move the resource is the destination.)

Errors (`patch.ts`, `util/src/patch.ts`):
- `patchText is required`
- `patch verification failed: ${parseError.message}` where message is one of:
  - `The first line of the patch must be '*** Begin Patch'` / `The last line of the patch must be '*** End Patch'`
  - `Invalid hunk at line ${n}: '${line}' is not a valid hunk header. Valid hunk headers: '*** Add File: {path}', '*** Delete File: {path}', '*** Update File: {path}'`
  - `Invalid hunk at line ${n}: ${reason}` with reasons: `Unexpected line after Delete File '${path}': '${next}'. Delete hunks do not contain body lines`; `Move destination for '${path}' must not be empty`; `Invalid Add File line for '${path}': expected a line starting with '+', got '${line}'`; `Expected update hunk to start with a @@ context marker, got: '${line}'`; `Update hunk does not contain any lines`; `Unexpected line found in update hunk: '${line}'. Every line should start with ' ' (context line), '+' (added line), or '-' (removed line)`; `Update file hunk for path '${path}' is empty`
- `patch rejected: empty patch`
- `patch verification failed: Failed to delete ${resource}: file does not exist` / `...: ${err}`
- `patch verification failed: Failed to read file to update ${absolute}: file does not exist` / `...: ${err}`
- `patch verification failed: Failed to find context '${ctx}' in ${path}`
- `patch verification failed: Failed to find an expected blank line in ${path}` / `Failed to find ${n} consecutive blank lines in ${path}`
- `patch verification failed: Failed to find expected lines in ${path}:\n${oldLines}`
- `Unable to prepare patch at ${hunk.path}`, `Unable to apply patch`
- Apply-phase: `${operation}: ${error}` + optional `. Completed before failure: ${resources}` with operations `Failed to delete X`, `Failed to write X`, `Wrote X but failed to remove Y`, `Failed to read X`, `Failed to sync X`; error texts `file does not exist`, `path is a directory`, `path is ${kind}`.

## Variants

Only the selection rule above.
