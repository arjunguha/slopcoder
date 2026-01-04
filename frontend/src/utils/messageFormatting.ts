export type PrettyPrintResult = {
  text: string;
  clipped: boolean;
};

type PrettyPrintOptions = {
  maxStringLength: number;
  maxArrayLength: number;
  maxObjectKeys: number;
  maxDepth: number;
};

const DEFAULT_OPTIONS: PrettyPrintOptions = {
  maxStringLength: 200,
  maxArrayLength: 25,
  maxObjectKeys: 25,
  maxDepth: 6,
};

export function parseJson(value?: string): unknown | null {
  if (!value) return null;
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}

export function clipText(value: string, maxLines: number, maxChars: number) {
  const lines = value.split(/\r?\n/);
  const limitedLines = lines.slice(0, maxLines);
  let clipped = lines.length > maxLines;
  let text = limitedLines.join("\n");
  if (text.length > maxChars) {
    text = text.slice(0, maxChars);
    clipped = true;
  }
  return { text, clipped };
}

export function summarizeOutput(output?: string) {
  if (!output) return "No output";
  const lines = output.split(/\r?\n/);
  const lineCount = lines[lines.length - 1] === "" ? lines.length - 1 : lines.length;
  const size = output.length;
  return `${lineCount} line${lineCount === 1 ? "" : "s"}, ${size} char${size === 1 ? "" : "s"}`;
}

export function summarizeJsonShape(value?: string) {
  const parsed = parseJson(value);
  if (parsed === null) return null;
  if (Array.isArray(parsed)) {
    return `JSON array (${parsed.length} item${parsed.length === 1 ? "" : "s"})`;
  }
  if (parsed && typeof parsed === "object") {
    const keys = Object.keys(parsed);
    const preview = keys.slice(0, 4).join(", ");
    return `JSON object (${keys.length} key${keys.length === 1 ? "" : "s"}${preview ? `: ${preview}` : ""})`;
  }
  return `JSON ${typeof parsed}`;
}

function truncateString(value: string, maxLength: number) {
  if (value.length <= maxLength) return { text: value, clipped: false };
  return { text: `${value.slice(0, maxLength)}...`, clipped: true };
}

function sanitizeJsonValue(
  value: unknown,
  options: PrettyPrintOptions,
  depth: number,
  seen: WeakSet<object>
): { value: unknown; clipped: boolean } {
  if (value === null || value === undefined) return { value, clipped: false };
  if (typeof value === "string") {
    const trimmed = truncateString(value, options.maxStringLength);
    return { value: trimmed.text, clipped: trimmed.clipped };
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return { value, clipped: false };
  }
  if (typeof value === "bigint") {
    return { value: value.toString(), clipped: false };
  }
  if (typeof value !== "object") {
    return { value: String(value), clipped: false };
  }
  if (seen.has(value)) {
    return { value: "[Circular]", clipped: true };
  }
  if (depth >= options.maxDepth) {
    return { value: "[Max depth reached]", clipped: true };
  }

  seen.add(value);

  if (Array.isArray(value)) {
    let clipped = false;
    const entries = value.slice(0, options.maxArrayLength);
    const sanitized = entries.map((entry) => {
      const result = sanitizeJsonValue(entry, options, depth + 1, seen);
      if (result.clipped) clipped = true;
      return result.value;
    });
    if (value.length > options.maxArrayLength) {
      sanitized.push(`... ${value.length - options.maxArrayLength} more items`);
      clipped = true;
    }
    return { value: sanitized, clipped };
  }

  let clipped = false;
  const obj: Record<string, unknown> = {};
  const entries = Object.entries(value as Record<string, unknown>);
  for (const [key, entry] of entries.slice(0, options.maxObjectKeys)) {
    const result = sanitizeJsonValue(entry, options, depth + 1, seen);
    if (result.clipped) clipped = true;
    obj[key] = result.value;
  }
  if (entries.length > options.maxObjectKeys) {
    obj["..."] = `${entries.length - options.maxObjectKeys} more keys`;
    clipped = true;
  }
  return { value: obj, clipped };
}

export function prettyPrintJsonValue(
  value: unknown,
  options: PrettyPrintOptions = DEFAULT_OPTIONS
): PrettyPrintResult {
  const { value: sanitized, clipped } = sanitizeJsonValue(value, options, 0, new WeakSet());
  let text = "";
  try {
    text = JSON.stringify(sanitized, null, 2) ?? "";
  } catch {
    text = String(sanitized);
  }
  return { text, clipped };
}

export function prettyPrintJsonString(
  value?: string,
  options: PrettyPrintOptions = DEFAULT_OPTIONS
): PrettyPrintResult | null {
  const parsed = parseJson(value);
  if (parsed === null) return null;
  return prettyPrintJsonValue(parsed, options);
}
