## 2024-05-24 - [SQLite Node Compilation Overhead]
**Learning:** node:sqlite natively recompiles statements on every `db.prepare(sql)` call, which adds significant overhead for repetitive dynamic queries.
**Action:** Always cache the compiled `StatementSync` objects using a Map when doing repeated database operations to improve execution performance.
