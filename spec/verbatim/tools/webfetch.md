# webfetch

Source: packages/core/src/tool/plugin/webfetch.ts, packages/core/src/tool/html-markdown.ts, packages/core/src/tool/http-body.ts, packages/core/src/mime.ts:34-42

Captured from the 0.5.1 binary's `/v1/code/messages` request (build agent, anysola/claude-sonnet-5-5, Linux). Ground truth: `_captured-request-build-sonnet.json`.

## Description (verbatim, as sent)

````text
Fetch content from an HTTP or HTTPS URL and return it as text, markdown, or HTML. Markdown is the default.

Use a more targeted tool when one is available. This tool is read-only. Large text results may be replaced with a preview while the complete output is retained in managed storage.
````

## input_schema (verbatim, as sent)

```json
{
  "type": "object",
  "properties": {
    "url": {
      "type": "string",
      "description": "The HTTP or HTTPS URL to fetch content from"
    },
    "format": {
      "type": "string",
      "enum": [
        "text",
        "markdown",
        "html"
      ],
      "description": "The format to return the content in. Defaults to markdown."
    },
    "timeout": {
      "type": "number",
      "exclusiveMinimum": 0,
      "maximum": 120,
      "description": "Optional timeout in seconds (maximum: 120)"
    }
  },
  "required": [
    "url"
  ],
  "additionalProperties": false
}
```

## Variants

None (static).

## Request sent (captured)

- `User-Agent: Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; AnysolaCode-User/1.0; +https://www.anysola.com/code` (webfetch.ts:61-62); on a Cloudflare challenge (403 + `cf-mitigated: challenge`) retried once with `User-Agent: opencode`.
- `Accept` per format (webfetch.ts:46-55):
  - markdown: `text/markdown;q=1.0, text/x-markdown;q=0.9, text/plain;q=0.8, text/html;q=0.7, */*;q=0.1`
  - text: `text/plain;q=1.0, text/markdown;q=0.9, text/html;q=0.8, */*;q=0.1`
  - html: `text/html;q=1.0, application/xhtml+xml;q=0.9, text/plain;q=0.8, text/markdown;q=0.7, */*;q=0.1`
- `Accept-Language: en-US,en;q=0.9`; also b3/traceparent tracing headers and `accept-encoding: gzip, deflate, br, zstd`.

## Model-visible output templates

- success: the body as-is (non-HTML), or converted (HTML + markdown -> custom converter; HTML + text -> text nodes concatenated, script/style/noscript/iframe/object/embed skipped, trimmed; html -> raw).
- errors (type `unknown`, the inner message replaces the `Unable to fetch ${url}` wrapper):
  `URL must use http:// or https://`; `StatusCode: non 2xx status code (${status} GET ${url})`;
  `Unsupported fetched image content type: ${mime}` (image/* except svg+xml and vnd.fastbidsheet);
  `Unsupported fetched file content type: ${mime}` (non-textual mime); `Response too large (exceeds 5242880 byte limit)`;
  `Request timed out`; `Too many redirects (more than 10)`.
- long output: generic truncation (2000 lines / 50 KiB) adds a second text block `[showing lines 1-N of M; full output saved to <data>/tool-output/tool_<id>]`.

Error envelope (all tools): a failing call reaches the model as a tool_result with `is_error: true` and content
`{"error":{"type":"<type>","message":"<message>"},"content":[]}` (JSON, no spaces), built in
packages/core/src/session/runner/to-llm-message.ts:143-150; `<type>`/`<message>` come from
packages/core/src/session/to-session-error.ts (ToolFailure with an inner error -> the inner error's message wins;
Permission.BlockedError/CorrectedError -> type `permission.rejected`).

### Captured (local server)

/html markdown:
````text
# Hello

Some **bold** and [link](/x).

- a

- b

```js
let a=1
```

| A | B |
| --- | --- |
| 1 | 2 |
````

/html text:
````text
THelloSome bold and link.ablet a=1AB12
````

/json:
````text
{"a":1}
````

/404:
is_error: true
````text
{"error":{"type":"unknown","message":"StatusCode: non 2xx status code (404 GET http://127.0.0.1:8819/404)"},"content":[]}
````

image/png:
is_error: true
````text
{"error":{"type":"unknown","message":"Unsupported fetched image content type: image/png"},"content":[]}
````

octet-stream:
is_error: true
````text
{"error":{"type":"unknown","message":"Unsupported fetched file content type: application/octet-stream"},"content":[]}
````

6 MiB:
is_error: true
````text
{"error":{"type":"unknown","message":"Response too large (exceeds 5242880 byte limit)"},"content":[]}
````

302 -> /json:
````text
{"a":1}
````

timeout 1 on slow:
is_error: true
````text
{"error":{"type":"unknown","message":"Request timed out"},"content":[]}
````

ftp://:
is_error: true
````text
{"error":{"type":"unknown","message":"URL must use http:// or https://"},"content":[]}
````

3000-line text (content became 2 blocks; tail of block 1 + block 2):
````text
...line 1999
[showing lines 1-2000 of 3000; full output saved to /home/ubuntu/anysola-code/rust/testing/runs/tools/home/.local/share/opencode/tool-output/tool_1a6d9e8ea0012Inl50WKihUpZS]
````
