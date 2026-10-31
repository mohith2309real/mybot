## 2024-03-24 - [Optimize SQLite JSON append]
**Learning:** Using SQLite's built-in `json_insert(..., '$[#]', ...)` is significantly faster (~3x in benchmarks) than fetching the JSON column, parsing it in Node.js, appending the item, stringifying it, and updating the column. This avoids large string transfers across the JS/C++ boundary and redundant V8 JSON operations.
**Action:** Use SQLite native JSON functions for manipulating JSON columns instead of doing read-modify-write loops in JavaScript.
