// Evaluated inside the QuickJS VM before a codemode script runs, after pi's codemode prelude.
//
// The VM is a runtime of its own with no I/O, so nothing here guards a realm boundary. The
// prelude keeps the host bridge in a closure the script cannot reach, and builds `tools`,
// `ALL_TOOLS`, the output helpers (`text`, `image`, `exit`, `console`), `store`/`load`, and
// the host globals on top of it. Arguments and results cross as JSON text.
//
// `bridge(kind, id, text, extra)` takes primitives only:
// - ("call", id, name, argsJson?) and ("global", id, name, argsJson?) ask the host to run a tool
//   or a global; the host answers through `settle`.
// - ("text", 0, text) and ("image", 0, base64, mimeType) append output.
// - ("done", 0, writesJson, valueJson?) and ("failed", 0, errorJson) end the script.
//
// Evaluates to `(bridge, toolsJson, globalsJson, storeJson, limitsJson) => { settle, run, stalled }`.
// `stalled()` says where the script stands once its jobs ran: "waiting" on a host call,
// "finished", or "stalled", which fails a script that waits on nothing: with no timers or I/O
// in the VM, nothing can ever resume it.
(function (bridge, toolsJson, globalsJson, storeJson, limitsJson) {
	"use strict";
	const stringify = JSON.stringify;
	const parse = JSON.parse;
	const promiseThen = Promise.prototype.then;
	const ErrorCtor = Error;
	const TypeErrorCtor = TypeError;
	const RangeErrorCtor = RangeError;
	const limits = parse(limitsJson);
	const pending = new Map();
	let nextId = 1;
	let finished = false;
	// Thrown by exit() to unwind the script after it already reported success.
	const EXIT = Object.freeze({});

	// The bridge takes no explicit `undefined`: an absent value is a missing argument.
	function send(kind, id, text, extra) {
		if (extra === undefined) bridge(kind, id, text);
		else bridge(kind, id, text, extra);
	}

	function done(ok, payload, writes) {
		if (finished) return;
		finished = true;
		if (ok) send("done", 0, writes, payload);
		else send("failed", 0, payload);
	}

	function serialize(value) {
		return value === undefined ? undefined : stringify(value);
	}

	// QuickJS stacks list frames only. Prefix "Name: message" like V8 so the text reads the
	// same as a Node error, and drop this prelude's frames.
	function errorText(error) {
		const head = error.message ? error.name + ": " + error.message : String(error.name);
		const frames =
			typeof error.stack === "string"
				? error.stack.split("\n").filter((line) => line.trim() && !line.includes("codemode-prelude.js"))
				: [];
		return [head, ...frames].join("\n");
	}

	function format(value) {
		if (typeof value === "string") return value;
		if (value instanceof ErrorCtor) return errorText(value);
		try {
			const json = stringify(value);
			return json === undefined ? String(value) : json;
		} catch {
			return String(value);
		}
	}

	function describeError(error) {
		if (error === EXIT) return stringify({ message: "exit" });
		// QuickJS throws null when it runs out of memory while making the error itself.
		if (error === null) {
			return stringify({ name: "InternalError", message: "out of memory (or the script threw null)" });
		}
		if (error instanceof ErrorCtor) {
			return stringify({ name: error.name, message: error.message, stack: errorText(error) });
		}
		return stringify({ message: format(error) });
	}

	function caller(kind, name, spread) {
		return (...args) =>
			new Promise((resolve, reject) => {
				if (finished) {
					reject(new ErrorCtor("The script has already finished"));
					return;
				}
				let json;
				try {
					json = serialize(spread ? args : args[0]);
				} catch (error) {
					reject(error);
					return;
				}
				const id = nextId++;
				pending.set(id, { resolve, reject });
				send(kind, id, name, json);
			});
	}

	// Tools known when the script starts, by identifier and by name. A name the script finds
	// later (a searchTools() match) is still callable: the proxy asks the host for it.
	const known = Object.create(null);
	const allTools = [];
	for (const { name, jsName, description } of parse(toolsJson)) {
		const fn = caller("call", name);
		// The first tool wins when two names normalize to the same identifier.
		if (!(jsName in known)) {
			known[jsName] = fn;
			allTools.push(Object.freeze({ name: jsName, description }));
		}
		if (!(name in known)) known[name] = fn;
	}
	Object.freeze(known);
	Object.freeze(allTools);
	const tools = new Proxy(known, {
		get(target, property) {
			if (typeof property !== "string") return undefined;
			if (property in target) return target[property];
			// Awaiting `tools` must not look like a call.
			if (property === "then") return undefined;
			return caller("call", property);
		},
	});

	const namespaces = new Map();
	for (const { name, spread } of parse(globalsJson)) {
		const fn = caller("global", name, spread);
		const dot = name.indexOf(".");
		if (dot === -1) {
			Object.defineProperty(globalThis, name, { value: fn, enumerable: true });
			continue;
		}
		const namespace = name.slice(0, dot);
		if (!namespaces.has(namespace)) namespaces.set(namespace, Object.create(null));
		namespaces.get(namespace)[name.slice(dot + 1)] = fn;
	}
	for (const [namespace, members] of namespaces) {
		Object.defineProperty(globalThis, namespace, { value: Object.freeze(members), enumerable: true });
	}

	// key -> JSON text. Sizes count key and JSON characters.
	const stored = new Map(Object.entries(parse(storeJson)));
	const writes = new Map();
	let storedChars = 0;
	for (const [key, json] of stored) storedChars += key.length + json.length;

	function checkKey(name, key) {
		if (typeof key !== "string") throw new TypeErrorCtor(name + "() key must be a string");
	}

	function store(key, value) {
		checkKey("store", key);
		const previous = stored.has(key) ? key.length + stored.get(key).length : 0;
		if (value === undefined) {
			stored.delete(key);
			storedChars -= previous;
			writes.set(key, undefined);
			return;
		}
		let json;
		try {
			json = stringify(value);
		} catch (error) {
			throw new TypeErrorCtor("store(" + stringify(key) + ") value is not JSON-serializable: " + format(error));
		}
		if (json === undefined) {
			throw new TypeErrorCtor("store(" + stringify(key) + ") value is not JSON-serializable");
		}
		if (json.length > limits.storeValueChars) {
			throw new RangeErrorCtor("store(" + stringify(key) + ") value exceeds " + limits.storeValueChars + " characters of JSON");
		}
		const next = storedChars - previous + key.length + json.length;
		if (next > limits.storeTotalChars) {
			throw new RangeErrorCtor("store is full: stored values would exceed " + limits.storeTotalChars + " characters of JSON");
		}
		stored.set(key, json);
		storedChars = next;
		writes.set(key, json);
	}

	function load(key) {
		checkKey("load", key);
		const json = stored.get(key);
		return json === undefined ? undefined : parse(json);
	}

	function serializeWrites() {
		const entries = [];
		for (const [key, json] of writes) entries.push(json === undefined ? [key] : [key, json]);
		return stringify(entries);
	}

	// Primitives become their string form, everything else JSON.
	function outputText(value) {
		if (value === undefined || value === null || (typeof value !== "object" && typeof value !== "function")) {
			return String(value);
		}
		const json = stringify(value);
		return json === undefined ? String(value) : json;
	}

	function text(value) {
		let rendered;
		try {
			rendered = outputText(value);
		} catch (error) {
			throw new TypeErrorCtor(error instanceof ErrorCtor ? error.message : String(error));
		}
		if (!finished) bridge("text", 0, rendered);
	}

	const IMAGE_EXPECTS = "image expects a non-empty image URL string, an object with image_url, or an MCP image block";

	function imageUrl(value) {
		if (typeof value === "string") return value;
		if (typeof value !== "object" || value === null || Array.isArray(value)) throw new TypeErrorCtor(IMAGE_EXPECTS);
		if (value.image_url !== undefined) {
			if (typeof value.image_url !== "string") throw new TypeErrorCtor(IMAGE_EXPECTS);
			return value.image_url;
		}
		if (typeof value.type !== "string") throw new TypeErrorCtor(IMAGE_EXPECTS);
		if (value.type !== "image") throw new TypeErrorCtor('image only accepts MCP image blocks, got "' + value.type + '"');
		if (typeof value.data !== "string" || value.data === "") throw new TypeErrorCtor("image expected MCP image data");
		if (value.data.toLowerCase().startsWith("data:")) return value.data;
		const mimeType = typeof value.mimeType === "string" && value.mimeType ? value.mimeType : "application/octet-stream";
		return "data:" + mimeType + ";base64," + value.data;
	}

	function image(value) {
		const url = imageUrl(value);
		if (url === "") throw new TypeErrorCtor(IMAGE_EXPECTS);
		const colon = url.indexOf(":");
		const scheme = colon === -1 ? "" : url.slice(0, colon).toLowerCase();
		if (scheme === "http" || scheme === "https") {
			throw new TypeErrorCtor("remote image URLs are not supported in tool outputs. Pass a base64 data URI instead");
		}
		const comma = url.indexOf(",");
		const header = comma === -1 ? [] : url.slice(colon + 1, comma).split(";");
		if (scheme !== "data" || comma === -1 || header.slice(1).every((part) => part.toLowerCase() !== "base64")) {
			throw new TypeErrorCtor("invalid image output. Pass a base64 data URI instead");
		}
		if (!finished) bridge("image", 0, url.slice(comma + 1), header[0] || "application/octet-stream");
	}

	function exit() {
		let writesJson;
		try {
			writesJson = serializeWrites();
		} catch (error) {
			done(false, describeError(error));
			throw EXIT;
		}
		done(true, undefined, writesJson);
		throw EXIT;
	}

	const console = {};
	for (const level of ["log", "info", "warn", "error", "debug"]) {
		console[level] = (...args) => {
			if (!finished) bridge("text", 0, args.map(format).join(" "));
		};
	}
	Object.freeze(console);

	Object.defineProperty(globalThis, "tools", { value: tools, enumerable: true });
	Object.defineProperty(globalThis, "ALL_TOOLS", { value: allTools, enumerable: true });
	Object.defineProperty(globalThis, "console", { value: console, enumerable: true });
	Object.defineProperty(globalThis, "text", { value: text, enumerable: true });
	Object.defineProperty(globalThis, "image", { value: image, enumerable: true });
	Object.defineProperty(globalThis, "exit", { value: exit, enumerable: true });
	Object.defineProperty(globalThis, "store", { value: store, enumerable: true });
	Object.defineProperty(globalThis, "load", { value: load, enumerable: true });

	return {
		settle(id, ok, hasPayload, payload) {
			const entry = pending.get(id);
			if (!entry) return;
			pending.delete(id);
			if (!ok) {
				entry.reject(new ErrorCtor(payload));
				return;
			}
			let value;
			try {
				value = hasPayload ? parse(payload) : undefined;
			} catch (error) {
				entry.reject(error);
				return;
			}
			entry.resolve(value);
		},
		run(fn) {
			let promise;
			try {
				promise = fn(tools, console);
			} catch (error) {
				if (error !== EXIT) done(false, describeError(error));
				return;
			}
			promiseThen.call(
				promise,
				(value) => {
					let json;
					let writesJson;
					try {
						json = serialize(value);
						writesJson = serializeWrites();
					} catch (error) {
						done(false, describeError(error));
						return;
					}
					done(true, json, writesJson);
				},
				(error) => {
					if (error !== EXIT) done(false, describeError(error));
				},
			);
		},
		stalled() {
			if (finished) return "finished";
			if (pending.size > 0) return "waiting";
			done(
				false,
				stringify({
					name: "Error",
					message:
						"The script is waiting on a promise that can never settle: no tool call is pending, and timers do not exist here.",
				}),
			);
			return "stalled";
		},
	};
})
