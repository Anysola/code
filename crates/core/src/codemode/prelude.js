// Runs once per execution before the program: builds the confined global environment the
// program sees (as 0.5.1's interpreter offered it) and the driver the host awaits. Host entry
// points: __sync(op, json) -> json and __async(op, json) -> Promise<json>.
(function (global) {
  "use strict";
  const hostSync = global.__sync;
  const hostAsync = global.__async;
  delete global.__sync;
  delete global.__async;

  const DIAGNOSTIC = Symbol("diagnostic");
  const TOOLS = Symbol("tools");
  const BASE = {};
  const logs = [];

  const call = (op, payload) => {
    const reply = JSON.parse(hostSync(op, JSON.stringify(payload)));
    if (reply.error !== undefined) throw hostError(reply.error);
    return reply.ok;
  };
  const callAsync = (op, payload) =>
    hostAsync(op, JSON.stringify(payload)).then((text) => {
      const reply = JSON.parse(text);
      if (reply.error !== undefined) throw hostError(reply.error);
      return reply.ok;
    });

  // A failure from the host: `message` is what the program sees, `diagnostic` what the model sees
  // when the program does not handle it.
  function hostError(error) {
    const Base = BASE[error.name ?? "Error"] ?? BASE.Error;
    const value = new Base(error.message);
    Object.defineProperty(value, DIAGNOSTIC, {
      value: error.diagnostic ?? (error.name ? `${error.name}: ${error.message}` : error.message),
    });
    return value;
  }

  // Errors the program constructs format as `Name: message`; errors the engine raises carry a
  // location instead. Wrapping the constructors tells them apart.
  const USER = Symbol("user");
  for (const name of ["Error", "TypeError", "RangeError", "SyntaxError", "ReferenceError", "EvalError", "URIError", "AggregateError"]) {
    const Base = global[name];
    BASE[name] = Base;
    const Wrapped = function (...args) {
      const value = Reflect.construct(Base, args, new.target === undefined || new.target === Wrapped ? Base : new.target);
      Object.defineProperty(value, USER, { value: true });
      return value;
    };
    Object.defineProperty(Wrapped, "name", { value: name });
    Wrapped.prototype = Base.prototype;
    Object.setPrototypeOf(Wrapped, Base === global.Error ? Function.prototype : global.Error);
    Object.defineProperty(Base.prototype, "constructor", { value: Wrapped, writable: true, configurable: true });
    if (Base === global.Error) Wrapped.isError = (value) => value instanceof Base;
    global[name] = Wrapped;
  }

  const rejectFunction = function () {
    throw new BASE.TypeError("The Function constructor is not supported; write the function inline.");
  };
  for (const sample of [function () {}, async function () {}, function* () {}, async function* () {}]) {
    Object.defineProperty(Object.getPrototypeOf(sample), "constructor", { value: rejectFunction, writable: true, configurable: true });
  }
  rejectFunction.prototype = Function.prototype;
  global.Function = rejectFunction;

  // ---- data crossing to the host -------------------------------------------------------------

  const isTools = (value) => (typeof value === "object" || typeof value === "function") && value !== null && value[TOOLS] !== undefined;

  async function settle(value, depth) {
    if (value instanceof Promise) value = await value;
    if (depth > 64 || value === null || typeof value !== "object" || isTools(value)) return value;
    if (Array.isArray(value)) {
      for (let i = 0; i < value.length; i++) if (value[i] instanceof Promise) value[i] = await settle(value[i], depth + 1);
      return value;
    }
    if (Object.getPrototypeOf(value) === Object.prototype) {
      for (const key of Object.keys(value)) if (value[key] instanceof Promise) value[key] = await settle(value[key], depth + 1);
    }
    return value;
  }

  function replacer(key, value) {
    if (isTools(value)) return undefined;
    if (value instanceof Uint8Array) {
      throw hostError({
        name: "TypeError",
        message: "A Uint8Array cannot cross to the host; pass text instead, e.g. `new TextDecoder().decode(bytes)` or `bytes.toBase64()`.",
      });
    }
    if (value instanceof Set) return [...value];
    if (value instanceof URLSearchParams) return value.toString();
    if (value instanceof Headers) return Object.fromEntries(value.entries());
    if (value instanceof BASE.Error) return Object.assign({ name: value.name, message: value.message }, value);
    if (value instanceof Promise) return undefined;
    return value;
  }

  function toData(value, space) {
    try {
      return JSON.stringify(value, replacer, space);
    } catch (error) {
      if (error instanceof BASE.TypeError && /circular/i.test(error.message) && error[DIAGNOSTIC] === undefined) {
        throw hostError({ name: "TypeError", message: "Converting circular structure to JSON." });
      }
      throw error;
    }
  }

  const fromData = (text) => (text === undefined ? undefined : JSON.parse(text));

  // ---- tools and search ----------------------------------------------------------------------

  function reference(path) {
    const target = path.length === 0 ? {} : () => {};
    return new Proxy(target, {
      get(_, key) {
        if (key === TOOLS) return path;
        if (typeof key === "symbol" || key === "then" || key === "toJSON") return undefined;
        return reference([...path, key]);
      },
      has: (_, key) => typeof key === "string" && call("keys", path).includes(key),
      ownKeys: () => call("keys", path),
      getOwnPropertyDescriptor: (_, key) =>
        typeof key === "string" && call("keys", path).includes(key)
          ? { value: reference([...path, key]), enumerable: true, configurable: true, writable: false }
          : undefined,
      apply: (_, __, args) => invoke(path, args),
      set: () => false,
      defineProperty: () => false,
      deleteProperty: () => false,
    });
  }

  function invoke(path, args) {
    const data = args.map((arg) => fromData(toData(arg)) ?? null);
    return callAsync("call", { path, args: data });
  }

  // Usable directly, and also awaitable or chainable like a Promise.
  function promiseLike(value) {
    if (value === null || typeof value !== "object") return value;
    for (const name of ["then", "catch", "finally"]) {
      Object.defineProperty(value, name, {
        value: (...args) => {
          const plain = Object.assign(Array.isArray(value) ? [] : {}, value);
          return Promise.resolve(plain)[name](...args);
        },
      });
    }
    return value;
  }

  function search(...args) {
    const data = args.map((arg) => fromData(toData(arg)) ?? null);
    return promiseLike(call("search", data));
  }

  // ---- console -------------------------------------------------------------------------------

  const MAX_DEPTH = 32;
  function inspect(value, seen, depth) {
    if (value === null || value === undefined) return "null";
    if (typeof value === "string") return JSON.stringify(value);
    if (typeof value === "function" || isTools(value)) return "[opaque reference]";
    if (typeof value !== "object") return String(value);
    if (depth > MAX_DEPTH) return "...";
    if (seen.has(value)) return "[Circular]";
    seen.add(value);
    try {
      const item = (v) => inspect(v, seen, depth + 1);
      if (value instanceof BASE.Error) return String(value);
      if (value instanceof Promise) return "[Promise (await it to get its value)]";
      if (value instanceof Date) return Number.isFinite(value.getTime()) ? value.toISOString() : "Invalid Date";
      if (value instanceof Map) return `Map(${value.size}) [${[...value].map(([k, v]) => `[${item(k)},${item(v)}]`).join(",")}]`;
      if (value instanceof Set) return `Set(${value.size}) [${[...value].map(item).join(",")}]`;
      if (value instanceof RegExp || value instanceof URL || value instanceof URLSearchParams) return String(value);
      if (Array.isArray(value)) return `[${value.map(item).join(",")}]`;
      return `{${Object.keys(value).map((k) => `${JSON.stringify(k)}:${item(value[k])}`).join(",")}}`;
    } finally {
      seen.delete(value);
    }
  }
  const show = (value) => (value === undefined ? "undefined" : typeof value === "string" ? value : inspect(value, new Set(), 0));

  function table(data, columns) {
    if (data === undefined) return "undefined";
    if (isTools(data)) return "[opaque reference]";
    const values = (item) => {
      if (item !== null && typeof item === "object" && !Array.isArray(item)) {
        if (Array.isArray(columns)) return Object.fromEntries(columns.map((c) => [String(c), item[c]]));
        return Object.fromEntries(Object.keys(item).map((k) => [k, item[k]]));
      }
      return { Value: item };
    };
    const rows = Array.isArray(data)
      ? data.map((item, index) => ({ index: String(index), values: values(item) }))
      : data !== null && typeof data === "object"
        ? Object.keys(data).map((index) => ({ index, values: values(data[index]) }))
        : [{ index: "0", values: { Value: data } }];
    const keys = Array.isArray(columns) ? columns.map(String) : [...new Set(rows.flatMap((row) => Object.keys(row.values)))];
    const cell = (v) => (v === undefined ? "" : typeof v === "string" ? v : inspect(v, new Set(), 0));
    return [["(index)", ...keys].join("\t"), ...rows.map((row) => [row.index, ...keys.map((k) => cell(row.values[k]))].join("\t"))].join("\n");
  }

  const prefixes = { warn: "[warn] ", error: "[error] ", debug: "[debug] " };
  const consoleObject = {};
  for (const name of ["log", "info", "debug", "warn", "error", "dir", "table"]) {
    consoleObject[name] = (...args) => {
      if (name === "dir") logs.push(args.length === 0 ? "undefined" : show(args[0]));
      else if (name === "table") logs.push(table(args[0], args[1]));
      else logs.push(`${prefixes[name] ?? ""}${args.map(show).join(" ")}`);
    };
  }

  // ---- web globals ---------------------------------------------------------------------------

  class TextEncoder {
    get encoding() {
      return "utf-8";
    }
    encode(text = "") {
      const bytes = [];
      for (const char of String(text)) {
        let code = char.codePointAt(0);
        if (code >= 0xd800 && code <= 0xdfff) code = 0xfffd;
        if (code < 0x80) bytes.push(code);
        else if (code < 0x800) bytes.push(0xc0 | (code >> 6), 0x80 | (code & 63));
        else if (code < 0x10000) bytes.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 63), 0x80 | (code & 63));
        else bytes.push(0xf0 | (code >> 18), 0x80 | ((code >> 12) & 63), 0x80 | ((code >> 6) & 63), 0x80 | (code & 63));
      }
      return new Uint8Array(bytes);
    }
  }

  class TextDecoder {
    get encoding() {
      return "utf-8";
    }
    decode(input) {
      if (input === undefined) return "";
      const bytes = input instanceof Uint8Array ? input : new Uint8Array(input.buffer ?? input);
      let out = "";
      for (let i = 0; i < bytes.length; ) {
        const b = bytes[i];
        const need = b < 0x80 ? 0 : b >= 0xf0 && b < 0xf8 ? 3 : b >= 0xe0 ? 2 : b >= 0xc2 && b < 0xe0 ? 1 : -1;
        if (need < 0 || i + need >= bytes.length + (need === 0 ? 1 : 0)) {
          out += need === 0 ? String.fromCharCode(b) : "�";
          i += 1;
          continue;
        }
        let code = need === 0 ? b : b & (0x3f >> need);
        let ok = true;
        for (let k = 1; k <= need; k++) {
          const c = bytes[i + k];
          if ((c & 0xc0) !== 0x80) ok = false;
          code = (code << 6) | (c & 63);
        }
        if (!ok) {
          out += "�";
          i += 1;
          continue;
        }
        out += String.fromCodePoint(code);
        i += need + 1;
      }
      return out;
    }
  }

  const encodeForm = (text) =>
    encodeURIComponent(text)
      .replace(/%20/g, "+")
      .replace(/[!'()~]/g, (c) => "%" + c.charCodeAt(0).toString(16).toUpperCase());
  const decodeForm = (text) => {
    try {
      return decodeURIComponent(text.replace(/\+/g, " "));
    } catch {
      return text;
    }
  };

  class URLSearchParams {
    #list = [];
    #owner;
    constructor(init = "", owner) {
      this.#owner = owner;
      if (typeof init === "string") this.#parse(init);
      else if (init instanceof URLSearchParams) this.#list = [...init.entries()];
      else if (Array.isArray(init)) this.#list = init.map(([k, v]) => [String(k), String(v)]);
      else if (init && typeof init === "object") this.#list = Object.keys(init).map((k) => [k, String(init[k])]);
    }
    #parse(text) {
      this.#list = text
        .replace(/^\?/, "")
        .split("&")
        .filter((part) => part !== "")
        .map((part) => {
          const at = part.indexOf("=");
          return at < 0 ? [decodeForm(part), ""] : [decodeForm(part.slice(0, at)), decodeForm(part.slice(at + 1))];
        });
    }
    #changed() {
      this.#owner?.(this.toString());
    }
    get size() {
      return this.#list.length;
    }
    append(name, value) {
      this.#list.push([String(name), String(value)]);
      this.#changed();
    }
    delete(name) {
      this.#list = this.#list.filter(([k]) => k !== String(name));
      this.#changed();
    }
    get(name) {
      return this.#list.find(([k]) => k === String(name))?.[1] ?? null;
    }
    getAll(name) {
      return this.#list.filter(([k]) => k === String(name)).map(([, v]) => v);
    }
    has(name) {
      return this.#list.some(([k]) => k === String(name));
    }
    set(name, value) {
      const key = String(name);
      const at = this.#list.findIndex(([k]) => k === key);
      if (at < 0) this.#list.push([key, String(value)]);
      else this.#list = this.#list.filter(([k], i) => k !== key || i === at).map((e, i) => (i === at ? [key, String(value)] : e));
      this.#changed();
    }
    sort() {
      this.#list.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
      this.#changed();
    }
    forEach(fn, self) {
      for (const [k, v] of this.#list) fn.call(self, v, k, this);
    }
    entries() {
      return this.#list.map((e) => [...e])[Symbol.iterator]();
    }
    keys() {
      return this.#list.map(([k]) => k)[Symbol.iterator]();
    }
    values() {
      return this.#list.map(([, v]) => v)[Symbol.iterator]();
    }
    [Symbol.iterator]() {
      return this.entries();
    }
    toString() {
      return this.#list.map(([k, v]) => `${encodeForm(k)}=${encodeForm(v)}`).join("&");
    }
  }

  class URL {
    #parts;
    #params;
    constructor(input, base) {
      this.#parts = call("url", [String(input), base === undefined ? null : String(base)]);
      this.#params = new URLSearchParams(this.#parts.search, (query) => {
        this.#parts = call("url", [this.#parts.href.replace(/(\?[^#]*)?(#.*)?$/, (query ? "?" + query : "") + this.#parts.hash), null]);
      });
    }
    static canParse(input, base) {
      try {
        new URL(input, base);
        return true;
      } catch {
        return false;
      }
    }
    get href() { return this.#parts.href; }
    get origin() { return this.#parts.origin; }
    get protocol() { return this.#parts.protocol; }
    get username() { return this.#parts.username; }
    get password() { return this.#parts.password; }
    get host() { return this.#parts.host; }
    get hostname() { return this.#parts.hostname; }
    get port() { return this.#parts.port; }
    get pathname() { return this.#parts.pathname; }
    get search() { return this.#parts.search; }
    get hash() { return this.#parts.hash; }
    get searchParams() { return this.#params; }
    toString() { return this.href; }
    toJSON() { return this.href; }
  }

  class Headers {
    #map = new Map();
    constructor(init) {
      const pairs = init instanceof Headers ? [...init.entries()] : Array.isArray(init) ? init : init && typeof init === "object" ? Object.entries(init) : [];
      for (const [k, v] of pairs) this.append(k, v);
    }
    append(name, value) {
      const key = String(name).toLowerCase();
      const current = this.#map.get(key);
      this.#map.set(key, current === undefined ? String(value) : `${current}, ${value}`);
    }
    set(name, value) {
      this.#map.set(String(name).toLowerCase(), String(value));
    }
    get(name) {
      return this.#map.get(String(name).toLowerCase()) ?? null;
    }
    has(name) {
      return this.#map.has(String(name).toLowerCase());
    }
    delete(name) {
      this.#map.delete(String(name).toLowerCase());
    }
    forEach(fn, self) {
      for (const [k, v] of this.entries()) fn.call(self, v, k, this);
    }
    entries() {
      return [...this.#map].sort(([a], [b]) => (a < b ? -1 : 1))[Symbol.iterator]();
    }
    keys() {
      return [...this.entries()].map(([k]) => k)[Symbol.iterator]();
    }
    values() {
      return [...this.entries()].map(([, v]) => v)[Symbol.iterator]();
    }
    [Symbol.iterator]() {
      return this.entries();
    }
  }

  const crypto = {
    randomUUID: () => call("uuid", null),
    getRandomValues(array) {
      const bytes = call("random", array.byteLength);
      new Uint8Array(array.buffer, array.byteOffset, array.byteLength).set(bytes);
      return array;
    },
  };

  function structuredClone(value, seen = new Map()) {
    if (value === null || typeof value !== "object") {
      if (typeof value === "function") throw new BASE.TypeError("structuredClone cannot clone a function.");
      return value;
    }
    if (seen.has(value)) return seen.get(value);
    let copy;
    if (value instanceof Date) copy = new Date(value.getTime());
    else if (value instanceof Map) {
      copy = new Map();
      seen.set(value, copy);
      for (const [k, v] of value) copy.set(structuredClone(k, seen), structuredClone(v, seen));
      return copy;
    } else if (value instanceof Set) {
      copy = new Set();
      seen.set(value, copy);
      for (const v of value) copy.add(structuredClone(v, seen));
      return copy;
    } else if (value instanceof Uint8Array) copy = new Uint8Array(value);
    else if (value instanceof RegExp) copy = new RegExp(value.source, value.flags);
    else if (Array.isArray(value)) {
      copy = [];
      seen.set(value, copy);
      for (const v of value) copy.push(structuredClone(v, seen));
      return copy;
    } else {
      copy = {};
      seen.set(value, copy);
      for (const k of Object.keys(value)) copy[k] = structuredClone(value[k], seen);
      return copy;
    }
    seen.set(value, copy);
    return copy;
  }

  async function fetch(input, init = {}) {
    const headers = init.headers instanceof Headers ? [...init.headers.entries()] : Array.isArray(init.headers) ? init.headers : Object.entries(init.headers ?? {});
    let body = init.body;
    if (body instanceof URLSearchParams) body = body.toString();
    else if (body instanceof Uint8Array) body = { bytes: [...body] };
    const reply = await callAsync("fetch", {
      url: String(input),
      method: typeof init.method === "string" ? init.method.toUpperCase() : "GET",
      headers: headers.map(([k, v]) => [String(k), String(v)]),
      body: body === undefined || body === null ? null : typeof body === "object" ? body : String(body),
    });
    const bytes = new Uint8Array(reply.body);
    const responseHeaders = Object.fromEntries(reply.headers);
    const text = () => new TextDecoder().decode(bytes);
    return {
      url: reply.url,
      status: reply.status,
      statusText: reply.statusText,
      ok: reply.ok,
      redirected: reply.redirected,
      headers: {
        get: (name) => responseHeaders[String(name).toLowerCase()] ?? null,
        has: (name) => String(name).toLowerCase() in responseHeaders,
        entries: () => Object.entries(responseHeaders),
      },
      text: async () => text(),
      json: async () => JSON.parse(text()),
      bytes: async () => bytes,
    };
  }

  // ---- the environment -----------------------------------------------------------------------

  const tools = reference([]);
  const bindings = { tools, search, console: consoleObject, fetch, TextEncoder, TextDecoder, URL, URLSearchParams, Headers, crypto, structuredClone };
  for (const [name, value] of Object.entries(bindings)) {
    Object.defineProperty(global, name, { value, writable: false, enumerable: false, configurable: false });
  }
  for (const name of ["eval", "queueMicrotask", "performance", "navigator", "gc", "print", "__loadScript"]) delete global[name];

  // ---- the driver ----------------------------------------------------------------------------

  const ENGINE_MESSAGES = [
    [/^(.+) is not defined$/, (_, name) => `Unknown identifier '${name}'.`],
    [/^cannot read property '(.*)' of (undefined|null)$/, (_, key, what) => `Cannot read properties of ${what} (reading '${key}').`],
    [/^not a function$/, () => "The called value is not a function."],
    [/^(.+) is not a function$/, (_, name) => `${name} is not a function.`],
  ];

  function engineMessage(error, offset) {
    const message = String(error.message);
    if (/^(Maximum call stack size exceeded|stack overflow)$/.test(message)) return "Execution exceeded the maximum nesting depth.";
    let text = message.endsWith(".") ? message : `${message}.`;
    for (const [pattern, replace] of ENGINE_MESSAGES) {
      const match = pattern.exec(message);
      if (match) {
        text = replace(...match);
        break;
      }
    }
    const frame = /<input>:(\d+):(\d+)/.exec(String(error.stack ?? ""));
    const location = frame ? ` (line ${Math.max(1, Number(frame[1]) - offset)}, col ${frame[2]})` : "";
    return `${error.name}: ${text}${location}`;
  }

  function describe(error, offset) {
    if (error !== null && typeof error === "object" && error[DIAGNOSTIC] !== undefined) return error[DIAGNOSTIC];
    if (error instanceof BASE.Error) return error[USER] ? String(error) : engineMessage(error, offset);
    if (isTools(error) || typeof error === "function") return "Uncaught: a non-data value";
    return `Uncaught: ${typeof error === "string" ? error : error === undefined ? "undefined" : inspect(error, new Set(), 0)}`;
  }

  function format(value) {
    if (value === global) return "null";
    const compact = toData(value);
    if (compact === undefined) return "null";
    const data = JSON.parse(compact);
    return typeof data === "string" ? data : JSON.stringify(data, null, 2);
  }

  // `completion` settles to `{ value }` for a global script, to the value for a function body.
  Object.defineProperty(global, "__finish", {
    value: async (completion, wrapped, offset) => {
      try {
        const settled = await completion;
        const value = await settle(wrapped ? settled.value : settled, 0);
        return JSON.stringify({ ok: format(value), logs });
      } catch (error) {
        return JSON.stringify({ error: describe(error, offset), logs });
      }
    },
  });
  delete global.globalThis;
})(globalThis);
