# question

Source: packages/core/src/tool/plugin/question.ts, packages/cli/src/run/noninteractive.ts:70-71

Captured from the 0.5.1 binary's `/v1/code/messages` request (build agent, anysola/claude-sonnet-5-5, Linux). Ground truth: `_captured-request-build-sonnet.json`.

## Description (verbatim, as sent)

````text
Use this tool when you need to ask the user questions during execution. This allows you to:
1. Gather user preferences or requirements
2. Clarify ambiguous instructions
3. Get decisions on implementation choices as you work
4. Offer choices to the user about what direction to take.

Usage notes:
- A "Type your own answer" option is added automatically; don't include a separate option for free form answers
- Set `multiple: true` to allow selecting more than one option
- If you recommend a specific option, make that the first option in the list and add "(Recommended)" at the end of the label
````

## input_schema (verbatim, as sent)

```json
{
  "type": "object",
  "properties": {
    "questions": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "question": {
            "type": "string",
            "description": "Complete question"
          },
          "header": {
            "type": "string",
            "description": "Very short label (max 30 chars)"
          },
          "options": {
            "type": "array",
            "items": {
              "type": "object",
              "properties": {
                "label": {
                  "type": "string",
                  "description": "Display text (1-5 words, concise)"
                },
                "description": {
                  "type": "string",
                  "description": "Explanation of choice"
                }
              },
              "required": [
                "label",
                "description"
              ],
              "additionalProperties": false
            },
            "description": "Available choices"
          },
          "multiple": {
            "type": "boolean"
          }
        },
        "required": [
          "question",
          "header",
          "options"
        ],
        "additionalProperties": false
      },
      "minItems": 1,
      "description": "Questions to ask"
    }
  },
  "required": [
    "questions"
  ],
  "additionalProperties": false
}
```

## Variants

Static. Removed from the tool list for agents that deny `question` (general, explore).

## Model-visible output templates (question.ts:38-46)

answered: `User has answered your questions: "${q1}"="${a1, a2}", "${q2}"="Unanswered". You can now continue with the user's answers in mind.`
non-interactive `run` (cancelled with feedback): `This non-interactive run cannot ask the user questions, so the question was cancelled. Continue without an answer; make reasonable assumptions and state them.` (type tool.execution, is_error)
dismissed in TUI (no message): the step ends; the tool shows as aborted (not model-visible text beyond the abort).
permission denied: `Permission denied: question` wrapper (inner message wins).
UI: one form "Questions", one field per question (title=header, description=question, options=label/description, custom answer always allowed; `multiple` -> multiselect).

Error envelope (all tools): a failing call reaches the model as a tool_result with `is_error: true` and content
`{"error":{"type":"<type>","message":"<message>"},"content":[]}` (JSON, no spaces), built in
packages/core/src/session/runner/to-llm-message.ts:143-150; `<type>`/`<message>` come from
packages/core/src/session/to-session-error.ts (ToolFailure with an inner error -> the inner error's message wins;
Permission.BlockedError/CorrectedError -> type `permission.rejected`).

### Captured (run without TTY)

is_error: true
````text
{"error":{"type":"tool.execution","message":"This non-interactive run cannot ask the user questions, so the question was cancelled. Continue without an answer; make reasonable assumptions and state them."},"content":[]}
````
