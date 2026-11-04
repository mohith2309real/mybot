## 2024-11-04 - SQLite compilation overhead
**Learning:** node:sqlite `db.prepare(sql)` recompiles the query every single time it is called. Given the current code in `src/db/index.ts` repeatedly uses `openDb().prepare(...)` for queries that are executed often (like `getTask`, `getBot`), it pays off significantly to cache the prepared `StatementSync` objects instead of recompiling on every call.
**Action:** Use a `Map` to cache statements by their query string. Wrap `db.prepare(sql)` with a helper `prepared(sql)` which caches the compiled StatementSync object.
