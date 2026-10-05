# Built-in skills (245)

Step-by-step know-how a teammate follows. Run one from the app (✨ in the composer, or Skills → Run), or `mybot2 run <bot> --skill "<name>" --arg key=value`.
Every one exports as an Agent Skills folder (`SKILL.md`), and skills in that format import too.

*Asks first* lists the steps where the bot always stops for your OK.


## Calendar & Scheduling (10)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Week ahead brief | Summarize next week's calendar with prep notes. |  | accepting, declining, creating or moving events |
| Find a meeting time | Find free slots for a meeting. | duration, window |  |
| Create a calendar event | Create an event the user asked for. | title, when, attendees | saving the event or sending invitations |
| Meeting prep pack | Prepare background for an upcoming meeting. | meeting |  |
| Time audit | Report how the last month's time was spent in meetings. |  |  |
| Birthday and anniversary list | Collect upcoming birthdays and anniversaries. |  |  |
| Deadline tracker | Pull deadlines from documents and email into one list. |  |  |
| Timezone meeting planner | Find a meeting time that works across time zones. | cities, duration |  |
| Recurring event cleanup | Find stale recurring events. |  | deleting or changing events |
| Daily agenda | Write today's agenda with travel time and prep. |  |  |

## Data & Spreadsheets (18)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Clean a CSV | Fix common CSV problems. | file |  |
| Summarize a spreadsheet | Describe what a CSV contains. | file |  |
| Pivot table | Group and total a CSV by a column. | file, group_by, value |  |
| Join two CSVs | Combine two tables on a shared column. | left, right, key |  |
| Chart from CSV | Make a simple chart image from data. | file, x, y |  |
| Data from a web table | Copy a table from a web page into a CSV. | url |  |
| Scrape a listing page | Collect structured items from a list page. | url, fields |  |
| Deduplicate records | Find and merge near-duplicate rows. | file, columns |  |
| Validate data | Check a dataset against rules. | file, rules |  |
| JSON to spreadsheet | Flatten JSON records into a CSV. | file |  |
| Trend analysis | Describe how a measure changed over time. | file, date_column, value_column |  |
| Survey results summary | Summarize survey responses. | file |  |
| Data dictionary | Document every column of a dataset. | file |  |
| Compare two datasets | Find differences between two versions of a table. | old, new, key |  |
| Convert units in a table | Convert a column from one unit to another. | file, column, from, to |  |
| Sample a large file | Take a random sample of a large CSV. | file, rows |  |
| Spreadsheet formula help | Write a spreadsheet formula for a task. | goal, app |  |
| Leaderboard from results | Rank entries by a score. | file, score_column |  |

## Developer (20)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Clone and explore a repo | Clone a repository and explain its structure. | repo_url | running install or build scripts from an unfamiliar repository |
| Run a test suite | Run a project's tests and summarize failures. | path |  |
| Dependency audit | List outdated or vulnerable dependencies. | path |  |
| Code review checklist | Review a diff for common problems. | path |  |
| Find TODOs | List TODO and FIXME comments in a codebase. | path |  |
| Generate a changelog | Draft release notes from git history. | path, since | pushing, tagging or publishing a release |
| API endpoint check | Check that a list of URLs respond correctly. | urls_file |  |
| Lint and format report | Run the project's linters and summarize. | path |  |
| Explain an error | Explain an error message and suggest fixes. | error |  |
| Write unit tests | Add unit tests for a function. | file, function |  |
| Docker cleanup report | Report unused images and volumes (inside this computer). |  | removing containers, images or volumes |
| SQL query helper | Write a SQL query from a description. | goal, schema |  |
| Regex builder | Build and test a regular expression. | goal, examples |  |
| Open source license check | List the licenses of a project's dependencies. | path |  |
| GitHub issues digest | Summarize recent issues in a repository. | repo | commenting, labelling or closing issues |
| Pull request summary | Explain what a pull request changes. | pr_url | approving, commenting or merging |
| Benchmark a command | Time a command over several runs. | command, runs |  |
| Environment report | Report what tools and versions this computer has. |  |  |
| Static site preview | Serve a folder locally and screenshot pages. | folder |  |
| Broken link checker | Find broken links on a website. | url |  |

## Email & Messages (15)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Inbox triage | Sort unread email into act, reply, read later and ignore. |  | sending any email |
| Draft email replies | Draft replies to emails that need one, without sending. |  | sending an email |
| Unsubscribe list | Find newsletters you never read and list how to unsubscribe. |  | clicking an unsubscribe link |
| Find an email | Find a specific email and summarize it. | description |  |
| Receipts collector | Collect receipts from email into a spreadsheet. | month |  |
| Follow-up reminder list | Find sent emails that never got a reply. |  |  |
| Send a prepared email | Send an email the user has explicitly written or approved. | to, subject, body_file | sending the email |
| Out-of-office setup | Prepare an auto-reply for a date range. | start, end, contact | saving changes to account settings |
| Thread summary | Summarize a long email thread. | subject |  |
| Slack catch-up | Summarize what happened in Slack channels while you were away. | channels | posting or reacting in Slack |
| Contact list cleanup | Find duplicate or incomplete contacts. | contacts_file |  |
| Discord catch-up | Summarize recent messages in Discord channels. | server, channels | posting a message |
| Email to task list | Turn action items buried in email into a task list. |  |  |
| Polite decline | Draft a polite way to say no. | request |  |
| Inbox search report | Report everything in email about a topic. | topic |  |

## Files & Documents (18)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Organize downloads | Sort a messy folder into subfolders by type. | folder | deleting files |
| Find duplicate files | Find duplicate files by content. | folder | deleting files |
| Rename files consistently | Rename a set of files to a pattern. | folder, pattern |  |
| Merge text files | Combine several text or Markdown files into one. | files, output |  |
| Document to outline | Extract the heading structure of a document. | file |  |
| Compare two documents | Show what changed between two versions. | old, new |  |
| Archive old files | Zip files older than a date. | folder, before |  |
| Extract text from PDFs | Pull the text out of PDF files. | folder |  |
| Folder size report | Show what is taking up space. | folder |  |
| Create a README | Write a README for a folder of files. | folder |  |
| Convert Markdown to HTML | Turn Markdown files into simple HTML pages. | folder |  |
| Template fill | Fill a template with values for many recipients. | template, data_csv |  |
| Notes to knowledge base | Turn scattered notes into linked topic pages. | folder |  |
| Image inventory | List images with dimensions and sizes. | folder |  |
| Backup a folder | Make a dated backup archive of a folder. | folder |  |
| Word count report | Count words across a set of documents. | folder |  |
| Redact personal data | Mask emails, phone numbers and IDs in a document. | file |  |
| Checklist from instructions | Turn a set of instructions into a checklist. | file |  |

## Finance (12)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Monthly spending report | Categorize a bank export and summarize spending. | statement_file | any transfer or payment |
| Budget plan | Draft a monthly budget from income and spending. | income, statement_file |  |
| Bill due dates | List upcoming bills and due dates. |  | paying a bill |
| Account balances check | Sign in and record balances across accounts. | accounts | any transfer, payment or change to account settings |
| Invoice from template | Generate an invoice document. | client, items | sending an invoice |
| Expense report | Build an expense report from receipts. | receipts_folder |  |
| Currency conversion sheet | Convert amounts using today's published rates. | amounts_file, to_currency |  |
| Loan comparison | Compare loan offers by total cost. | offers_file |  |
| Tax document checklist | Build a checklist of tax documents to gather. | country, situation |  |
| Investment portfolio snapshot | Summarize holdings from a brokerage export. | holdings_file | any trade or transfer |
| Savings goal plan | Plan how to reach a savings goal. | goal, deadline, current |  |
| Split a shared bill | Work out who owes whom after shared expenses. | expenses_file | sending money or payment requests |

## Health & Fitness (6)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Workout plan | A weekly workout plan for a goal. | goal, days, equipment |  |
| Nutrition label comparison | Compare foods by their nutrition facts. | foods |  |
| Doctor appointment prep | Prepare questions and notes for an appointment. | reason |  |
| Medication information | Look up official information about a medicine. | medicine |  |
| Sleep habit tracker | Summarize sleep data from an export. | file |  |
| Running route finder | Find running routes of a given distance. | location, distance |  |

## Home & Life (12)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Meal plan | Plan a week of meals with a shopping list. | diet, people |  |
| Recipe scaler | Scale a recipe and convert units. | recipe_url, servings, units |  |
| Moving checklist | A dated checklist for moving home. | move_date |  |
| Utility provider comparison | Compare electricity, internet or similar providers. | service, location | switching provider or signing a contract |
| Home maintenance schedule | A yearly maintenance calendar. | home_type, climate |  |
| Appliance manual finder | Find the manual for an appliance. | model |  |
| Event planner | Plan a party or gathering. | event, guests, budget | booking or buying |
| Pet care research | Research care needs for a pet. | animal |  |
| Plant care guide | Care instructions for houseplants. | plants |  |
| Renter's rights lookup | Find tenant rules where you live. | location, question |  |
| Car maintenance lookup | Find the service schedule for a car. | make_model_year |  |
| Charity research | Check a charity before donating. | charity | donating |

## Jobs & Career (10)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Job search | Find job postings that match a profile. | role, location, must_haves | submitting an application |
| Company research for interview | Prepare for an interview with a company. | company, role |  |
| Salary benchmark | Find salary ranges for a role. | role, location |  |
| Application tracker | Keep a table of applications and their status. |  |  |
| Fill a job application | Fill an application form and stop before submitting. | job_url, resume_file | submitting the application |
| Skills gap analysis | Compare current skills with a target role. | resume_file, target_role |  |
| Networking list | List relevant communities and events for a career goal. | field, location |  |
| Freelance gig scan | Find freelance projects that fit a skill set. | skills | sending a proposal or message |
| LinkedIn profile review | Suggest improvements to a LinkedIn profile. | profile_url | editing the profile |
| Interview practice questions | Generate practice questions with model answers. | role, level |  |

## Learning (10)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Study plan | Make a week-by-week plan to learn a subject. | subject, weeks, hours_per_week |  |
| Flashcards from notes | Turn notes into question-and-answer flashcards. | file |  |
| Quiz me | Write a quiz on a topic with an answer key. | topic, level |  |
| Course finder | Find good courses on a subject. | subject | enrolling or paying |
| Reading list | Build a reading list on a subject. | subject |  |
| Vocabulary list | Collect vocabulary for a language and topic. | language, topic |  |
| Explain like a tutor | Teach a concept step by step with checks. | concept |  |
| Lecture notes from video | Make notes from a lecture video. | video_url |  |
| Practice problems | Generate practice problems with solutions. | topic, count |  |
| Citation formatter | Format references in a citation style. | file, style |  |

## News & Monitoring (10)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Morning news brief | A short brief of today's top news. | topics |  |
| Keyword alert | Check news for a keyword and log new results. | keyword |  |
| Weather brief | Today's and tomorrow's weather for a place. | location |  |
| Stock watch | Prices and news for a list of tickers. | tickers | any trade |
| Release watcher | Check for new releases of software. | projects |  |
| Website uptime check | Check that sites are up and fast. | urls |  |
| Job posting watcher | Report new postings on a careers page. | careers_url |  |
| Government notice watcher | Check an official page for new notices. | url |  |
| Podcast episode tracker | List new episodes of chosen podcasts. | podcasts |  |
| Sports results | Latest results and next fixtures for teams. | teams |  |

## Productivity (10)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Weekly review | Review last week and plan next week. |  |  |
| Task breakdown | Break a project into small tasks. | project |  |
| Prioritize a task list | Sort tasks by urgency and importance. | file |  |
| Daily journal prompt | Start today's journal entry with prompts. |  |  |
| Decision log | Record a decision with context and options. | decision |  |
| Focus session plan | Plan a day in focus blocks. | tasks |  |
| Project status report | Write a status update from project files. | folder |  |
| Reading queue triage | Sort saved articles into read, skim and skip. | links_file |  |
| Habit tracker setup | Create a simple habit tracker file. | habits |  |
| Meeting agenda | Draft an agenda for a meeting. | purpose, duration |  |

## Research (25)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Summarize a web page | Read one page and write a short summary with the key facts. | url |  |
| Compare products | Compare several products side by side on price and features. | products, criteria |  |
| Find competitors | List a company's main competitors with one line on each. | company |  |
| Literature scan | Find recent papers or articles on a topic and summarize each. | topic |  |
| Fact check a claim | Check a claim against primary sources and give a verdict. | claim |  |
| Company profile | Build a one-page profile of a company. | company |  |
| Person background (public) | Summarize a public figure's professional background from public sources. | person |  |
| Market size estimate | Estimate a market's size from published figures, with the method shown. | market |  |
| Collect reviews | Gather and summarize customer reviews for a product or service. | product |  |
| Explain a topic simply | Research a topic and explain it for a beginner. | topic |  |
| Pros and cons | Research a decision and lay out the pros and cons. | decision |  |
| Find statistics | Find reliable statistics on a subject, with sources. | subject |  |
| Monitor a page for changes | Snapshot a page and report what changed since last time. | url |  |
| Patent search | Search patents related to an idea and summarize the closest ones. | idea |  |
| Grant and funding finder | Find open grants or funding programs that fit a project. | project, location |  |
| Event finder | Find upcoming events on a topic in a place. | topic, location |  |
| Wikipedia digest | Turn a Wikipedia article into a structured digest. | article |  |
| Source list | Build an annotated list of the best sources on a subject. | subject |  |
| Price history check | Check whether a product's current price is a good deal. | product |  |
| Regulation lookup | Find what rules apply to an activity in a jurisdiction. | activity, jurisdiction |  |
| News timeline | Build a dated timeline of a news story. | story |  |
| Tool or app recommendation | Find the best tools for a job and compare them. | job |  |
| Domain name ideas | Brainstorm domain names and check which are available. | business | buying or registering a domain |
| Academic paper explainer | Explain a research paper in plain language. | paper_url |  |
| Local business finder | Find well-reviewed local businesses of a type. | type, location |  |

## Shopping (15)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Find the best price | Find where a product is cheapest right now. | product | making a purchase |
| Add to cart | Put a specific item in the cart and stop before checkout. | item_url, options | checkout, payment or placing an order |
| Gift ideas | Suggest gifts for a person within a budget. | person, budget |  |
| Order tracker | Check the status of recent orders. |  |  |
| Return an item | Start a return and stop before submitting. | order | submitting a return |
| Grocery list to cart | Turn a grocery list into a store cart, stopping before checkout. | list_file, store | checkout or payment |
| Coupon finder | Find working coupon codes for a store. | store |  |
| Subscription audit | List active subscriptions found in email and bank exports. | statement_file | cancelling or changing a subscription |
| Product specs sheet | Collect the full specs for a product from the manufacturer. | product |  |
| Second-hand search | Search used marketplaces for an item. | item, location | messaging a seller or making an offer |
| Warranty lookup | Find warranty terms and how to claim. | product |  |
| Restock watcher | Check whether an out-of-stock item is back. | item_url |  |
| Size and fit research | Find sizing guidance for clothing or shoes. | item |  |
| Compare phone plans | Compare mobile plans for a usage profile. | usage, country |  |
| Wishlist price check | Check prices on a list of saved items. | list_file |  |

## Social Media (12)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Draft social posts | Draft posts for several networks from one announcement. | announcement | publishing or scheduling a post |
| Post a prepared update | Post text the user wrote, stopping for confirmation. | network, text_file | publishing the post |
| Mentions report | Collect recent public mentions of a name or brand. | name |  |
| Content calendar | Plan a month of posts. | theme, frequency |  |
| Profile audit | Review a social profile and suggest improvements. | profile_url | changing profile settings |
| Hashtag research | Find relevant hashtags with activity levels. | topic, network |  |
| Reply suggestions | Draft replies to comments on your posts. | post_url | posting replies |
| Competitor social snapshot | Compare competitors' social presence. | accounts |  |
| Thread from article | Turn an article into a short thread. | url | publishing |
| YouTube video digest | Summarize a YouTube video from its description and transcript. | video_url |  |
| LinkedIn post draft | Draft a LinkedIn post about an achievement or lesson. | topic | publishing |
| Reddit research | See what Reddit communities say about something. | topic |  |

## Travel (12)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Flight search | Find flight options without booking. | from, to, dates | booking or paying |
| Hotel shortlist | Shortlist hotels for a stay. | city, dates, budget | reserving or paying |
| Trip itinerary | Plan a day-by-day itinerary. | destination, days, interests |  |
| Visa and entry requirements | Check entry requirements for a trip. | nationality, destination |  |
| Packing list | Make a packing list for a trip. | destination, dates, activities |  |
| Check in for a flight | Open online check-in and stop before confirming. | airline, booking_ref, last_name | confirming check-in, choosing paid seats or paying |
| Local transport guide | Explain how to get around a city. | city |  |
| Restaurant reservations scout | Find restaurants with availability, without booking. | city, date, party_size | making a reservation |
| Travel budget | Estimate the total cost of a trip. | destination, days, style |  |
| Car rental comparison | Compare car rental offers. | location, dates | booking |
| Travel advisory check | Check current travel advisories for a destination. | destination |  |
| Loyalty points summary | Summarize balances across travel loyalty programs. | programs | redeeming or transferring points |

## Website Checks (10)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| SEO check | Check a page's basic SEO. | url |  |
| Accessibility quick scan | Spot common accessibility problems. | url |  |
| Page performance snapshot | Measure load timing of a page. | url |  |
| Form test | Test a form's validation without submitting real data. | url | submitting a form with real data |
| Screenshot a set of pages | Take screenshots of several pages. | urls |  |
| Mobile view check | Check how a page looks at phone width. | url |  |
| Cookie banner check | Review a site's cookie consent banner. | url |  |
| Content inventory | List the pages of a small site. | url |  |
| Compare two web pages | Report text differences between two pages. | url_a, url_b |  |
| Structured data check | Inspect a page's schema.org data. | url |  |

## Writing (20)

| Skill | What it does | Inputs | Asks first |
|---|---|---|---|
| Proofread a document | Fix spelling, grammar and punctuation without changing the voice. | file |  |
| Shorten text | Cut a piece of writing to a target length, keeping what matters. | file, target_words |  |
| Change the tone | Rewrite text in a different tone. | file, tone |  |
| Blog post draft | Research and draft a blog post. | topic, audience | publishing or posting anything |
| Press release draft | Draft a press release from a few facts. | news |  |
| Meeting notes cleanup | Turn rough meeting notes into clean minutes with action items. | file |  |
| Executive summary | Write a one-paragraph executive summary of a long document. | file |  |
| Product description | Write a product description from specs. | product, specs |  |
| Cover letter draft | Draft a cover letter matched to a job posting. | job_url, resume_file | submitting an application |
| Resume tailoring | Suggest resume edits to fit a specific job. | job_url, resume_file |  |
| Translate a document | Translate a text file and keep its formatting. | file, language |  |
| FAQ from documentation | Write an FAQ from a product's documentation. | docs_url |  |
| Speech or toast draft | Draft a short speech for an occasion. | occasion, details |  |
| Headline options | Write headline options for a piece. | file |  |
| Outline a long document | Produce a detailed outline before writing. | topic, length |  |
| Plain-language rewrite | Rewrite jargon-heavy text so anyone can follow it. | file |  |
| Newsletter draft | Draft a newsletter issue from links and notes. | notes_file | sending a newsletter or email |
| Bio writer | Write short, medium and long professional bios. | resume_file |  |
| Style guide check | Check a document against a style guide. | file, guide_file |  |
| Thank-you note | Draft a thank-you note. | recipient, reason | sending a message |
