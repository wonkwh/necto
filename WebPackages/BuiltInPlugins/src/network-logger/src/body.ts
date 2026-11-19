//
//  Copyright (c) 2026 Viva Republica, Inc.
//

export type JSONTokenKind = "key" | "string" | "number" | "literal" | "plain";

/// Parses a body as JSON when it says it is, or looks like it is.
/// - Returns: The parsed value, or `undefined` when the body is not JSON.
export function parseJSONBody(text: string, contentType: string | undefined): unknown {
  const trimmed = text.trimStart();
  if (!contentType?.includes("json") && !trimmed.startsWith("{") && !trimmed.startsWith("[")) return undefined;
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

/// Splits JSON text into coloured runs; everything between them is `plain`.
/// - Parameter text: JSON as produced by `JSON.stringify`.
/// - Returns: Runs whose texts join back into `text`.
export function jsonTokens(text: string): { text: string; kind: JSONTokenKind }[] {
  const pattern = /("(?:\\.|[^"\\])*")(\s*:)?|\b(?:true|false|null)\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g;
  const tokens: { text: string; kind: JSONTokenKind }[] = [];
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    const index = match.index;
    if (index > last) tokens.push({ text: text.slice(last, index), kind: "plain" });
    if (match[1] !== undefined) {
      tokens.push({ text: match[1], kind: match[2] ? "key" : "string" });
      if (match[2]) tokens.push({ text: match[2], kind: "plain" });
    } else {
      tokens.push({ text: match[0], kind: /^[tfn]/.test(match[0]) ? "literal" : "number" });
    }
    last = index + match[0].length;
  }
  if (last < text.length) tokens.push({ text: text.slice(last), kind: "plain" });
  return tokens;
}

/// Reads a body as `key=value&…` when it says it is a form, or looks like one.
/// - Returns: The pairs in order, or `undefined` when the body is not a form.
export function parseFormBody(text: string, contentType: string | undefined): [string, string][] | undefined {
  const trimmed = text.trim();
  const looksLikeForm = /^[^\s=&{[]+=[^\s&]*(?:&[^\s=&]+=[^\s&]*)*$/.test(trimmed);
  if (!contentType?.includes("x-www-form-urlencoded") && !looksLikeForm) return undefined;
  const entries = [...new URLSearchParams(trimmed)];
  return entries.length ? entries : undefined;
}
