## 2024-11-09 - Caching SQLite Statements
**Learning:** node:sqlite `DatabaseSync.prepare()` method has compilation overhead that should be avoided on hot paths by caching `StatementSync` objects. Using a global map is dangerous when multiple database instances exist.
**Action:** Use a `WeakMap<DatabaseSync, Map<string, StatementSync>>` to safely cache statements tied to the lifetime of each specific `DatabaseSync` instance.
