# subagent

Source: packages/core/src/tool/plugin/subagent.ts, packages/core/src/session/subagent-completion.ts, packages/core/src/plugin/agent.ts

Captured from the 0.5.1 binary's `/v1/code/messages` request (build agent, anysola/claude-sonnet-5-5, Linux). Ground truth: `_captured-request-build-sonnet.json`.

## Description (verbatim, as sent)

````text
Spawns an agent in a child session to work on the specified task.
The output includes a sessionID you can pass back later to continue that specific conversation with the subagent.
New child sessions start with fresh context, so include all relevant context and instructions when you don't pass a sessionID.
Foreground (default) runs the subagent to completion and returns its final response.
Background mode (background=true) launches it asynchronously and returns immediately; you are notified when it finishes.
Use background only for independent work that can run while you continue elsewhere.

Available subagents:
- explore: Fast agent specialized for exploring codebases. Use this when you need to quickly find files by patterns (eg. "src/components/**/*.tsx"), search code for keywords (eg. "API endpoints"), or answer questions about the codebase (eg. "how do API endpoints work?"). When calling this agent, specify the desired thoroughness level: "quick" for basic searches, "medium" for moderate exploration, or "very thorough" for comprehensive analysis across multiple locations and naming conventions.
- general: General-purpose agent for researching complex questions and executing multi-step tasks. Use this agent to execute multiple units of work in parallel.
````

## input_schema (verbatim, as sent)

```json
{
  "type": "object",
  "properties": {
    "agent": {
      "type": "string",
      "description": "The type of specialized agent to use for this task. If the user asks for a subagent by a name that is not one of the available subagents, they most likely mean a model: pick a suitable agent and pass the name through the model parameter instead."
    },
    "description": {
      "type": "string",
      "description": "A short 3-5 word label for the task, displayed to the user"
    },
    "prompt": {
      "type": "string",
      "description": "The task for the subagent to perform"
    },
    "model": {
      "type": "string",
      "description": "NEVER set this unless the user explicitly asks for a particular model or variant. It must be a chat model; image and decision models can't run subagents. The value is written as \"providerID/modelID\", or \"providerID/modelID#variant\" to include a variant. Do not guess the ID: look the model up with the models tool, filtering to your own provider first."
    },
    "sessionID": {
      "type": "string",
      "pattern": "^ses",
      "description": "Send a follow-up to an existing subagent, running or finished. Omit to start a new one."
    },
    "background": {
      "type": "boolean",
      "description": "Run the subagent in the background and return immediately. You will be notified when it completes. DO NOT sleep, poll, or proactively check on its progress."
    }
  },
  "required": [
    "agent",
    "description",
    "prompt"
  ],
  "additionalProperties": false
}
```

## Variants

`Available subagents:` block is appended by a session context hook (subagent.ts:286-312): agents with mode != primary,
not hidden, and `Permission.evaluate("subagent", agent.id, callerAgent.permissions) != deny`, sorted by id, line
`- ${id}: ${description ?? "This subagent should only be called when explicitly requested."}`. Omitted entirely if none.
Default: explore, general (descriptions in plugin/agent.ts:65-77). Custom agents from config/markdown join the list.

## Model-visible output templates

- foreground completed: `<subagent sessionID="${id}" state="completed">\n${text}\n</subagent>` (text = child's last assistant text parts joined by blank line, or `Subagent completed without a text response.`)
- background (immediate): 4 lines joined by \n (subagent.ts:19-28):
  `The subagent is working in the background (sessionID: ${id}). You will be notified automatically when it finishes.`
  `DO NOT sleep, poll for progress, ask the subagent for status, or duplicate this subagent's work; avoid working with the same files or topics it is using.`
  `Work on non-overlapping tasks, or briefly tell the user what you launched and end your response.`
  `To send it follow-ups or change its instructions while it runs, call subagent again with this sessionID.`
- background completion notice (synthetic user message into the parent): `<subagent sessionID="${id}" state="${completed|error|cancelled}" description="${description}">\n${text}\n</subagent>` with text = output / error ?? `Subagent failed` / `Subagent cancelled` (subagent-completion.ts:30-43)
- errors: `Unknown agent: ${agent}`; `Agent ${agent} cannot run as a subagent`; `Subagent depth limit reached (${limit}). Increase "experimental.subagent_depth" to allow nested subagents.` (default limit 1);
  `Subagent denied: ${id}`; `Subagent session not found: ${sessionID}`; `Session ${id} is not a child of the current session`;
  `Subagent failed (sessionID: ${id}): ${error ?? "unknown error"}`; `Subagent cancelled (sessionID: ${id})`;
  model errors: `Invalid model "${input}". Use "providerID/modelID" or "providerID/modelID#variant".`, `Model "${p}/${m}" is not available. Use the models tool to see what is available.`, `Model "${p}/${m}" is an image|a decision model; subagents need a chat model. Use the models tool to find one.`, `Model "${p}/${m}" has no variants. Omit the variant.`, `Variant "${v}" is not available for "${p}/${m}". Available: ${list}.`
- The child's first user message is `You are a subagent spawned by another session.\n${prompt}` (subagent.ts:208-212); follow-ups via sessionID send the prompt as-is.
- Empty-string `model`/`sessionID` inputs are deleted before validation (execute.before hook, subagent.ts:274-284).

Error envelope (all tools): a failing call reaches the model as a tool_result with `is_error: true` and content
`{"error":{"type":"<type>","message":"<message>"},"content":[]}` (JSON, no spaces), built in
packages/core/src/session/runner/to-llm-message.ts:143-150; `<type>`/`<message>` come from
packages/core/src/session/to-session-error.ts (ToolFailure with an inner error -> the inner error's message wins;
Permission.BlockedError/CorrectedError -> type `permission.rejected`).

### Captured

explore:
````text
<subagent sessionID="ses_e592527beffe4Oc9se7GgUgycU" state="completed">
child explore result
</subagent>
````

general:
````text
<subagent sessionID="ses_e59252708ffelW59hddKTlygAw" state="completed">
child general result
</subagent>
````

unknown agent:
is_error: true
````text
{"error":{"type":"tool.execution","message":"Unknown agent: nosuch"},"content":[]}
````
