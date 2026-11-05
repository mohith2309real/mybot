## 2023-11-05 - node:sqlite Query Recompilation
**Learning:** In this codebase's architecture, repeated execution of dynamic queries using `node:sqlite` directly triggers significant performance overhead due to constant recompilation via `db.prepare(sql)`.
**Action:** When using `node:sqlite` for repeated query patterns, wrap the `prepare` calls to cache and reuse the compiled `StatementSync` objects. The database object itself can have its `prepare` method monkey-patched to transparently provide caching for all queries.
