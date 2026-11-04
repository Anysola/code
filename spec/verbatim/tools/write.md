# write

Source: packages/core/src/tool/plugin/write.ts, packages/core/src/file-mutation.ts, packages/core/src/file-access.ts, packages/util/src/bom.ts, packages/core/src/formatter.ts

Captured from the 0.5.1 binary's `/v1/code/messages` request (build agent, anysola/claude-sonnet-5-5, Linux). Ground truth: `_captured-request-build-sonnet.json`.

## Description (verbatim, as sent)

````text
Writes a file to the local filesystem, overwriting if one exists.

Missing parent directories are created automatically.

Use this tool to create new files or overwrite existing files. For partial changes, use the edit tool instead.
````

## input_schema (verbatim, as sent)

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Path to the file to write to"
    },
    "content": {
      "type": "string",
      "description": "Content to write to the file"
    }
  },
  "required": [
    "path",
    "content"
  ],
  "additionalProperties": false
}
```

## Model-visible output templates

Success (`write.ts:38-39`): `${existed ? "Wrote" : "Created"} file successfully: ${resource}`. Captured:

```text
Created file successfully: new/deep/n.txt
Wrote file successfully: a.txt
Created file successfully: /home/ubuntu/anysola-code/rust/testing/runs/tools-a/outside/o.txt
```

Errors:

```text
{"error":{"type":"tool.execution","message":"Unable to write sub"},"content":[]}
{"error":{"type":"tool.execution","message":"Cannot use write to modify files outside the Plan directory: <home>/.opencode/plan"},"content":[]}
```

- Every failure is wrapped as `Unable to write ${input.path}` (`write.ts:95`); when the wrapped cause carries its own non-empty message the model sees that message instead (e.g. permission block -> `Permission denied: edit`, type `permission.rejected`; decline in non-interactive run -> the permission.rejected text in read.md).

## Variants

None (static, `write.ts:23-28,61`). Removed for `gpt-` model ids (patch selection), never for Anysola models.
