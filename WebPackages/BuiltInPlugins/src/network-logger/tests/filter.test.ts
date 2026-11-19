//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import { assert, test } from "vitest";

import { makeFilter } from "../src/filter";

const urls = [
  "GET http://192.168.111.1/api/keepalive",
  "POST http://192.168.111.1/api/login",
  "GET https://example.com/users/42",
];

const matching = (query: string) => urls.filter(makeFilter(query));

test("comma separated terms must all hold and ! excludes", () => {
  assert.deepEqual(matching("192.168.111.1, !keepalive"), [urls[1]]);
  assert.deepEqual(matching("!192.168.111.1"), [urls[2]]);
  assert.deepEqual(matching(""), urls);
  assert.deepEqual(matching(" , ! ,"), urls);
});

test("slashes make a case-insensitive regular expression", () => {
  assert.deepEqual(matching("/users/\\d+$/"), [urls[2]]);
  assert.deepEqual(matching("/^(get|post) http:/"), [urls[0], urls[1]]);
  assert.deepEqual(matching("!/LOGIN|keepalive/"), [urls[2]]);
});

test("an invalid pattern falls back to a substring match", () => {
  assert.deepEqual(matching("/api/("), []);
  assert.deepEqual(matching("/api/"), [urls[0], urls[1]]);
});
