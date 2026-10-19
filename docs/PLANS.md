# Hi-Fi plans and growth

## Principles

- The browser, Pages, documents and every local agent harness stay free. People bring their own models, so Hi-Fi never marks up tokens.
- Paid plans sell things that cost Hi-Fi money to run (sync, hosted sandboxes, background agents) or that teams need (shared spaces, policy, audit).
- No account is needed until someone wants a paid feature.

## Plans

| | Free | Pro | Team | Enterprise |
|---|---|---|---|---|
| Price | $0 | $12/month or $120/year | $24 per seat/month, 3 seats minimum | Custom, from 50 seats |
| Browser, Pages, terminals, worktrees | Yes | Yes | Yes | Yes |
| Local harnesses (all 11) | Yes | Yes | Yes | Yes |
| Documents (PDF, DOCX, PPTX, XLSX) | Yes | Yes | Yes | Yes |
| Parallel agents per window | 3 | 8 | 8 | Unlimited |
| Encrypted sync of spaces, Pages, chats | No | Yes | Yes | Self-hosted option |
| Scheduled and background agents | No | Yes | Yes | Yes |
| Hosted computer-use sandboxes | No | 20 hours/month | 40 hours per seat/month | Custom |
| Composio app connections | Your own key | Included workspace | Shared org workspace | Private |
| Shared spaces, Pages and skills | No | No | Yes | Yes |
| Org guard policies, audit log | No | No | Yes | Yes, with DLP rules |
| SSO, SCIM, invoicing | No | No | No | Yes |
| Support | Community | Email, 2 business days | Email, 1 business day | Dedicated channel |

Students and open-source maintainers get Pro free for a year.

### Unit economics

- Pro costs roughly $2–3 per user per month to serve: sync storage, relay bandwidth and amortised sandbox hours. That leaves a gross margin of about 75–80%.
- Reaching $1M ARR at 3% conversion needs about 230,000 monthly active users, which makes 100,000 users in three months the first milestone.

## Path to 100,000 users in 90 days

The wedge is developers who already pay for Claude Code, Codex or Cursor and juggle those CLIs with a browser and a notes app. Hi-Fi gives them one window with no new model bill.

### Days 0–14: ship and seed

- Ship signed builds for macOS, Windows and Linux, plus Homebrew, winget and Flathub listings.
- Launch the site and a 60-second video of an agent swarm researching a topic, filling a spreadsheet and drafting a slide deck while the user keeps browsing.
- Seed with 300 hand-picked users from agent-CLI communities and collect feedback in a public Discord.
- Target: 5,000 users.

### Days 15–45: launch week and content

- Launch on Hacker News ("Show HN: a 10 MB browser where your coding agents get their own tabs"), Product Hunt, r/rust, r/LocalLLaMA and r/ClaudeAI on separate days.
- Publish technical posts: compositing native web views under GPUI, keeping an agent browser under 50 MB of RAM, running 11 harnesses through one MCP server.
- Ship a public skills gallery: shareable SKILL.md recipes that any harness in Hi-Fi can use. Each shared skill links back to Hi-Fi.
- Target: 40,000 users.

### Days 46–90: loops

- **Share a run.** One click turns an agent run (its timeline, sources and output files) into a public read-only page with an "Open in Hi-Fi" button.
- **Documents as distribution.** Files that agents make carry an optional, removable "Made with Hi-Fi" footer.
- **Team invites.** Shared spaces on Team invite teammates by link. Each paid seat brings in 2–3 more free users.
- **Integrations.** Listings in the Claude Code, OpenCode and Composio directories, plus import from Arc, Chrome and Safari bookmarks.
- Target: 100,000 users, with 2,500 paying.

### Metrics

| Metric | Target by day 90 |
|---|---|
| Installs | 160,000 |
| Weekly active users | 100,000 |
| Users who run an agent in their first session | 60% |
| Week-4 retention | 35% |
| Free → paid conversion | 2.5% |
