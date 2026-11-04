# skill

Source: packages/core/src/tool/plugin/skill.ts, packages/core/src/skill.ts (toModelOutput/prepare), packages/core/src/skill/discovery.ts

Captured from the 0.5.1 binary's `/v1/code/messages` request (build agent, anysola/claude-sonnet-5-5, Linux). Ground truth: `_captured-request-build-sonnet.json`.

## Description (verbatim, as sent)

````text
Load a specialized skill's instructions and resources into the current conversation when the task at hand matches its description.

The skill ID must match an available skill or a skill explicitly referenced by the user.
````

## input_schema (verbatim, as sent)

```json
{
  "type": "object",
  "properties": {
    "id": {
      "type": "string",
      "description": "The ID of an available skill or a skill explicitly referenced by the user"
    }
  },
  "required": [
    "id"
  ],
  "additionalProperties": false
}
```

## Variants

Description static. The list of skills is NOT in the tool description: it is in system block 2 as `<available_skills>` (session/system-prompt area).

## Model-visible output templates

- success: `<skill_content name="${name}">\n# Skill: ${name}\n\n${body}\n\nBase directory for this skill: ${dir}\nRelative paths in this skill (e.g., scripts/, reference/) are relative to this base directory.\nNote: file list is sampled.\n\n<skill_files>\n<file>${abs}</file>...\n</skill_files>\n</skill_content>` (at most 10 files, skill.ts:46,68)
- unknown/denied: `Unable to load skill ${id}`

Error envelope (all tools): a failing call reaches the model as a tool_result with `is_error: true` and content
`{"error":{"type":"<type>","message":"<message>"},"content":[]}` (JSON, no spaces), built in
packages/core/src/session/runner/to-llm-message.ts:143-150; `<type>`/`<message>` come from
packages/core/src/session/to-session-error.ts (ToolFailure with an inner error -> the inner error's message wins;
Permission.BlockedError/CorrectedError -> type `permission.rejected`).

### Captured (proj/.claude/skills/demo/SKILL.md + notes.txt)

````text
<skill_content name="demo">
# Skill: demo

Do the demo thing.

Base directory for this skill: /home/ubuntu/anysola-code/rust/testing/runs/tools/proj/.claude/skills/demo
Relative paths in this skill (e.g., scripts/, reference/) are relative to this base directory.
Note: file list is sampled.

<skill_files>
<file>/home/ubuntu/anysola-code/rust/testing/runs/tools/proj/.claude/skills/demo/notes.txt</file>
</skill_files>
</skill_content>
````

missing:
is_error: true
````text
{"error":{"type":"tool.execution","message":"Unable to load skill missing"},"content":[]}
````
