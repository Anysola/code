# todowrite

Source: packages/core/src/tool/plugin/todo.ts, packages/core/src/tool/todowrite.txt

Conditional: only registered when config `experimental.todo === true` (todo.ts:50); NOT sent by default in 0.5.1. Captured from `_captured-request-build-experimental-flags.json`.

## Description (verbatim)

````text
Create and update a todo list for the current session. The user sees it as your working plan.

Use it proactively when a task has 3 or more distinct steps, or when the user gives several tasks or asks for a todo list. Skip it for a single simple task or a purely conversational request. When in doubt, use it.

- Send the full list on every call; it replaces the previous one.
- Mark items `in_progress` when you start them and `completed` as soon as they're actually done.
- Only call it when the list changes.

````

## input_schema (verbatim)

```json
{
  "type": "object",
  "properties": {
    "todos": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "content": {
            "type": "string",
            "description": "Brief description of the task"
          },
          "status": {
            "type": "string",
            "enum": [
              "pending",
              "in_progress",
              "completed",
              "cancelled"
            ],
            "description": "Current status of the task: pending, in_progress, completed, cancelled"
          }
        },
        "required": [
          "content",
          "status"
        ],
        "additionalProperties": false
      },
      "description": "The updated todo list"
    }
  },
  "required": [
    "todos"
  ],
  "additionalProperties": false
}
```

## Model-visible output templates (todo.ts:30-41)

- empty list: `Todos cleared`
- otherwise `Todos updated: ${completed}/${total} completed[, ${n} in progress][, ${n} pending][, ${n} cancelled][. In progress: "${content}", ...]`
- permission failure wrapper: `Unable to update todos` (inner message wins)
