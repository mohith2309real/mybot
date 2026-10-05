# Features (170)

What MyBot 2.0 does, one feature per line. On top of these are the catalogs: [249 actions](ACTIONS.md), [245 built-in skills](SKILLS.md) and [300 sites](SITES.md).


## The window (29)

1. Native desktop app (Rust, egui): no web view anywhere
2. Roster of teammates with colour avatars
3. Live state on each teammate: working, or needs you
4. A conversation per teammate, rebuilt from history on open
5. Replies stream in as they're written
6. Live preview of the model's thinking while it works
7. Finished reasoning folded into a collapsible “Thought”
8. Every step shown with its outcome (✔ / ✖), expandable to the full result
9. Chat Markdown: headings, bullets, bold, inline code, code blocks
10. Selectable, copyable text in the conversation
11. Starter ideas for an empty conversation
12. Composer: Enter sends, Shift+Enter adds a line
13. Stop a running task
14. Pick a skill to run from the composer, with search
15. “Needs you” banner: Take over, I'm done, Skip
16. Confirmation banner: Approve or Decline
17. Sign-in request cards above everything, top right
18. The window asks for attention when a sign-in request arrives
19. Short notices (toasts) for results and errors
20. Remembers the open teammate and the computer panel between launches
21. Window icon drawn in code (MyBot's own mark)
22. Adapts down to 900×560: compact header, wrapped banner, truncated steps
23. Lock and unlock from the sidebar
24. Unlock screen; first run creates the passphrase (typed twice, 8+ characters)
25. Optional: remember the passphrase in the system keychain
26. Continue without unlocking when keys come from the environment
27. New-teammate dialog: name, provider, live model list, effort, mode, persona
28. Edit a teammate; delete one (with confirmation)
29. Shows which providers have no key yet

## Each teammate's computer (17)

30. A Docker container per conversation
31. A separate desktop (X display) per teammate
32. A separate Chromium and browser profile per teammate
33. Starts by itself when a task needs it, or with Start computer
34. Live view drawn natively from VNC (RFB 3.3 / 3.7 / 3.8)
35. VNC password authentication
36. Raw, CopyRect and DesktopSize encodings
37. Take over the mouse
38. Take over the keyboard, including named keys and Ctrl shortcuts
39. Scroll wheel in takeover
40. Paste from your clipboard into the bot's desktop
41. Open the same desktop in a browser tab
42. Notices when the live view drops, and says why
43. Activity log of the computer
44. Docker and image status in Settings
45. Reset every teammate's browser profile (sign out everywhere)
46. Stop or remove the computer from the command line

## What a teammate can do (16)

47. Read the page as text plus numbered elements
48. Open a URL
49. Click an element by number
50. Type into a field by number, optionally submitting
51. Scroll
52. Sign in with a saved login (fill_login)
53. Typing into a password field is redirected to fill_login
54. Run shell commands in its own computer
55. Find one of the 249 actions, and run it
56. Hand over to you with a reason (request_human)
57. Finish with a summary (task_done)
58. The 249 actions: text, data, math, dates, hashing, files, archives, web, programs, browser, notes and memory
59. Every local action carries an example that the tests run
60. Actions with consequences are marked, and need your OK
61. Remembers the last 10 tasks with you as context
62. Up to 40 steps per task

## Where it stops for you (10)

63. CAPTCHA on the page
64. A page asking for a 2-step code
65. A one-time-code field
66. A payment card field
67. Checkout and billing pages
68. Buttons that place an order or pay
69. Steps a skill marks always-confirm
70. A page you already took over isn't flagged again for the same path
71. A pause waits 15 minutes for you, then the task stops cleanly
72. Cancelling works at any point, including while it waits for you

## Asking before acting (9)

73. Three modes: Ask first, Auto, Bypass
74. Ask first: checks in when unsure
75. Auto: routine work without checking in
76. Bypass: no stops for consequential steps
77. Your request counts as permission (“…and send it”)
78. …but not when you said not to (“draft it, don't send it”)
79. Covers sending, publishing, buying, moving money, deleting, changing access, production changes, accepting terms
80. Destructive shell commands refused in every mode
81. Never types passwords, codes or card numbers itself, in any mode

## Saved logins (14)

82. Logins encrypted with AES-256-GCM, key from scrypt
83. Ask every time: allow once, always for this site, or deny
84. No answer means no
85. Bound to the exact origin, https only (http only for localhost)
86. Sign-in pages for 300 known sites, suggested as you type
87. Sites that use 2-step sign-in are flagged
88. A 5-minute allowance for two-step sign-ins within one task
89. Re-checks the live page at fill time: origin, password field, username field
90. The password is typed by MyBot, never seen by the model
91. Page reads blank out password values
92. History of sign-in requests, with no passwords in it
93. “Don't ask for this site” switch per login
94. Same files as MyBot 1.x, byte for byte
95. Passwords wiped from memory after use, hidden from debug output

## Keys and models (15)

96. API keys encrypted with your passphrase
97. Keys from the environment take precedence
98. Unlock from the system keychain
99. Claude through the Messages API, streamed
100. Adaptive thinking with summaries
101. Five effort levels, low to max
102. Server-side model fallback where the model supports it
103. Tool input streamed and checked as strict JSON
104. Refusals handled and shown
105. A tool call cut off by the length limit is never run
106. OpenAI through the Responses API
107. Gemini, with thought signatures sent back unchanged
108. Live model lists
109. A custom endpoint per provider
110. A local OpenAI-compatible proxy works without a key

## Skills (21)

111. 245 built-in skills in 18 categories
112. Filter the library by category, and search it
113. Skills with inputs ask for them before running
114. Safety rules on skills, including always-confirm
115. Write your own skill
116. Edit or delete your skills
117. Skills taught by demonstration
118. Import Agent Skills (SKILL.md) from a folder
119. …from a .zip or .skill file
120. …from a GitHub folder link
121. …by dropping it on the window
122. Every imported file is scanned (high / medium / info findings)
123. Review screen: findings, files, license, tools, instructions
124. Imported skills start switched off
125. Refuses to switch on a skill whose files changed after review
126. Imported scripts run only inside the bot's computer
127. Skill files are copied into the computer when used
128. Switched-on imported skills are offered to every teammate
129. Export any skill as a SKILL.md folder
130. Run a skill from the Skills screen with any teammate
131. Schedule a skill straight from its card

## Teach a task (11)

132. Record yourself doing a task in the bot's browser
133. Passwords, codes and card numbers blanked inside the page
134. A second check of every recorded action in MyBot
135. Credentials stripped from recorded URLs
136. Keys recorded by name only, never keystrokes
137. Stops by itself after 10 minutes
138. Live log of what's being recorded
139. The teammate's model drafts a skill from the recording
140. Review and edit the draft before saving
141. Saved under a unique name
142. The recording is kept if drafting fails, to try again

## Routines (7)

143. Run a skill on a schedule (cron)
144. Presets like “weekdays at 9”
145. Plain-language schedule and next run shown as you type
146. Choose the teammate and inputs per routine
147. Pause, resume, run now, delete
148. Last run shown
149. Fire while the app is open, or headless with routines watch

## Command line (16)

150. keys: set, list, remove
151. bots: list, add, remove
152. run: a task streamed to the terminal
153. Sign-in requests answered at the terminal
154. Pauses answered at the terminal
155. Ctrl+C stops the task
156. tasks and log
157. resume and cancel from another terminal
158. logins: add, list, remove, always, pending, allow, deny, history
159. skills: list, show, import, review, enable, disable, remove, export
160. teach from the terminal
161. routines: list, add, remove, on, off, run, watch
162. actions and sites search, with --json
163. models: the live list
164. computer: status, reset profiles, down
165. Secrets read without echo, or from a pipe for scripts

## Data (5)

166. SQLite store of teammates, tasks, steps, skills, routines, requests
167. Every step of every task logged
168. Tasks left running by a crash are closed out at startup
169. MYBOT_HOME, MYBOT2_DB and MYBOT_PASSPHRASE
170. Shares ~/.mybot with MyBot 1.x
