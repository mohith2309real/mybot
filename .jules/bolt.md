## 2025-02-18 - Database Performance (node:sqlite)
**Learning:** Cache compiled `StatementSync` objects to avoid recompilation overhead for repeated queries. Use an explicit, bounded `cachedPrepare` helper function (e.g., clearing the Map at 500 entries to prevent memory leaks) rather than globally monkey-patching `db.prepare` or using Proxies, which introduce CPU overhead and shared state mutation risks.
**Action:** Always implement a LRU or size-bounded map to store compiled statements and reuse them for database operations.
