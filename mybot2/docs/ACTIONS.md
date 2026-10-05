# Actions (249)

Everything a teammate can do besides its core tools. Models find these with `actions_search` and run them with `action_run`.

- **local**: runs inside MyBot, no computer needed (every one has an example the test suite executes)
- **computer**: runs in the bot's container (files, archives, web requests, programs)
- **browser**: drives the bot's own browser
- **meta**: notes, memory and skills

*Needs OK* means the bot asks you first, unless your request already said to.


## Archives (5)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `tar_create` | computer | `output`, `source` | Create a .tar.gz from a file or folder |  |
| `tar_extract` | computer | `archive`, `to` | Extract a .tar / .tar.gz into a folder |  |
| `zip_create` | computer | `output`, `source` | Create a .zip from a file or folder |  |
| `zip_extract` | computer | `archive`, `to` | Extract a .zip (refuses paths that escape the target folder) |  |
| `zip_list` | computer | `archive` | List the contents of a .zip |  |

## Browser (43)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `element_attribute` | browser | `name`, `ref` | An attribute of one element (href, src, aria-label…) |  |
| `element_text` | browser | `ref` | The text of one element |  |
| `page_back` | browser |  | Go back one page in history |  |
| `page_clear_field` | browser | `ref` | Empty a text field |  |
| `page_cookie_names` | browser |  | Names and domains of this page's cookies (never their values) |  |
| `page_count_elements` | browser | `selector` | Count elements matching a CSS selector |  |
| `page_double_click` | browser | `ref` | Double-click an element |  |
| `page_find_text` | browser | `text` | Find text on the page: matching snippets and the nearest element refs |  |
| `page_focus` | browser | `ref` | Move keyboard focus to an element |  |
| `page_forms` | browser |  | Forms and their fields (names, types, labels — never values of password fields) |  |
| `page_forward` | browser |  | Go forward one page in history |  |
| `page_go_to_ref_link` | browser | `ref` | Open the URL of a link ref in a new tab |  |
| `page_headings` | browser |  | The page's heading outline (h1–h6) |  |
| `page_highlight_ref` | browser | `ref` | Outline an element so a watching human can see it |  |
| `page_hover` | browser | `ref` | Hover the mouse over an element |  |
| `page_images` | browser |  | Images on the page: src, alt text and size |  |
| `page_links` | browser | `same_site`? | Every link on the page: text and URL |  |
| `page_list_items` | browser | `selector`? | Text of list items (li) on the page |  |
| `page_meta` | browser |  | Meta tags, canonical URL and JSON-LD structured data |  |
| `page_performance` | browser |  | Load timing and resource counts for the page |  |
| `page_press_key` | browser | `key` | Press a navigation key: Enter, Tab, Escape, ArrowDown, PageDown, Home, End… |  |
| `page_reload` | browser |  | Reload the current page |  |
| `page_reset_viewport` | browser |  | Back to the normal window size |  |
| `page_right_click` | browser | `ref` | Right-click an element |  |
| `page_save_pdf` | browser | `path`? | Save the page as a PDF into /workspace |  |
| `page_screenshot` | browser | `path`? | Save a screenshot of the page into /workspace/screenshots |  |
| `page_scroll_to_bottom` | browser |  | Scroll to the bottom (loads lazy content) |  |
| `page_scroll_to_ref` | browser | `ref` | Scroll an element into view |  |
| `page_scroll_to_top` | browser |  | Scroll to the top |  |
| `page_select_option` | browser | `option`, `ref` | Choose an option in a <select> by its visible text or value |  |
| `page_set_checkbox` | browser | `checked`, `ref` | Tick or untick a checkbox / radio |  |
| `page_set_viewport` | browser | `height`, `width` | Emulate a screen size (e.g. 390x844 for a phone) |  |
| `page_tables` | browser | `index`? | Tables on the page as CSV |  |
| `page_text` | browser | `max_chars`? | All visible text of the page (up to N characters) |  |
| `page_title` | browser |  | The current page's title |  |
| `page_upload_file` | browser | `path`, `ref` | Attach a file from /workspace to a file input |  |
| `page_url` | browser |  | The current page's URL |  |
| `page_wait_for_text` | browser | `seconds`?, `text` | Wait until text appears on the page |  |
| `page_word_count` | browser |  | Words of visible text on the page |  |
| `tab_close` | browser | `index` | Close a tab by index |  |
| `tab_list` | browser |  | List open tabs |  |
| `tab_new` | browser | `url`? | Open a new tab (optionally at a URL) |  |
| `tab_switch` | browser | `index` | Switch to a tab by index from tab_list |  |

## Data (36)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `base64_decode` | local | `text` | Decode base64 to text |  |
| `base64_encode` | local | `text`, `url_safe`? | Base64-encode text |  |
| `csv_columns` | local | `csv`, `delimiter`? | List CSV columns with types and sample values |  |
| `csv_dedupe` | local | `columns`?, `csv`, `delimiter`? | Remove duplicate rows (optionally by some columns) |  |
| `csv_filter` | local | `column`, `csv`, `delimiter`?, `op`, `value` | Keep rows where a column matches (=, !=, >, <, contains) |  |
| `csv_group_sum` | local | `csv`, `delimiter`?, `group_by`, `value` | Group by a column and count/sum/average another |  |
| `csv_row_count` | local | `csv`, `delimiter`? | Count data rows and columns |  |
| `csv_select_columns` | local | `columns`, `csv`, `delimiter`? | Keep only some columns, in a given order |  |
| `csv_sort` | local | `column`, `csv`, `delimiter`?, `descending`? | Sort rows by a column (numbers sort numerically) |  |
| `csv_stats` | local | `csv`, `delimiter`? | Count, min, max, mean, median and sum for numeric columns |  |
| `csv_to_json` | local | `csv`, `delimiter`? | Turn CSV into a JSON array of objects |  |
| `csv_to_markdown_table` | local | `csv`, `delimiter`? | Render CSV as a Markdown table |  |
| `csv_transpose` | local | `csv`, `delimiter`? | Swap rows and columns |  |
| `hex_decode` | local | `text` | Decode hex to text |  |
| `hex_encode` | local | `text` | Hex-encode text |  |
| `json_diff` | local | `a`, `b` | List the differences between two JSON documents |  |
| `json_flatten` | local | `json` | Flatten nested JSON into dot.paths |  |
| `json_keys` | local | `json`, `path`? | List the keys of a JSON object (at an optional path) |  |
| `json_merge` | local | `a`, `b` | Deep-merge two JSON objects (second wins) |  |
| `json_minify` | local | `json` | Minify JSON |  |
| `json_pretty` | local | `json` | Pretty-print JSON |  |
| `json_query` | local | `json`, `path` | Get a value by path, e.g. items[0].name |  |
| `json_to_csv` | local | `json` | Turn a JSON array of objects into CSV |  |
| `json_to_toml` | local | `json` | Convert a JSON object to TOML |  |
| `json_unflatten` | local | `json` | Turn dot.path keys back into nested JSON |  |
| `json_validate` | local | `json` | Check that text is valid JSON and say where it breaks |  |
| `list_chunk` | local | `items`, `size` | Split a list into groups of N |  |
| `list_compare` | local | `a`, `b` | Items only in A, only in B, and in both |  |
| `list_sort` | local | `descending`?, `items` | Sort a list |  |
| `list_unique` | local | `items` | Unique items of a list, in order |  |
| `query_string_build` | local | `params` | Build a query string from JSON key/values |  |
| `toml_to_json` | local | `toml` | Convert TOML to JSON |  |
| `url_decode` | local | `text` | Decode percent-encoded text |  |
| `url_encode` | local | `text` | Percent-encode text for a URL |  |
| `url_join` | local | `base`, `link` | Resolve a relative link against a base URL |  |
| `url_parse` | local | `url` | Split a URL into scheme, host, path, query and fragment |  |

## Dates & Times (17)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `add_business_days` | local | `date`, `days` | Add working days (Mon–Fri) to a date |  |
| `age_from_birthdate` | local | `birthdate`, `on`? | Age in years (and days to the next birthday) |  |
| `business_days_between` | local | `from`, `to` | Working days (Mon–Fri) between two dates, excluding the start |  |
| `cron_next_runs` | local | `count`?, `schedule` | The next times a cron schedule fires |  |
| `date_add` | local | `amount`, `date`, `unit` | Add days, weeks, months or years to a date |  |
| `date_diff` | local | `from`, `to` | Days (and weeks, months) between two dates |  |
| `date_format` | local | `date`, `pattern` | Reformat a date (strftime pattern like %d %B %Y) |  |
| `date_now` | local |  | Current date and time (local and UTC) |  |
| `date_to_unix` | local | `datetime` | Convert a date/time (UTC) to a Unix timestamp |  |
| `day_of_week` | local | `date` | Which weekday a date falls on |  |
| `days_until` | local | `date` | Days from today until a date |  |
| `duration_format` | local | `value` | Turn seconds into h/m/s (or parse 1h30m into seconds) |  |
| `is_leap_year` | local | `year` | Whether a year is a leap year |  |
| `month_calendar` | local | `month`, `year` | A text calendar for a month |  |
| `timezone_convert` | local | `datetime`, `from_offset`, `to_offset` | Convert a time between UTC offsets (e.g. +05:30 to -04:00) |  |
| `unix_to_date` | local | `timestamp` | Convert a Unix timestamp (seconds or ms) to a date |  |
| `week_number` | local | `date` | ISO week number and year of a date |  |

## Files (30)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `browser_downloads` | computer |  | List files the browser downloaded |  |
| `count_files` | computer | `path`? | Count files by extension in a folder |  |
| `csv_preview` | computer | `path`, `rows`? | Show the first rows of a CSV file as a table |  |
| `file_append` | computer | `content`, `path` | Append text to a file |  |
| `file_copy` | computer | `from`, `to` | Copy a file or folder (never overwrites) |  |
| `file_delete` | computer | `path` | Delete a file or folder (asks the human unless the task said to delete) | delete or overwrite data |
| `file_diff` | computer | `a`, `b` | Unified diff of two files |  |
| `file_find` | computer | `path`?, `pattern` | Find files by name pattern |  |
| `file_hash` | computer | `path` | SHA-256 checksum of a file |  |
| `file_head` | computer | `lines`?, `path` | First N lines of a file |  |
| `file_info` | computer | `path` | Size, type, dates and permissions of a file |  |
| `file_line_count` | computer | `path` | Count lines, words and bytes |  |
| `file_list` | computer | `path`? | List a folder with sizes and dates |  |
| `file_move` | computer | `from`, `to` | Move or rename a file (never overwrites) |  |
| `file_read` | computer | `lines`?, `path` | Read a text file (first 400 lines by default) |  |
| `file_replace_text` | computer | `find`, `path`, `replace` | Find and replace text inside a file (keeps a .bak copy) |  |
| `file_search_text` | computer | `path`?, `text` | Search inside files for text (grep) |  |
| `file_tail` | computer | `lines`?, `path` | Last N lines of a file |  |
| `file_tree` | computer | `depth`?, `path`? | Show a folder tree (depth 3) |  |
| `file_write` | computer | `content`, `path` | Write text to a file, creating folders (overwrites) |  |
| `find_duplicates` | computer | `path`? | Find files with identical content |  |
| `folder_size` | computer | `path`? | Size of a folder and its biggest items |  |
| `image_info` | computer | `path` | Format and pixel size of an image file |  |
| `json_file_query` | computer | `filter`, `path` | Query a JSON file with a jq filter |  |
| `lines_matching` | computer | `ignore_case`?, `path`, `pattern` | Lines of a file that match a regex (with line numbers) |  |
| `make_folder` | computer | `path` | Create a folder (and parents) |  |
| `markdown_file_to_html` | computer | `path` | Convert a Markdown file to an HTML file next to it |  |
| `open_in_editor_view` | computer | `from`?, `lines`?, `path` | Show a file with line numbers |  |
| `pdf_to_text` | computer | `path` | Extract text from a PDF (pdftotext if installed, else a basic reader) |  |
| `touch_timestamp` | computer | `path` | Create a file or update its modified time |  |

## Hashing & IDs (16)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `crc32` | local | `text` | CRC-32 checksum of text |  |
| `hash_md5` | local | `text` | MD5 hash of text (hex) — for checksums, not security |  |
| `hash_sha1` | local | `text` | SHA-1 hash of text (hex) — for checksums, not security |  |
| `hash_sha256` | local | `text` | SHA-256 hash of text (hex) |  |
| `hash_sha512` | local | `text` | SHA-512 hash of text (hex) |  |
| `hmac_sha256` | local | `key`, `message` | HMAC-SHA256 of a message with a key (hex) |  |
| `jwt_decode` | local | `token` | Show a JWT's header and claims (does NOT verify the signature) |  |
| `luhn_check` | local | `number` | Check a number's Luhn checksum (IDs, IMEIs) |  |
| `password_strength` | local | `password` | Estimate a password's strength without storing or sending it |  |
| `random_number` | local | `max`, `min` | Random integer between min and max (inclusive) |  |
| `random_password` | local | `length`?, `symbols`? | Generate a strong random password for a new account |  |
| `random_pick` | local | `count`?, `items` | Pick random items from a list |  |
| `random_string` | local | `length`? | Random letters and digits |  |
| `rot13` | local | `text` | ROT13 a text (a toy cipher, for puzzles) |  |
| `uuid_v4` | local | `count`? | Generate random UUIDs |  |
| `uuid_validate` | local | `text` | Check whether text is a valid UUID |  |

## Library (4)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `site_info` | meta | `site` | Where a site lives and where you sign in |  |
| `site_search` | meta | `query` | List known sites in a category or matching a word |  |
| `skill_search` | meta | `query` | Search the skill library (built-in, yours and imported) |  |
| `skill_show` | meta | `name` | Show a skill's full instructions |  |

## Math (26)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `aspect_ratio` | local | `height`, `new_width`?, `width` | Simplify a width:height ratio and fit a new size |  |
| `average` | local | `numbers` | Average (mean) of numbers |  |
| `base_convert` | local | `from`, `to`, `value` | Convert an integer between bases 2–36 |  |
| `bmi` | local | `height_cm`, `weight_kg` | Body-mass index from weight (kg) and height (cm) — a rough screening number only |  |
| `break_even` | local | `fixed_costs`, `price`, `unit_cost` | Units needed to break even |  |
| `calculate` | local | `expression` | Evaluate arithmetic: + - * / % ^, parentheses, sqrt, round(x, d), min, max… |  |
| `compound_interest` | local | `annual_rate`, `monthly_contribution`?, `principal`, `years` | Future value with compound interest and optional monthly contributions |  |
| `currency_format` | local | `amount`, `currency` | Format an amount as currency |  |
| `discount_price` | local | `discount_percent`, `price`, `tax_percent`? | Price after a percentage discount (and tax) |  |
| `gcd_lcm` | local | `a`, `b` | Greatest common divisor and least common multiple |  |
| `loan_payment` | local | `annual_rate`, `principal`, `years` | Monthly payment, total paid and total interest for a loan |  |
| `number_format` | local | `decimals`?, `value` | Format a number with thousands separators |  |
| `number_to_words` | local | `n` | Write an integer in English words |  |
| `percent_change` | local | `new`, `old` | Percent change from an old value to a new one |  |
| `percent_of` | local | `percent`, `value` | What is P% of X |  |
| `percent_ratio` | local | `part`, `whole` | X is what percent of Y |  |
| `percentile` | local | `numbers`, `p` | The p-th percentile of numbers |  |
| `prime_check` | local | `n` | Is a number prime (and its smallest factor if not) |  |
| `ratio_scale` | local | `from`, `to`, `value` | Scale a quantity by a ratio (recipes, resizing): value × to / from |  |
| `roman_numerals` | local | `value` | Convert between numbers and Roman numerals |  |
| `round_number` | local | `decimals`?, `value` | Round to a number of decimal places |  |
| `sales_tax` | local | `amount`, `mode`?, `rate` | Add or remove tax from an amount |  |
| `statistics` | local | `numbers` | Mean, median, mode, standard deviation, min, max of numbers |  |
| `sum` | local | `numbers` | Sum of numbers |  |
| `tip_split` | local | `bill`, `people`?, `tip_percent`? | Tip and per-person share of a bill |  |
| `unit_convert` | local | `from`, `to`, `value` | Convert between units (length, mass, volume, temperature, speed, data, time, area, energy) |  |

## Notes & Memory (8)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `memory_forget` | meta | `key` | Forget a remembered fact |  |
| `memory_recall` | meta | `key`? | Look up remembered facts (all, or one key) |  |
| `memory_save` | meta | `key`, `value` | Remember a fact for later runs (key → value, kept in /workspace/.mybot/memory.json) |  |
| `note_add` | meta | `text` | Append a timestamped note to /workspace/notes.md |  |
| `notes_read` | meta |  | Read /workspace/notes.md |  |
| `todo_add` | meta | `item` | Add an item to /workspace/todo.md |  |
| `todo_done` | meta | `number` | Tick off a to-do item by its number |  |
| `todo_list` | meta |  | Show /workspace/todo.md |  |

## Run (4)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `list_desktops` | meta |  | Desktops on this computer, one per bot |  |
| `progress_note` | meta | `text` | Post a short progress update to the human's thread |  |
| `task_info` | meta |  | This run's bot, task id and original instruction |  |
| `wait_seconds` | meta | `seconds` | Wait a few seconds (max 120) for something to finish |  |

## Run & System (8)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `disk_free` | computer |  | Free disk space |  |
| `env_tools` | computer |  | Which common tools are installed |  |
| `git_status` | computer | `path` | git status and recent commits of a repository |  |
| `process_list` | computer |  | Running processes (top by CPU) |  |
| `python_run` | computer | `code` | Run a Python 3 script inside the computer |  |
| `sqlite_query` | computer | `path`, `sql` | Run a read-only SQL query on a SQLite file |  |
| `system_info` | computer |  | OS, CPU, memory and tool versions in the computer |  |
| `wait_for_file` | computer | `path`, `seconds`? | Wait (up to N seconds) for a file to appear, e.g. a download |  |

## Text (44)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `extract_dates` | local | `text` | Pull out date-like strings |  |
| `extract_emails` | local | `text` | Pull out every email address |  |
| `extract_hashtags` | local | `text` | Pull out #hashtags |  |
| `extract_mentions` | local | `text` | Pull out @mentions |  |
| `extract_numbers` | local | `text` | Pull out every number |  |
| `extract_phone_numbers` | local | `text` | Pull out phone-number-like strings |  |
| `extract_urls` | local | `text` | Pull out every URL |  |
| `html_escape` | local | `text` | Escape text for safe inclusion in HTML |  |
| `html_strip_tags` | local | `html` | Strip HTML tags (and scripts/styles) to plain text |  |
| `html_unescape` | local | `text` | Turn HTML entities back into characters |  |
| `markdown_to_html` | local | `markdown` | Convert Markdown to HTML |  |
| `markdown_toc` | local | `markdown` | Build a table of contents from Markdown headings |  |
| `regex_find` | local | `pattern`, `text` | Find all regex matches (with capture groups) |  |
| `regex_replace` | local | `pattern`, `replace`, `text` | Replace regex matches ($1 for groups) |  |
| `regex_test` | local | `pattern`, `text` | Test which lines match a regex |  |
| `text_camel_case` | local | `text` | Convert to camelCase |  |
| `text_char_frequency` | local | `text` | Count each character |  |
| `text_count_occurrences` | local | `phrase`, `text` | Count how often a phrase appears |  |
| `text_dedupe_lines` | local | `text` | Remove duplicate lines, keeping first occurrences |  |
| `text_diff` | local | `a`, `b` | Line-by-line difference between two texts |  |
| `text_find_replace` | local | `find`, `ignore_case`?, `replace`, `text` | Replace every occurrence of a phrase |  |
| `text_indent` | local | `spaces`?, `text` | Indent every line |  |
| `text_join_lines` | local | `separator`?, `text` | Join lines with a separator |  |
| `text_kebab_case` | local | `text` | Convert to kebab-case |  |
| `text_lorem_ipsum` | local | `words`? | Placeholder text of a given number of words |  |
| `text_lower` | local | `text` | Convert to lower case |  |
| `text_number_lines` | local | `text` | Prefix every line with its number |  |
| `text_pascal_case` | local | `text` | Convert to PascalCase |  |
| `text_readability` | local | `text` | Flesch reading ease and grade level |  |
| `text_remove_blank_lines` | local | `text` | Remove empty lines |  |
| `text_reverse` | local | `text` | Reverse the characters |  |
| `text_sentence_case` | local | `text` | Convert to Sentence case |  |
| `text_similarity` | local | `a`, `b` | Edit distance and similarity between two strings |  |
| `text_slugify` | local | `text` | Make a URL slug |  |
| `text_snake_case` | local | `text` | Convert to snake_case |  |
| `text_sort_lines` | local | `numeric`?, `reverse`?, `text` | Sort lines (alphabetically, numerically, or reversed) |  |
| `text_split` | local | `separator`?, `text` | Split text by a separator into numbered parts |  |
| `text_stats` | local | `text` | Count characters, words, lines, sentences and paragraphs |  |
| `text_title_case` | local | `text` | Convert To Title Case |  |
| `text_trim` | local | `text` | Trim whitespace at both ends of every line |  |
| `text_truncate` | local | `max`, `text` | Cut text to a maximum length, adding an ellipsis |  |
| `text_upper` | local | `text` | Convert to UPPER CASE |  |
| `text_wrap` | local | `text`, `width`? | Wrap text to a line width |  |
| `word_frequency` | local | `text`, `top`? | Most common words (ignoring common stop words) |  |

## Web & Network (8)

| Action | Kind | Parameters | What it does | Needs OK |
|---|---|---|---|---|
| `dns_lookup` | computer | `host` | Resolve a hostname to addresses |  |
| `download_url` | computer | `path`?, `url` | Download a URL to a file in /workspace |  |
| `git_clone` | computer | `url` | Clone a git repository into /workspace/repos |  |
| `http_get` | computer | `url` | Fetch a URL and show the response body (first 20 KB) |  |
| `http_headers` | computer | `url` | Show a URL's status and response headers |  |
| `http_status` | computer | `url` | Status code and timing for a URL |  |
| `robots_txt` | computer | `site` | Read a site's robots.txt (what automated visitors may fetch) |  |
| `tls_certificate` | computer | `host` | When a site's TLS certificate expires |  |
