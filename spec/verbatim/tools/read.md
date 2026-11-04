# read

Source: packages/core/src/tool/plugin/read.ts, packages/core/src/tool/read-filesystem.ts, packages/core/src/file-access.ts, packages/core/src/mime.ts, packages/core/src/tool.ts (image normalization), packages/core/src/session/model-request.ts (unsupported media / image bounding)

Captured from the 0.5.1 binary's `/v1/code/messages` request (build agent, anysola/claude-sonnet-5-5, Linux). Ground truth: `_captured-request-build-sonnet.json`.

## Description (verbatim, as sent)

````text
Read the contents of a file or directory. Supports text files, images, and PDFs. Images and PDFs are presented directly to the model. Each text line is prefixed by its 1-based line number as <line>: <content>. The prefix is for reference and is not part of the file content. Directory entries are returned one per line. Use offset and limit to read large files or directories in sections. Prefer one larger read over many small slices, and use grep to find specific content in large files.
````

## input_schema (verbatim, as sent)

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "File or directory to read"
    },
    "offset": {
      "type": "integer",
      "minimum": 0,
      "description": "The line or directory entry to start reading from (1-based)"
    },
    "limit": {
      "type": "integer",
      "minimum": 0,
      "description": "The maximum number of lines or directory entries to read (defaults to and capped at 2000)"
    }
  },
  "required": [
    "path"
  ],
  "additionalProperties": false
}
```

## Error envelope (applies to every tool)

A failing tool call is sent to the model as a `tool_result` with `"is_error": true` whose `content` is a JSON string (captured):

```text
{"error":{"type":"<type>","message":"<message>"},"content":[]}
```

`type` is `tool.execution` for a plain `ToolFailure` (no wrapped cause), `permission.rejected` for a permission block/decline-with-feedback, and `unknown` when the `ToolFailure` wraps an error object whose own message is non-empty (the wrapped message then replaces the tool's message; `packages/core/src/session/to-session-error.ts:75-80`). Successful results are a plain string `content` (or an array when images/files are present).

## Model-visible output templates

Success, text file (`read.ts:227-237`). Header then `<n>: <line>` per line, joined by `\n`, no trailing newline:

```text
Read file <path>, lines <start>-<end>
<start>: <line>
...
```

- Empty file: `Read file <path>, 0 lines`
- `<path>` is the path exactly as the model passed it (relative stays relative, absolute stays absolute); after filename recovery it is `join(dirname(input.path), basename(actualName))`.
- Paged result with more lines (`read.ts:235-236`): an extra last line `[Output truncated. Continue reading with offset: <next>]`

Captured examples:

```text
Read file a.txt, lines 1-3
1: line one
2: line two
3: line three
```

```text
Read file long.txt, lines 2990-2994
2990: row 2990
2991: row 2991
2992: row 2992
2993: row 2993
2994: row 2994
[Output truncated. Continue reading with offset: 2995]
```

```text
Read file big.txt, lines 1-512
1: yyyy...(99 chars)
...
512: yyyy...
[Output truncated. Continue reading with offset: 513]
```

Line longer than 2000 UTF-16 code units (`read-filesystem.ts:18-20,326-327`): first 2000 chars + `... (line truncated to 2000 chars)`.

CRLF quirk (captured): a file read WITHOUT paging (size <= 51200 bytes and no offset/limit) keeps `\r`: tool_result string was `"Read file crlf.txt, lines 1-3\n1: alpha\r\n2: beta\r\n3: gamma\r"`. Paged reads strip one trailing `\r` per line (`read-filesystem.ts:297`).

Blank trailing lines: file `"one\n\n\n"` -> `Read file blank.txt, lines 1-3\n1: one\n2: \n3: ` (note `"2: "` with trailing space).

Directory (`read.ts:214-225`):

```text
Read directory <path>, entries <start>-<end>
<entry>
...
[Output truncated. Continue reading with offset: <next>]   (only when truncated)
```

- Empty page / offset past the end: `Read directory <path>, 0 entries` (not an error).
- Entries: directories first with a trailing path separator (`/` on Unix, `\` on Windows, `path.sep`), then files and symlinks, each group sorted with JS `localeCompare`; `other` entries (sockets, fifos) are hidden. Hidden dirs such as `.git/` are listed. Captured:

```text
Read directory ., entries 1-15
.git/
sub/
a.txt
alpha-notes.txt
big.txt
bin.dat
blank.txt
crlf.txt
empty.txt
link.txt
long.txt
many.txt
nonl.txt
Screenshot 2026 at 10.txt
wide.txt
```

Image / PDF (`read.ts:203-212`): content array `[text, file]`; text is exactly `Image read successfully` or `PDF read successfully`. On the Anthropic wire (captured) the file becomes `{"type":"image","source":{"type":"base64","media_type":"image/png","data":"..."}}` inside the tool_result content array.

Image normalization notes appended as extra text items (`packages/core/src/tool.ts:109-118`):

```text
[1 image omitted: could not be decoded.]
[<n> images omitted: could not be decoded.]
[1 image omitted: could not be resized below the image size limit.]
[<n> images omitted: could not be resized below the image size limit.]
```

Captured for a corrupt PNG: `[text]Image read successfully` + `[text][1 image omitted: could not be decoded.]` (not is_error).

Unsupported media for the model (`packages/core/src/session/model-request.ts:138-151`), replaces the file item; captured for a PDF with the Anysola gateway models (which declare no pdf input):

```text
ERROR: Cannot read "doc.pdf" (this model does not support pdf input). Inform the user.
```

Template: `ERROR: Cannot read ${name ? `"${name}"` : modality} (this model does not support ${unsupported} input). Inform the user.`

Old images dropped from the request when inline image base64 exceeds 25 MiB (cut back to 15 MiB, oldest first) (`model-request.ts:44-47,195-222`):

```text
[This image was removed to reduce the request size and is no longer visible. Do not make claims about its contents from memory. If needed, retrieve it again with an available tool or ask the user to attach it again.]
```

Errors (all captured unless marked):

```text
{"error":{"type":"tool.execution","message":"File not found: nope.txt"},"content":[]}
{"error":{"type":"tool.execution","message":"File not found: alpha.txt\n\nDid you mean one of these?\na.txt"},"content":[]}
{"error":{"type":"tool.execution","message":"File not found: missing/dir/x.txt\n\nThe folder /abs/proj/missing does not exist. The nearest folder that does is /abs/proj."},"content":[]}
{"error":{"type":"unknown","message":"Cannot read binary file: bin.dat"},"content":[]}
{"error":{"type":"unknown","message":"Offset 10 is out of range for this file (3 lines)"},"content":[]}
{"error":{"type":"unknown","message":"Media exceeds 20971520 byte ingestion limit: huge.png"},"content":[]}
{"error":{"type":"tool.execution","message":"Invalid arguments for tool \"read\":\n- offset: Expected a value greater than or equal to 0\n\nArguments provided:\n{\n  \"path\": \"a.txt\",\n  \"offset\": -1\n}\n\nUpdate the arguments and call the tool again."},"content":[]}
{"error":{"type":"permission.rejected","message":"This non-interactive run cannot ask the user for permission, so the request was rejected. Continue without this action."},"content":[]}
```

Message templates (source):
- `File not found: ${input}` (`read.ts:184,195`)
- `File not found: ${input}\n\nDid you mean one of these?\n${suggestions.join("\n")}` (up to 3; suggestion = sibling whose lowercase name contains, or is contained in, the lowercase requested basename; joined with `dirname(input)`) (`read.ts:154-162,191-196`)
- Missing folder: `[`File not found: ${input}`, `The folder ${child} does not exist. The nearest folder that does is ${parent}.`, `Did you mean one of these?\n${suggestions}`(optional)].join("\n\n")` - child/parent are absolute (`read.ts:166-185`)
- `Cannot read binary file: ${resource}` (`read-filesystem.ts:27`)
- `Media exceeds ${maximumBytes} byte ingestion limit: ${resource}` (`read-filesystem.ts:39`)
- `Offset ${offset} is out of range for this file (${lines} ${lines === 1 ? "line" : "lines"})` (`read-filesystem.ts:48`)
- `Path is not ${expected}: ${resource}` with expected `a file` or `a file or directory` (`read-filesystem.ts:57`)
- `Unable to read ${input.path}` (fallback for other failures; the model sees the wrapped cause's message instead if it has one) (`read.ts:123`)
- `resource` = location-relative path with `/` separators for internal files, absolute (with `/`) for external ones.

## Variants

None captured: identical description and schema for auto, claude-sonnet-5-5, claude-haiku-5-5 and the plan agent. Description and schema are static strings (`read.ts:18-24,43-44`); not platform-dependent.
