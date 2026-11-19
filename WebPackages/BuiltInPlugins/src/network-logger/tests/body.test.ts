//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import { assert, test } from "vitest";

import { jsonTokens, parseFormBody, parseJSONBody } from "../src/body";

test("bodies are parsed when the type says JSON or the text looks like it", () => {
  assert.deepEqual(parseJSONBody('{"a":1}', "application/json"), { a: 1 });
  assert.deepEqual(parseJSONBody(' [1]', "text/plain"), [1]);
  assert.equal(parseJSONBody("42", undefined), undefined);
  assert.equal(parseJSONBody("{broken", "application/json"), undefined);
});

test("tokens colour keys, values and literals and rejoin to the input", () => {
  const text = JSON.stringify({ "na\"me": "a:b", n: -1.5e3, ok: true, none: null }, null, 2);
  const tokens = jsonTokens(text);
  assert.equal(tokens.map(({ text }) => text).join(""), text);
  const kinds = tokens.filter(({ kind }) => kind !== "plain").map(({ text, kind }) => `${kind}:${text}`);
  assert.deepEqual(kinds, [
    'key:"na\\"me"', 'string:"a:b"',
    'key:"n"', "number:-1500",
    'key:"ok"', "literal:true",
    'key:"none"', "literal:null",
  ]);
});

test("form bodies become ordered pairs", () => {
  assert.deepEqual(parseFormBody("command=recstop&param=2", undefined), [["command", "recstop"], ["param", "2"]]);
  assert.deepEqual(parseFormBody("q=a+b&x=%26", "application/x-www-form-urlencoded"), [["q", "a b"], ["x", "&"]]);
  assert.equal(parseFormBody('{"a":1}', "application/json"), undefined);
  assert.equal(parseFormBody("just text", "text/plain"), undefined);
});
