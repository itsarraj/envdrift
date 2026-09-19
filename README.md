# envdrift

A `.env`/`.env.example` drift linter. Fills a real, unaddressed gap — not a
port of an existing tool from another language.

Every team that keeps a `.env.example` (documenting required variables) next
to a real `.env` (gitignored, actual values) eventually hits the same silent
failure: someone adds a new required variable to `.env.example` but forgets
to add it to their own `.env` (breaks locally, usually discovered the hard
way), or adds a variable straight to `.env` and forgets to document it in
`.env.example` (breaks onboarding for the next person who clones the repo),
or leaves a variable in `.env` empty after copying it from the example
without ever filling in a real value. Nothing greps for this automatically
today — `envdrift` does, in one pass, in three named categories.

## Usage

```bash
envdrift check                                   # .env.example vs .env in cwd
envdrift check --example .env.staging.example --env .env.staging
envdrift                                          # same as `envdrift check` with defaults

envdrift sync                                     # documents undocumented .env vars into .env.example
envdrift sync --dry-run                           # show what sync would add, without writing
```

`check` reports three categories:

- **missing** — documented in `.env.example`, absent from `.env` entirely.
  Will break at runtime; nothing provides this variable a value at all.
- **unfilled** — present in both files, but `.env`'s value is empty while
  `.env.example`'s value is non-empty (i.e. the example implies a real value
  belongs here, and none was ever filled in). A variable that's empty in
  *both* files is **not** flagged — an empty default in the example is how
  you document "this one's genuinely optional."
- **extra** — present in `.env`, not documented in `.env.example` at all.
  Doesn't break anything locally, but it's an onboarding gap: the next
  person to clone the repo has no idea this variable exists or what it
  should be.

Exit code is `1` if any of the three categories is non-empty, `0` otherwise
— no special `--ci` flag needed to make it CI-usable, matching this repo's
own `sqllint`/`commitguard` convention (both just exit `1` on any finding by
default; `envdrift` does the same).

`sync` only ever appends **key names** with empty values — it never copies a
real value out of `.env` into `.env.example`, on purpose (`.env.example`
usually ends up committed to git; a value copied out of `.env` might be a
real secret). It's the safe half of fixing drift: it can close the "extra"
category automatically, but "missing" and "unfilled" always need a human to
type in a real value.

If `.env` doesn't exist yet at all, `check` treats it as empty (a fresh
clone with only `.env.example` is a real, common state, not an error) — it
prints a one-line note and every documented variable shows up as missing.
`.env.example` not existing, on the other hand, is a hard error for `check`
(there's no source of truth to diff against); `sync` tolerates it missing
and creates it fresh.

## `.env` parsing

- comments (`#`-prefixed lines) and blank lines: skipped
- `export KEY=value` shell-style prefix: stripped
- `KEY="double quoted value"` and `KEY='single quoted value'`: quotes
  stripped; double-quoted values additionally support `\"`, `\\`, `\n`,
  `\t` escapes (matching real shell semantics: single-quoted strings never
  process escapes)
- unquoted values run to end-of-line verbatim (trailing whitespace
  trimmed) — a literal `#` inside an unquoted value is **not** treated as a
  mid-line comment marker, since real values (URLs with fragments,
  connection strings) legitimately contain one; only a line that *starts*
  with `#` is a comment
- **multi-line quoted values are supported**: if a quote opened on one line
  doesn't close on that same line, subsequent physical lines are folded in
  (joined with `\n`) until the matching closing quote is found
- duplicate keys: last one wins, matching both real shell `source`-ing and
  every mainstream dotenv loader

## Status: built, verified live through a full missing/unfilled/extra drift scenario end to end

- **27 unit tests** (`cargo test`, all passing): the parser (`src/dotenv.rs`,
  20 tests) — simple pairs, comments/blank lines, both quote styles,
  `export` with a space and with a tab, empty unquoted and empty quoted
  values, a literal `#` inside an unquoted value surviving intact, trailing
  content after a closing quote ignored, multi-line double- and
  single-quoted values, an escaped `\"` inside a double-quoted value, single
  quotes confirmed to *not* process escapes, duplicate keys (last wins),
  lines with no `=` skipped, CRLF line endings, whitespace around key/value
  trimmed, and an unterminated quote consuming the rest of the file instead
  of panicking or silently truncating — and the diff logic (`src/diff.rs`,
  7 tests) — clean match, a var missing entirely, a var present-but-empty
  against a non-empty example (unfilled), empty-in-both correctly *not*
  flagged, an undocumented extra var, a whitespace-only value counting as
  empty, and a combined scenario with all three categories plus one clean
  match in a single diff.
- **Live end-to-end run** against a real fixture built in a temp directory
  with a real `.env.example` and a real `.env`, deliberately constructed to
  hit all three categories plus a clean match, including `export` and mixed
  quote styles:

  `.env.example`:
  ```
  APP_NAME=myapp
  PORT=3000
  SECRET_TOKEN="replace with a real token"
  DATABASE_URL=postgres://user:password@localhost:5432/myapp_dev
  API_KEY=sk-replace-with-a-real-key
  FEATURE_BETA_UI=
  ```
  `.env`:
  ```
  export APP_NAME=myapp
  PORT=3000
  SECRET_TOKEN='a real token with spaces in it'
  API_KEY=
  FEATURE_BETA_UI=
  DEBUG_MODE=true
  ```
  Running the actual compiled release binary (`envdrift check`) against
  this produced:
  ```
  missing (1): in .env.example but not in .env — will break at runtime
    DATABASE_URL

  unfilled (1): present in .env but empty — .env.example suggests a real value is needed
    API_KEY

  extra (1): in .env but not documented in .env.example — onboarding gap
    DEBUG_MODE

  3 problem(s) found (4 matched cleanly)
  ```
  with exit code `1` — all three categories correct, and the 4 matched
  variables (`APP_NAME` despite the `export` prefix, `PORT`, `SECRET_TOKEN`
  despite being double-quoted in one file and single-quoted in the other,
  and `FEATURE_BETA_UI` empty in both and correctly *not* flagged as
  unfilled) confirm no false positives.
- **Live-verified the clean and missing-`.env` paths** separately: two
  identical files produced `no drift: 2 variable(s) match` and exit `0`;
  deleting `.env` entirely produced the "does not exist yet" note plus both
  variables correctly reported missing, exit `1`.
- **Live-verified `sync`**, against the same fixture: `--dry-run` printed
  `would add 1 key(s) ... DEBUG_MODE=` and left the file byte-for-byte
  unchanged (confirmed by re-`cat`-ing it before the real run); the real run
  then appended exactly `DEBUG_MODE=\n` to the end of `.env.example` —
  the *key* from `.env`, an *empty* value, never the real `true` that was
  actually in `.env` — and re-running `check` afterward showed the `extra`
  category gone (2 problems left: `missing` + `unfilled`, both of which
  genuinely need a human to type in a real value, 5 matched). Running
  `sync` again reported "already documents every variable ... nothing to
  sync" and left the file untouched. Also verified `sync` against a
  directory with a `.env.example` but no `.env` at all: fails loudly
  (`.env does not exist — nothing to sync from`) rather than silently
  writing nothing useful.
- **`--example`/`--env` path overrides** verified working against
  arbitrarily-named files (`prod.example`/`prod.env`), not just the
  `.env.example`/`.env` defaults.
- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` both
  clean.

**Not done / deliberately deferred**:

- **Multi-line *unquoted* values**: not a real `.env` construct (an
  unquoted value ends at the newline, full stop, in every dotenv dialect
  this was checked against) — only quoted values can legitimately span
  multiple lines, and that case *is* handled.
- **`${OTHER_VAR}` interpolation** some dotenv dialects (notably
  docker-compose's `env_file` handling) support inside values: treated as
  inert literal text here, not expanded. A linter that diffs two files
  shouldn't be executing/expanding what it finds in them — expansion would
  also make "is this value empty" ambiguous (empty until expanded, or
  really empty?).
- **`.env.local`/`.env.production`/etc. variant-file support** as a named
  concept: not built as a distinct feature, but not really a gap either —
  `--example`/`--env` take arbitrary paths, so `envdrift check --example
  .env.example --env .env.production` (or diffing any two dotenv-shaped
  files against each other) already works today, as verified above against
  `prod.example`/`prod.env`. What's *not* built is auto-discovering every
  `.env.*` variant in a directory and checking them all in one invocation —
  that would be a reasonable v2, not attempted here.
- **Leaked-real-secret detection** (an `.env.example` value that's
  non-empty *and* looks like a genuine credential rather than a
  placeholder): named in this tool's own motivating problem statement, but
  deliberately not implemented here — that's a distinct concern
  (content-based secret scanning, not structural drift) already owned by
  `leakscan` elsewhere in this collection, and conflating the two would
  make `envdrift` worse at its one job rather than better at two.
- **Comment/ordering preservation beyond appending**: `sync` appends new
  keys at the end of `.env.example` in the order they were found in `.env`;
  it does not try to insert a new key near a related section or preserve
  any particular grouping — the existing file's content above the
  appended lines is left completely untouched, byte-for-byte.
