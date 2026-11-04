# websearch

Source: packages/core/src/tool/plugin/websearch.ts, packages/core/src/plugin/websearch/anysola.ts, packages/core/src/websearch.ts

Captured from the 0.5.1 binary's `/v1/code/messages` request (build agent, anysola/claude-sonnet-5-5, Linux). Ground truth: `_captured-request-build-sonnet.json`.

## Description (verbatim, as sent)

````text
Search the web using the user's selected search integration. Use this for current information beyond knowledge cutoff.

The current year is 2026. Use this year when searching for recent information or current events.
````

## input_schema (verbatim, as sent)

```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Websearch query"
    }
  },
  "required": [
    "query"
  ],
  "additionalProperties": false
}
```

## Variants

`The current year is ${new Date().getFullYear()}.` is evaluated once at module load (websearch.ts:21-23), i.e. the
year of the process start, local time zone. Everything else is static. The tool is removed from the request when web
search is disabled (config `"websearch": false`) by a session context hook (websearch.ts:188-198).

## Gateway request (captured)

POST `${ANYSOLA_API_URL or https://api.anysola.com}/v1/code/search`, body `{"query":"<query>"}`, headers
`accept: application/json`, `authorization: Bearer <token>`, `content-type: application/json`, `x-anysola-client: opencode`,
`x-anysola-version: 0.5.1`, `user-agent: opencode/latest/2.2.5/cli`, b3/traceparent. Timeout 30 s, max response 1 MiB.
Response `{results:[{url,title?,content?,published? (ms)}], credits?}`; `credits` is added to the session's spend.

## Model-visible output templates (websearch.ts:154-164)

Per result `## [${title ?? url}](${url})` + (`\nPublished: ${ISO}` if published) + (`\n\n${content}` if content), joined by blank line.
No results: `No search results found. Please try a different query.`
Errors (type `tool.execution`, anysola.ts:42-53, 93, 110, 116, 120, 72):
- 401 `Your Anysola sign-in has expired or was revoked. Run `+"`anysola login`"+`.`
- 402 `Out of Anysola credits for web search. Add credits at https://account.anysola.com/billing`
- 429 `Anysola web search is rate limited: ${detail}`
- other `Anysola web search failed (HTTP ${status}): ${detail}` (detail = body error_description ?? error.message ?? message ?? error code ?? `HTTP ${status}`)
- no token `Web search needs an Anysola sign-in. Run `+"`anysola login`"+`.`; too big `Anysola web search returned too much data`;
  timeout `Anysola web search timed out`; network `Cannot reach ${apiURL}`; bad JSON `Anysola web search returned an unexpected response`.

Error envelope (all tools): a failing call reaches the model as a tool_result with `is_error: true` and content
`{"error":{"type":"<type>","message":"<message>"},"content":[]}` (JSON, no spaces), built in
packages/core/src/session/runner/to-llm-message.ts:143-150; `<type>`/`<message>` come from
packages/core/src/session/to-session-error.ts (ToolFailure with an inner error -> the inner error's message wins;
Permission.BlockedError/CorrectedError -> type `permission.rejected`).

### Captured

1 result:
````text
## [Bun](https://bun.com)

Bun 1.4.4
````

title missing + published, then title+content:
````text
## [https://a.b](https://a.b)
Published: 2026-09-21T14:13:20.000Z

## [C](https://c.d)

cc
````

empty:
````text
No search results found. Please try a different query.
````

401:
is_error: true
````text
{"error":{"type":"tool.execution","message":"Your Anysola sign-in has expired or was revoked. Run `anysola login`."},"content":[]}
````

402:
is_error: true
````text
{"error":{"type":"tool.execution","message":"Out of Anysola credits for web search. Add credits at https://account.anysola.com/billing"},"content":[]}
````

429:
is_error: true
````text
{"error":{"type":"tool.execution","message":"Anysola web search is rate limited: gateway says no"},"content":[]}
````

500:
is_error: true
````text
{"error":{"type":"tool.execution","message":"Anysola web search failed (HTTP 500): gateway says no"},"content":[]}
````

not JSON:
is_error: true
````text
{"error":{"type":"tool.execution","message":"Anysola web search returned an unexpected response"},"content":[]}
````

