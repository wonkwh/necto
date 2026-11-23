//
//  Copyright (c) 2026 Viva Republica, Inc.
//

/// Builds a predicate from the filter field.
///
/// Terms are comma separated and all must hold. `!term` excludes, `/pattern/` is a
/// case-insensitive regular expression, anything else a case-insensitive substring.
/// - Parameter query: The raw text of the filter field.
/// - Returns: A predicate over the text a record is matched against.
export function makeFilter(query: string): (text: string) => boolean {
  // ponytail: a comma always splits, so a regex can't contain one (`/a{1,3}/`).
  const terms = query
    .split(",")
    .map((raw) => raw.trim())
    .filter(Boolean)
    .map((raw) => {
      const negated = raw.startsWith("!");
      const body = negated ? raw.slice(1).trim() : raw;
      return { negated, test: termTest(body) };
    })
    .filter(({ test }) => test !== null);
  return (text) => terms.every(({ negated, test }) => test!(text) !== negated);
}

function termTest(body: string): ((text: string) => boolean) | null {
  if (!body) return null;
  const regex = body.length > 2 && body.startsWith("/") && body.endsWith("/") ? compile(body.slice(1, -1)) : null;
  if (regex) return (text) => regex.test(text);
  const needle = body.toLowerCase();
  return (text) => text.toLowerCase().includes(needle);
}

/// An unfinished pattern, typed one key at a time, falls back to a substring match.
function compile(pattern: string): RegExp | null {
  try {
    return new RegExp(pattern, "i");
  } catch {
    return null;
  }
}
