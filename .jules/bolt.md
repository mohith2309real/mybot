## 2024-03-24 - [SQLite Statement Caching]
**Learning:** node:sqlite `db.prepare()` recompiles the SQL string into a statement object every time it's called. For a database-heavy application, this repeated recompilation overhead on hot paths (like getting/updating tasks and routines) degrades performance.
**Action:** Use an explicit, bounded LRU/clear-on-full Map to cache `StatementSync` objects by their SQL string. Always use `cachedPrepare(sql)` instead of `db.prepare(sql)` for repeated queries.
